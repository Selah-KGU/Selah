#[cfg(test)]
use super::oauth_http::ParsedOAuthRequest;
use super::oauth_http::{
    decide_oauth_callback, is_probe_path, is_retryable_io, looks_like_http,
    parse_oauth_http_request, read_http_headers, CallbackDecision,
};

/// Read a loopback OAuth redirect without treating browser probes as failure.
///
/// Chrome and Safari often open an extra connection (favicon, preflight, or a
/// speculative socket) before the real callback. A single blocking read used to
/// either consume a partial callback or abort the whole login when that probe
/// reset. Connections are handled independently, and a request is only final
/// once its headers are complete.
pub(in crate::google_commands) fn wait_for_oauth_callback(
    listener: std::net::TcpListener,
    expected_state: &str,
    timeout: std::time::Duration,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(String, std::net::TcpStream), (String, Option<std::net::TcpStream>)> {
    let _ = listener.set_nonblocking(true);
    let deadline = std::time::Instant::now() + timeout;
    let (tx, rx) = std::sync::mpsc::channel::<ConnResult>();
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(("ログインを中断しました".into(), None));
        }
        if std::time::Instant::now() >= deadline {
            return Err((
                "Google Calendar の認証がタイムアウトしました。ブラウザで認証を完了してから、もう一度お試しください。".into(),
                None,
            ));
        }
        match listener.accept() {
            Ok((stream, addr)) => {
                log::info!("OAuth callback connection from {}", addr);
                let tx = tx.clone();
                let expected_state = expected_state.to_string();
                std::thread::spawn(move || {
                    let result = handle_oauth_connection(stream, &expected_state);
                    let _ = tx.send(result);
                });
            }
            Err(e) if is_retryable_io(&e) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err((format!("接続受信失敗: {}", e), None)),
        }
        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(ConnResult::Code { code, stream }) => return Ok((code, stream)),
            Ok(ConnResult::OAuthError { error, stream }) => return Err((error, Some(stream))),
            Ok(ConnResult::Ignore) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(("認証コールバックの受信が中断されました".into(), None));
            }
        }
    }
}

enum ConnResult {
    Code {
        code: String,
        stream: std::net::TcpStream,
    },
    OAuthError {
        error: String,
        stream: std::net::TcpStream,
    },
    Ignore,
}

fn handle_oauth_connection(mut stream: std::net::TcpStream, expected_state: &str) -> ConnResult {
    let _ = stream.set_nodelay(true);
    // Accepted sockets inherit the listener's nonblocking flag. Force blocking
    // reads so a callback that arrives in a later packet is not dropped.
    if let Err(e) = stream.set_nonblocking(false) {
        log::debug!("Ignoring OAuth connection that could not block: {}", e);
        return ConnResult::Ignore;
    }
    let buf = match read_http_headers(&mut stream) {
        Ok(buf) => buf,
        Err(e) => {
            log::debug!("Ignoring OAuth probe connection: {}", e);
            return ConnResult::Ignore;
        }
    };
    if buf.is_empty() {
        log::debug!("Ignoring empty OAuth callback connection");
        return ConnResult::Ignore;
    }
    if !looks_like_http(&buf) {
        log::warn!(
            "OAuth callback was not HTTP (first bytes {:02x} {:02x}); the browser may have upgraded the redirect",
            buf.first().copied().unwrap_or(0),
            buf.get(1).copied().unwrap_or(0)
        );
        return ConnResult::Ignore;
    }
    let req = match parse_oauth_http_request(&buf) {
        Ok(req) => req,
        Err(reason) => {
            log::debug!("Ignoring incomplete OAuth callback request: {}", reason);
            return ConnResult::Ignore;
        }
    };
    match decide_oauth_callback(&req, expected_state) {
        CallbackDecision::Code(code) => {
            log::info!(
                "OAuth callback accepted (code_len={}, path={})",
                code.len(),
                req.path
            );
            ConnResult::Code { code, stream }
        }
        CallbackDecision::OAuthError(error) => {
            log::warn!("OAuth callback returned error: {}", error);
            ConnResult::OAuthError { error, stream }
        }
        CallbackDecision::Preflight => {
            send_oauth_empty(&stream, "204 No Content", req.origin.as_deref());
            ConnResult::Ignore
        }
        CallbackDecision::Ignore => {
            if req.query.contains_key("code") {
                log::warn!("Ignoring OAuth callback whose state did not match this login");
            } else {
                log::info!("Ignoring non-callback request {} {}", req.method, req.path);
            }
            if req.headers_complete {
                if is_probe_path(&req.path) {
                    send_oauth_empty(&stream, "204 No Content", req.origin.as_deref());
                } else {
                    send_oauth_waiting_response(&stream, req.origin.as_deref());
                }
            }
            ConnResult::Ignore
        }
    }
}

fn safe_origin(origin: Option<&str>) -> &str {
    match origin {
        Some(origin)
            if !origin.is_empty()
                && !origin
                    .bytes()
                    .any(|b| b == b'\r' || b == b'\n' || b == b' ') =>
        {
            origin
        }
        _ => "*",
    }
}

fn cors_headers(origin: Option<&str>) -> String {
    format!(
        "Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nVary: Origin\r\n",
        safe_origin(origin)
    )
}

fn write_http_response(
    stream: &std::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    origin: Option<&str>,
) {
    use std::io::Write;
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
    let mut stream = stream;
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{}\r\n{body}",
        body.len(),
        cors_headers(origin),
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn send_oauth_empty(stream: &std::net::TcpStream, status: &str, origin: Option<&str>) {
    write_http_response(stream, status, "text/plain; charset=utf-8", "", origin);
}

fn send_oauth_waiting_response(stream: &std::net::TcpStream, origin: Option<&str>) {
    let body = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>body{font-family:-apple-system,system-ui,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f5f5f7;color:#1d1d1f}.card{text-align:center;padding:30px;border-radius:14px;background:#fff;box-shadow:0 2px 12px rgba(0,0,0,.08)}h1{font-size:18px;margin:0 0 8px}p{font-size:13px;color:#86868b;margin:0}</style></head><body><div class=\"card\"><h1>認証を待機中...</h1><p>このタブはそのままにしてください。</p></div></body></html>";
    write_http_response(stream, "200 OK", "text/html; charset=utf-8", body, origin);
}

/// Send the final HTML response to the browser after token exchange.
pub(in crate::google_commands) fn send_oauth_response(
    stream: &std::net::TcpStream,
    success: bool,
    error: Option<&str>,
    origin: Option<&str>,
) {
    let (status, body) = if success {
        (
            "200 OK",
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>body{font-family:-apple-system,system-ui,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f5f5f7;color:#1d1d1f}.card{text-align:center;padding:40px;border-radius:16px;background:#fff;box-shadow:0 2px 12px rgba(0,0,0,.08)}h1{font-size:20px;margin:0 0 8px}p{font-size:14px;color:#86868b;margin:0}</style></head><body><div class=\"card\"><h1>Google Calendar 認証完了</h1><p>このタブを閉じてください。</p></div></body></html>".to_string(),
        )
    } else {
        let escaped_error = error
            .unwrap_or("不明なエラー")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        (
            "400 Bad Request",
            format!(
                "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body><h1>認証エラー</h1><p>{}</p></body></html>",
                escaped_error
            ),
        )
    };
    write_http_response(stream, status, "text/html; charset=utf-8", &body, origin);
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
