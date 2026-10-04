pub(in crate::google_commands) fn is_retryable_io(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

pub(in crate::google_commands) struct ParsedOAuthRequest {
    pub(in crate::google_commands) method: String,
    pub(in crate::google_commands) path: String,
    pub(in crate::google_commands) query: std::collections::HashMap<String, String>,
    pub(in crate::google_commands) origin: Option<String>,
    pub(in crate::google_commands) headers_complete: bool,
}

#[derive(Debug)]
pub(in crate::google_commands) enum CallbackDecision {
    Code(String),
    OAuthError(String),
    Preflight,
    Ignore,
}

const OAUTH_HEADER_LIMIT: usize = 16 * 1024;
const OAUTH_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub(in crate::google_commands) fn looks_like_http(buf: &[u8]) -> bool {
    buf.starts_with(b"GET ")
        || buf.starts_with(b"HEAD ")
        || buf.starts_with(b"OPTIONS ")
        || buf.starts_with(b"POST ")
}

pub(in crate::google_commands) fn read_http_headers(
    stream: &mut std::net::TcpStream,
) -> std::io::Result<Vec<u8>> {
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

pub(in crate::google_commands) fn parse_oauth_http_request(
    buf: &[u8],
) -> Result<ParsedOAuthRequest, &'static str> {
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

pub(in crate::google_commands) fn decide_oauth_callback(
    req: &ParsedOAuthRequest,
    expected_state: &str,
) -> CallbackDecision {
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

pub(in crate::google_commands) fn is_probe_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "favicon.ico" | "robots.txt" | "apple-touch-icon.png" | "apple-touch-icon-precomposed.png"
    )
}
