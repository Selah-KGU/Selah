use reqwest::redirect::Policy;
use reqwest::Client;
use std::sync::Arc;

use crate::auth::AuthSession;
use crate::config;

const SESSION_FILE: &str = "session.json";
const COOKIES_FILE: &str = "cookies.json";

pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Build a reqwest HTTP client with shared configuration (no-redirect, UA, cookie provider).
pub(crate) fn build_http_client(cookie_store: Arc<reqwest_cookie_store::CookieStoreMutex>) -> Client {
    Client::builder()
        .cookie_provider(cookie_store)
        .redirect(Policy::none())
        .user_agent(USER_AGENT)
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

/// Check if an HTML response body indicates the session has expired.
/// This catches SSO login forms, Shibboleth redirects, and various session timeout pages.
fn is_session_expired_body(body: &str) -> bool {
    // SSO login form redirect
    if body.contains("action=\"UnSSOLoginControl") || body.contains("action=\"/uniasv2/UnSSOLoginControl") {
        return true;
    }
    // Okta/Shibboleth SSO redirect in meta refresh or JS
    if body.contains("sso.kwansei.ac.jp") && (body.contains("saml") || body.contains("redirect") || body.contains("location.href")) {
        return true;
    }
    // Japanese session timeout / error messages from the app
    if body.contains("セッションがタイムアウト") || body.contains("セッション切れ") {
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
    false
}

const SESSION_EXPIRED_MSG: &str = "セッションが期限切れです。再ログインしてください。";

pub(crate) fn data_dir() -> std::path::PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = base.join("com.kgu.selah");
    let _ = std::fs::create_dir_all(&dir);
    // Migrate from old paths
    for old_name in ["com.kwic.app", "com.haru.kwic"] {
        let old = base.join(old_name);
        if old.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&old) {
                for entry in entries.flatten() {
                    let dest = dir.join(entry.file_name());
                    if !dest.exists() {
                        let _ = std::fs::rename(entry.path(), dest);
                    }
                }
            }
            let _ = std::fs::remove_dir(&old);
        }
    }
    dir
}

/// Save a cookie jar to a JSON file in the data directory.
pub(crate) fn save_cookie_jar(store: &reqwest_cookie_store::CookieStoreMutex, filename: &str) {
    let dir = data_dir();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    let mut buf = Vec::new();
    if cookie_store::serde::json::save(&store, &mut buf).is_ok() {
        if let Err(e) = std::fs::write(dir.join(filename), &buf) {
            log::warn!("Failed to save cookies ({}): {}", filename, e);
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
        let _ = std::fs::remove_file(dir.join(SESSION_FILE));
        let _ = std::fs::remove_file(dir.join(COOKIES_FILE));
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
                if let Err(e) = std::fs::write(dir.join(SESSION_FILE), json) {
                    log::warn!("Failed to save session: {}", e);
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
            None => return false,
        };

        // Load cookies
        match load_cookie_jar(COOKIES_FILE) {
            Some(store) => {
                let cookie_store = Arc::new(
                    reqwest_cookie_store::CookieStoreMutex::new(store),
                );
                self.http = build_http_client(cookie_store.clone());
                self.cookie_store = cookie_store;
                self.session = Some(session);
                log::info!("Session restored from disk");
                true
            }
            None => false,
        }
    }

    /// Fetch a page from the KG-Course system (requires authentication)
    pub async fn fetch_page(&self, path: &str) -> Result<String, String> {
        if !self.is_authenticated() {
            return Err("認証されていません".into());
        }

        use std::io::Write;
        #[cfg(debug_assertions)]
        let mut dbg = std::fs::OpenOptions::new().create(true).append(true)
            .open(std::env::temp_dir().join("kgc-fetch.log")).ok();
        #[cfg(not(debug_assertions))]
        let mut dbg: Option<std::fs::File> = None;
        macro_rules! dbg_log {
            ($($arg:tt)*) => {
                if let Some(ref mut f) = dbg { let _ = writeln!(f, $($arg)*); }
            }
        }

        let url = format!("{}{}", config::KG_COURSE_BASE, path);
        let mut current_url = url;
        dbg_log!("[FETCH] start: {}", current_url);
        
        // Follow redirects manually to maintain cookies
        for i in 0..10 {
            let resp = self.http.get(&current_url).send().await
                .map_err(|e| format!("リクエスト失敗: {}", e))?;
            
            let status = resp.status();
            dbg_log!("[FETCH] #{} {} -> {}", i, current_url, status);
            if status.is_redirection() {
                if let Some(loc) = resp.headers().get("location") {
                    let loc_str = loc.to_str().unwrap_or_default();
                    if loc_str.starts_with('/') {
                        current_url = format!("{}{}", config::KG_COURSE_BASE, loc_str);
                    } else {
                        current_url = loc_str.to_string();
                    }
                    dbg_log!("[FETCH] redirect -> {}", current_url);
                    // If redirected to SSO, session is expired
                    if current_url.contains("sso.kwansei.ac.jp") {
                        dbg_log!("[FETCH] SSO redirect detected, session expired");
                        return Err(SESSION_EXPIRED_MSG.into());
                    }
                    continue;
                }
            }

            if !status.is_success() {
                dbg_log!("[FETCH] non-success status: {}", status);
                return Err(format!("HTTP {}", status));
            }

            let body = resp.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?;
            dbg_log!("[FETCH] body length: {}, has UnSSO: {}", body.len(), body.contains("UnSSOLoginControl"));
            // Check if response is a SSO login page (form action pointing to UnSSOLoginControl)
            if is_session_expired_body(&body) {
                return Err(SESSION_EXPIRED_MSG.into());
            }
            return Ok(body);
        }

        Err("リダイレクトが多すぎます".into())
    }

    /// POST a form to the KG-Course system (requires authentication)
    pub async fn post_form(&self, path: &str, params: &[(String, String)]) -> Result<String, String> {
        if !self.is_authenticated() {
            return Err("認証されていません".into());
        }

        let url = format!("{}{}", config::KG_COURSE_BASE, path);
        let resp = self.http.post(&url)
            .header("Referer", &format!("{}/uniasv2/ARF010.do", config::KG_COURSE_BASE))
            .header("Origin", config::KG_COURSE_BASE)
            .form(params)
            .send().await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        let status = resp.status();
        log::debug!("[POST_FORM] {} -> status={}", path, status);

        // Follow redirects manually like fetch_page
        let mut current_url = String::new();
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                current_url = if loc_str.starts_with('/') {
                    format!("{}{}", config::KG_COURSE_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                log::debug!("[POST_FORM] redirect -> {}", current_url);
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(SESSION_EXPIRED_MSG.into());
                }
            }
        }

        if !current_url.is_empty() {
            // Follow redirect chain (up to 10)
            for _ in 0..10 {
                let resp2 = self.http.get(&current_url).send().await
                    .map_err(|e| format!("リダイレクト失敗: {}", e))?;
                let st = resp2.status();
                if st.is_redirection() {
                    if let Some(loc) = resp2.headers().get("location") {
                        let loc_str = loc.to_str().unwrap_or_default();
                        current_url = if loc_str.starts_with('/') {
                            format!("{}{}", config::KG_COURSE_BASE, loc_str)
                        } else {
                            loc_str.to_string()
                        };
                        log::debug!("[POST_FORM] redirect chain -> {}", current_url);
                        if current_url.contains("sso.kwansei.ac.jp") {
                            return Err(SESSION_EXPIRED_MSG.into());
                        }
                        continue;
                    }
                }
                let body = resp2.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?;
                if is_session_expired_body(&body) {
                    return Err(SESSION_EXPIRED_MSG.into());
                }
                return Ok(body);
            }
            return Err("リダイレクトが多すぎます".into());
        }

        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }

        let body = resp.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        if is_session_expired_body(&body) {
            return Err(SESSION_EXPIRED_MSG.into());
        }
        Ok(body)
    }
}
