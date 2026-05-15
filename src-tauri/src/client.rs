use reqwest::redirect::Policy;
use reqwest::Client;
use std::sync::Arc;

use crate::auth::AuthSession;
use crate::config;

/// Safe byte-limited preview of a string for log / error messages.
/// Adjusts the boundary to avoid splitting a multi-byte UTF-8 character.
pub(crate) fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if max_bytes >= s.len() {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Find the nearest valid char boundary at or before `byte_pos`.
pub(crate) fn floor_char_boundary(s: &str, byte_pos: usize) -> usize {
    let mut pos = byte_pos.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Find the nearest valid char boundary at or after `byte_pos`.
pub(crate) fn ceil_char_boundary(s: &str, byte_pos: usize) -> usize {
    let mut pos = byte_pos.min(s.len());
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

const SESSION_FILE: &str = "session.json";
const COOKIES_FILE: &str = "cookies.json";

pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Build a reqwest HTTP client with shared configuration (no-redirect, UA, cookie provider).
pub(crate) fn build_http_client(
    cookie_store: Arc<reqwest_cookie_store::CookieStoreMutex>,
) -> Client {
    Client::builder()
        .cookie_provider(cookie_store)
        .redirect(Policy::none())
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

/// Create a fresh cookie store + HTTP client pair.
pub(crate) fn new_cookie_client() -> (Arc<reqwest_cookie_store::CookieStoreMutex>, Client) {
    let cookie_store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
        cookie_store::CookieStore::default(),
    ));
    let http = build_http_client(cookie_store.clone());
    (cookie_store, http)
}

/// Find the soonest-expiring cookie in a cookie store and return seconds until it expires.
/// Returns None if all cookies are session-only (no explicit expiry).
pub(crate) fn soonest_cookie_expiry(store: &reqwest_cookie_store::CookieStoreMutex) -> Option<i64> {
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    let mut soonest: Option<i64> = None;
    for cookie in store.iter_unexpired() {
        if let cookie_store::CookieExpiration::AtUtc(expiry) = &cookie.expires {
            // Compute seconds remaining using the time crate re-exported by cookie_store
            let now = ::time::OffsetDateTime::now_utc();
            let remaining = (*expiry - now).whole_seconds();
            soonest = Some(soonest.map_or(remaining, |s: i64| s.min(remaining)));
        }
    }
    soonest
}

/// Check if an HTML response body indicates the session has expired.
/// This catches SSO login forms, Shibboleth redirects, and various session timeout pages.
pub(crate) fn is_session_expired_body(body: &str) -> bool {
    // SSO login form redirect
    if body.contains("action=\"UnSSOLoginControl")
        || body.contains("action=\"/uniasv2/UnSSOLoginControl")
    {
        return true;
    }
    // Okta/Shibboleth SSO redirect in meta refresh or JS
    if body.contains("sso.kwansei.ac.jp")
        && (body.contains("saml") || body.contains("redirect") || body.contains("location.href"))
    {
        return true;
    }
    // Japanese session timeout / error messages from the app
    if body.contains("セッションがタイムアウト") || body.contains("セッション切れ")
    {
        return true;
    }
    // Struts token error or "不正なアクセス" sometimes means session lost
    if body.contains("不正なアクセスです") && !body.contains("class=\"course") {
        return true;
    }
    // Generic login form detection — page has a password input likely means SSO redirect
    if body.contains("type=\"password\"") && body.contains("login") && !body.contains("uniasv2") {
        return true;
    }
    // KG-Course returns 200 with empty hidden inputs when server-side session is stale
    // (cookie accepted but user data missing). The student ID field exists but is blank.
    // Student IDs always start with a letter (e.g. "B..." or "D..."), so an empty
    // value=\"\" next to lblScrgNo/hdnScrgNo is a reliable stale-session indicator.
    if body.contains("lblScrgNo") || body.contains("hdnScrgNo") {
        // Look for the pattern: name="lblScrgNo" ... value=""
        // These hidden inputs exist on all KGC pages with the student header.
        if let Some(pos) = body.find("lblScrgNo").or_else(|| body.find("hdnScrgNo")) {
            // Check the value attribute in the surrounding area (within 200 chars)
            let mut region_end = (pos + 200).min(body.len());
            while region_end < body.len() && !body.is_char_boundary(region_end) {
                region_end += 1;
            }
            let region = &body[pos..region_end];
            if let Some(vpos) = region.find("value=\"") {
                let after_value = &region[vpos + 7..];
                if after_value.starts_with('"') {
                    // value="" — empty student ID, session is stale
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) const SESSION_EXPIRED_MSG: &str = "セッションが期限切れです。再ログインしてください。";

pub(crate) fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let dir = base.join("com.kgu.selah");
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

/// Save a cookie jar to a JSON file in the data directory.
pub(crate) fn save_cookie_jar(store: &reqwest_cookie_store::CookieStoreMutex, filename: &str) {
    let dir = data_dir();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    let mut buf = Vec::new();
    if cookie_store::serde::json::save(&store, &mut buf).is_ok() {
        let path = dir.join(filename);
        if let Err(e) = std::fs::write(&path, &buf) {
            log::warn!("Failed to save cookies ({}): {}", filename, e);
        } else {
            #[cfg(unix)]
            {
                let _ = std::fs::set_permissions(
                    &path,
                    std::os::unix::fs::PermissionsExt::from_mode(0o600),
                );
            }
        }
    }
}

/// Load a cookie jar from a JSON file in the data directory.
/// Returns None if the file doesn't exist or can't be parsed.
pub(crate) fn load_cookie_jar(filename: &str) -> Option<cookie_store::CookieStore> {
    let path = data_dir().join(filename);
    let file = std::fs::File::open(&path).ok()?;
    let reader = std::io::BufReader::new(file);
    match cookie_store::serde::json::load(reader) {
        Ok(store) => Some(store),
        Err(e) => {
            log::warn!("Failed to load cookies ({}): {}", filename, e);
            None
        }
    }
}

/// Shared redirect-following GET fetch used by all three service clients.
/// Follows up to 10 redirects, detects SSO redirects and expired-session body patterns.
pub(crate) async fn fetch_with_redirect(
    http: &Client,
    url: &str,
    base_url: &str,
    expired_msg: &str,
    is_body_expired: fn(&str) -> bool,
) -> Result<String, String> {
    let mut current_url = url.to_string();
    for i in 0..10 {
        let resp = http
            .get(&current_url)
            .send()
            .await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;
        let status = resp.status();
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                current_url = if loc_str.starts_with('/') {
                    format!("{}{}", base_url, loc_str)
                } else {
                    loc_str.to_string()
                };
                log::debug!(
                    "redirect #{} -> {}",
                    i + 1,
                    safe_truncate(&current_url, 120)
                );
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(expired_msg.into());
                }
                continue;
            }
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let preview: String = body.chars().take(500).collect();
            log::debug!("HTTP {} body (first 500 chars): {}", status, preview);
            return Err(format!("HTTP {}", status));
        }
        let body = resp
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        if is_body_expired(&body) {
            return Err(expired_msg.into());
        }
        return Ok(body);
    }
    Err("リダイレクトが多すぎます".into())
}

/// Shared POST-then-follow-redirects used by all three service clients.
/// Sends a pre-built request, then follows any redirect with `fetch_with_redirect` (GET chain).
pub(crate) async fn send_and_follow_redirect(
    http: &Client,
    request: reqwest::RequestBuilder,
    base_url: &str,
    expired_msg: &str,
    is_body_expired: fn(&str) -> bool,
) -> Result<String, String> {
    let resp = request
        .send()
        .await
        .map_err(|e| format!("リクエスト失敗: {}", e))?;

    let status = resp.status();
    let resp_url = resp.url().to_string();
    log::debug!(
        "send_and_follow_redirect: status={}, url={}",
        status,
        safe_truncate(&resp_url, 120)
    );
    if status.is_redirection() {
        if let Some(loc) = resp.headers().get("location") {
            let loc_str = loc.to_str().unwrap_or_default();
            let next_url = if loc_str.starts_with('/') {
                format!("{}{}", base_url, loc_str)
            } else {
                loc_str.to_string()
            };
            if next_url.contains("sso.kwansei.ac.jp") {
                return Err(expired_msg.into());
            }
            return fetch_with_redirect(http, &next_url, base_url, expired_msg, is_body_expired)
                .await;
        }
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(500).collect();
        log::debug!("HTTP {} body (first 500 chars): {}", status, preview);
        return Err(format!("HTTP {}", status));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("レスポンス読取失敗: {}", e))?;
    if is_body_expired(&text) {
        return Err(expired_msg.into());
    }
    Ok(text)
}

/// Convenience: POST a URL-encoded form and follow redirects.
pub(crate) async fn post_form_with_redirect<I, K, V>(
    http: &Client,
    url: &str,
    base_url: &str,
    expired_msg: &str,
    is_body_expired: fn(&str) -> bool,
    params: I,
    extra_headers: &[(&str, &str)],
) -> Result<String, String>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let form_data: Vec<(String, String)> = params
        .into_iter()
        .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
        .collect();
    let mut builder = http.post(url).form(&form_data);
    for &(k, v) in extra_headers {
        builder = builder.header(k, v);
    }
    send_and_follow_redirect(http, builder, base_url, expired_msg, is_body_expired).await
}

/// Fetch a KG-Course page using a raw reqwest Client (no auth guard).
/// Used by headless refresh to verify session without holding the KgcClient mutex.
pub(crate) async fn fetch_page_with(http: &Client, url: &str) -> Result<String, String> {
    fetch_with_redirect(
        http,
        url,
        config::KG_COURSE_BASE,
        SESSION_EXPIRED_MSG,
        is_session_expired_body,
    )
    .await
}

/// Main HTTP client for KG-Course (kg-course.kwansei.ac.jp)
pub struct KgcClient {
    pub http: Client,
    pub cookie_store: Arc<reqwest_cookie_store::CookieStoreMutex>,
    pub session: Option<AuthSession>,
}

impl KgcClient {
    pub fn new() -> Self {
        let (cookie_store, http) = new_cookie_client();
        Self {
            http,
            cookie_store,
            session: None,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.session.is_some()
    }

    pub fn clear_session(&mut self) {
        self.session = None;
        // Delete persisted session files
        let dir = data_dir();
        if let Err(e) = std::fs::remove_file(dir.join(SESSION_FILE)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("KGC clear_session: failed to delete session file: {}", e);
            }
        }
        if let Err(e) = std::fs::remove_file(dir.join(COOKIES_FILE)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("KGC clear_session: failed to delete cookies file: {}", e);
            }
        }
        // Recreate client with fresh cookie jar
        let (cookie_store, http) = new_cookie_client();
        self.http = http;
        self.cookie_store = cookie_store;
    }

    /// Save session and cookies to disk
    pub fn save_session(&self) {
        let dir = data_dir();

        // Save AuthSession
        if let Some(session) = &self.session {
            if let Ok(json) = serde_json::to_string_pretty(session) {
                let path = dir.join(SESSION_FILE);
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("Failed to save session: {}", e);
                } else {
                    #[cfg(unix)]
                    {
                        let _ = std::fs::set_permissions(
                            &path,
                            std::os::unix::fs::PermissionsExt::from_mode(0o600),
                        );
                    }
                }
            }
        }

        // Save cookies
        save_cookie_jar(&self.cookie_store, COOKIES_FILE);
    }

    /// Try to restore session and cookies from disk.
    /// Returns true if session was restored (still needs validation).
    pub fn try_restore_session(&mut self) -> bool {
        let dir = data_dir();
        let session_path = dir.join(SESSION_FILE);
        if !session_path.exists() {
            return false;
        }

        // Load session
        let session: AuthSession = match std::fs::read_to_string(&session_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(s) => s,
            None => {
                log::warn!("KGC try_restore_session: failed to read/parse session file");
                return false;
            }
        };

        // Load cookies
        match load_cookie_jar(COOKIES_FILE) {
            Some(store) => {
                let cookie_store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(store));
                self.http = build_http_client(cookie_store.clone());
                self.cookie_store = cookie_store;
                self.session = Some(session);
                log::info!("Session restored from disk");
                true
            }
            None => {
                log::warn!("KGC try_restore_session: failed to load cookies from disk");
                false
            }
        }
    }

    /// Return seconds until the soonest-expiring session cookie expires.
    /// Returns None if there are no time-limited cookies (all SessionEnd).
    pub fn soonest_cookie_expiry_secs(&self) -> Option<i64> {
        soonest_cookie_expiry(&self.cookie_store)
    }
}
