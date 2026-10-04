use super::*;
use std::io::Write;

fn parsed(raw: &str) -> ParsedOAuthRequest {
    parse_oauth_http_request(raw.as_bytes()).expect("request should parse")
}

#[test]
fn accepts_percent_encoded_code_when_state_matches() {
    let req = parsed("GET /?code=abc%2F123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    match decide_oauth_callback(&req, "xyz") {
        CallbackDecision::Code(code) => assert_eq!(code, "abc/123"),
        other => panic!("unexpected decision: {other:?}"),
    }
}

#[test]
fn ignores_probe_and_mismatched_state() {
    let favicon = parsed("GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    assert!(matches!(
        decide_oauth_callback(&favicon, "xyz"),
        CallbackDecision::Ignore
    ));
    let wrong = parsed("GET /?code=abc&state=other HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    assert!(matches!(
        decide_oauth_callback(&wrong, "xyz"),
        CallbackDecision::Ignore
    ));
    let preflight = parsed(
        "OPTIONS / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://accounts.google.com\r\n\r\n",
    );
    assert!(matches!(
        decide_oauth_callback(&preflight, "xyz"),
        CallbackDecision::Preflight
    ));
    assert_eq!(
        preflight.origin.as_deref(),
        Some("https://accounts.google.com")
    );
}

#[test]
fn does_not_parse_a_partial_request_line_as_missing_code() {
    assert!(parse_oauth_http_request(b"GET /?code=abc").is_err());
}

#[test]
fn reads_callback_split_across_packets() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_oauth_connection(stream, "state-1")
    });
    let mut client = connect_with_retry(port);
    client.write_all(b"GET /?code=split-code").unwrap();
    client.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(80));
    client
        .write_all(b"&state=state-1 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    match server.join().unwrap() {
        ConnResult::Code { code, .. } => assert_eq!(code, "split-code"),
        ConnResult::Ignore => panic!("split callback was ignored"),
        ConnResult::OAuthError { error, .. } => panic!("oauth error: {error}"),
    }
}

#[test]
fn probe_reset_does_not_drop_the_real_callback() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        wait_for_oauth_callback(
            listener,
            "good",
            std::time::Duration::from_secs(5),
            &std::sync::atomic::AtomicBool::new(false),
        )
    });
    let probe = connect_with_retry(port);
    drop(probe);
    let mut favicon = connect_with_retry(port);
    favicon
        .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut mismatched = connect_with_retry(port);
    mismatched
        .write_all(
            b"GET /?code=stale&state=old HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut real = connect_with_retry(port);
    real.write_all(
        b"GET /?code=real&state=good HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    let (code, _) = server.join().unwrap().expect("callback should be accepted");
    assert_eq!(code, "real");
}

fn connect_with_retry(port: u16) -> std::net::TcpStream {
    let addr = format!("127.0.0.1:{port}");
    for _ in 0..50 {
        if let Ok(stream) = std::net::TcpStream::connect(&addr) {
            return stream;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::net::TcpStream::connect(addr).unwrap()
}
