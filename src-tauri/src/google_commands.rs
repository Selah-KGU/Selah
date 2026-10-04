use tauri::{Emitter, Manager, State};

use crate::google_calendar::{CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus};
use crate::GCalState;

#[tauri::command]
pub async fn gcal_check_session(state: State<'_, GCalState>) -> Result<GoogleCalStatus, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.status())
}

#[tauri::command]
pub async fn gcal_get_config(state: State<'_, GCalState>) -> Result<GoogleCalConfig, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.config.clone())
}

#[tauri::command]
pub async fn gcal_save_config(
    state: State<'_, GCalState>,
    config: GoogleCalConfig,
) -> Result<(), String> {
    let mut gcal = state.client.lock().await;
    // Empty fields mean "use built-in default" — persist the user's choice
    // (empty on disk) but keep the resolved defaults in memory so OAuth works
    // immediately without a restart.
    crate::google_calendar::save_config(&config)?;
    gcal.config = crate::google_calendar::resolve_with_defaults(config);
    Ok(())
}

#[tauri::command]
pub async fn gcal_open_login(
    app: tauri::AppHandle,
    state: State<'_, GCalState>,
) -> Result<(), String> {
    log::info!("Opening Google Calendar login via system browser");

    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("ローカルサーバー起動失敗: {}", e))?;
    let port = std_listener
        .local_addr()
        .map_err(|e| format!("ポート取得失敗: {}", e))?
        .port();

    let attempt = {
        let gcal = state.client.lock().await;
        gcal.begin_login(port)?
    };
    let auth_url = attempt.url.clone();
    let verifier = attempt.verifier;
    let redirect_uri = attempt.redirect_uri;
    let expected_state = attempt.state;
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_for_wait = cancel.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

    let waiter = tokio::task::spawn_blocking(move || {
        if let Err(e) = std_listener.set_nonblocking(true) {
            let msg = format!("ローカルサーバー初期化失敗: {}", e);
            let _ = ready_tx.send(Err(msg.clone()));
            return Err((msg, None));
        }
        let _ = ready_tx.send(Ok(()));
        wait_for_oauth_callback(
            std_listener,
            &expected_state,
            std::time::Duration::from_secs(300),
            &cancel_for_wait,
        )
    });

    match tokio::time::timeout(std::time::Duration::from_secs(3), ready_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err(e);
        }
        Ok(Err(_)) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err("ローカルサーバー初期化失敗: ready channel closed".into());
        }
        Err(_) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err("ローカルサーバー起動タイムアウト".into());
        }
    }

    log::info!(
        "OAuth listener ready on port {}, opening browser to: {}",
        port,
        crate::client::safe_truncate(&auth_url, 200)
    );

    // Open the system browser only after the listener is accepting. This command
    // stays pending until the code is exchanged, so the UI does not depend on an
    // event that can be missed.
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(&auth_url, None::<&str>) {
        log::warn!(
            "opener plugin failed to open URL: {} — trying OS fallback",
            e
        );
        if let Err(fallback_err) = open_url_os_fallback(&auth_url) {
            log::error!("OS fallback also failed: {}", fallback_err);
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err(format!(
                "ブラウザを開けませんでした: {} (fallback: {})",
                e, fallback_err
            ));
        }
    }
    log::info!("Browser launch requested for Google Calendar OAuth");

    let callback = waiter
        .await
        .map_err(|e| format!("認証コールバック待ちに失敗: {}", e))?;
    match callback {
        Ok((auth_code, stream)) => {
            let app_state = app.state::<GCalState>();
            let mut gcal = app_state.client.lock().await;
            match gcal
                .exchange_code(&auth_code, &verifier, &redirect_uri)
                .await
            {
                Ok(()) => {
                    log::info!("Google Calendar login successful");
                    send_oauth_response(&stream, true, None, None);
                    let _ = app.emit("gcal-login-success", ());
                    Ok(())
                }
                Err(e) => {
                    log::error!("Google Calendar login failed: {}", e);
                    send_oauth_response(&stream, false, Some(&e), None);
                    let _ = app.emit("gcal-login-error", &e);
                    Err(e)
                }
            }
        }
        Err((e, stream)) => {
            log::error!("Google OAuth callback error: {}", e);
            if let Some(stream) = stream.as_ref() {
                send_oauth_response(stream, false, Some(&e), None);
            }
            let _ = app.emit("gcal-login-error", &e);
            Err(e)
        }
    }
}

/// OS-level fallback for opening a URL when the Tauri opener plugin fails.
fn open_url_os_fallback(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("`open` spawn failed: {}", e))?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let operation: Vec<u16> = std::ffi::OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let target: Vec<u16> = std::ffi::OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!(
                "ShellExecuteW failed with code {}",
                result as isize
            ));
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("`xdg-open` spawn failed: {}", e))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("No OS-level fallback for this platform".into())
}

/// Read a loopback OAuth redirect without treating browser probes as failure.
///
/// Chrome and Safari often open an extra connection (favicon, preflight, or a
/// speculative socket) before the real callback. A single blocking read used to
/// either consume a partial callback or abort the whole login when that probe
/// reset. Connections are handled independently, and a request is only final
/// once its headers are complete.
fn wait_for_oauth_callback(
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

fn is_retryable_io(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
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

struct ParsedOAuthRequest {
    method: String,
    path: String,
    query: std::collections::HashMap<String, String>,
    origin: Option<String>,
    headers_complete: bool,
}

#[derive(Debug)]
enum CallbackDecision {
    Code(String),
    OAuthError(String),
    Preflight,
    Ignore,
}

const OAUTH_HEADER_LIMIT: usize = 16 * 1024;
const OAUTH_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

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

fn looks_like_http(buf: &[u8]) -> bool {
    buf.starts_with(b"GET ")
        || buf.starts_with(b"HEAD ")
        || buf.starts_with(b"OPTIONS ")
        || buf.starts_with(b"POST ")
}

fn read_http_headers(stream: &mut std::net::TcpStream) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    stream.set_read_timeout(Some(OAUTH_READ_TIMEOUT))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if http_headers_complete(&buf) || buf.len() >= OAUTH_HEADER_LIMIT {
            break;
        }
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                let room = OAUTH_HEADER_LIMIT.saturating_sub(buf.len());
                let n = n.min(room);
                buf.extend_from_slice(&tmp[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if is_retryable_io(&e) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(buf)
}

fn http_headers_complete(buf: &[u8]) -> bool {
    buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.windows(2).any(|w| w == b"\n\n")
}

fn request_line_complete(buf: &[u8]) -> bool {
    buf.contains(&b'\n')
}

fn parse_oauth_http_request(buf: &[u8]) -> Result<ParsedOAuthRequest, &'static str> {
    let headers_complete = http_headers_complete(buf);
    if !headers_complete && !request_line_complete(buf) {
        return Err("incomplete");
    }
    let text = String::from_utf8_lossy(buf);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized.split('\n');
    let first = lines.next().unwrap_or("");
    if first.is_empty() || !first.contains(" HTTP/") {
        return Err("incomplete");
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").to_string();
    let query = target
        .split_once('?')
        .map(|(_, query)| parse_query(query))
        .unwrap_or_default();
    let mut origin = None;
    if headers_complete {
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("origin") {
                    origin = Some(value.trim().to_string());
                }
            }
        }
    }
    Ok(ParsedOAuthRequest {
        method,
        path,
        query,
        origin,
        headers_complete,
    })
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            let key = urlencoding::decode(kv.next()?).ok()?.into_owned();
            let value = urlencoding::decode(kv.next().unwrap_or(""))
                .ok()?
                .into_owned();
            Some((key, value))
        })
        .collect()
}

fn decide_oauth_callback(req: &ParsedOAuthRequest, expected_state: &str) -> CallbackDecision {
    if req.method.eq_ignore_ascii_case("OPTIONS") {
        return CallbackDecision::Preflight;
    }
    if !req.method.eq_ignore_ascii_case("GET") && !req.method.eq_ignore_ascii_case("HEAD") {
        return CallbackDecision::Ignore;
    }
    let state_matches = req
        .query
        .get("state")
        .is_some_and(|state| state == expected_state);
    if let Some(code) = req.query.get("code").filter(|code| !code.is_empty()) {
        if state_matches {
            return CallbackDecision::Code(code.clone());
        }
        return CallbackDecision::Ignore;
    }
    if let Some(err) = req.query.get("error").filter(|err| !err.is_empty()) {
        if state_matches || !req.query.contains_key("state") {
            return CallbackDecision::OAuthError(err.clone());
        }
    }
    CallbackDecision::Ignore
}

fn is_probe_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "favicon.ico" | "robots.txt" | "apple-touch-icon.png" | "apple-touch-icon-precomposed.png"
    )
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
fn send_oauth_response(
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
mod tests {
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
}
#[tauri::command]
pub async fn gcal_disconnect(state: State<'_, GCalState>) -> Result<(), String> {
    let mut gcal = state.client.lock().await;
    gcal.disconnect();
    log::info!("Google Calendar disconnected");
    Ok(())
}

/// Sync this week's timetable to Google Calendar
#[tauri::command]
pub async fn gcal_sync_timetable(
    state: State<'_, GCalState>,
    entries: Vec<CalendarSyncEntry>,
    week_label: String,
) -> Result<String, String> {
    let mut gcal = state.client.lock().await;
    gcal.sync_timetable(entries, week_label).await
}

#[tauri::command]
pub async fn gcal_clear_calendar(
    state: State<'_, GCalState>,
    delete_calendar: bool,
) -> Result<String, String> {
    let mut gcal = state.client.lock().await;
    gcal.clear_calendar(delete_calendar).await
}
