use reqwest::Client;
use std::sync::Arc;

use crate::client::{build_http_client, data_dir, load_cookie_jar, new_cookie_client, save_cookie_jar};

const KWIC_COOKIES_FILE: &str = "kwic_portal_cookies.json";

/// Check if KWIC Portal response indicates session expired
pub(crate) fn is_kwic_session_expired(body: &str) -> bool {
    // Redirected to login page (KWIC shows its own login form)
    if body.contains("linkCommonLogin") && body.contains(r#"class="login-body""#) {
        return true;
    }
    // KWIC-specific login page
    if body.contains("type=\"password\"") && body.contains("kwic.kwansei.ac.jp") {
        return true;
    }
    // SAML redirect
    if body.contains("sso.kwansei.ac.jp") && body.contains("SAMLRequest") {
        return true;
    }
    false
}

pub const KWIC_SESSION_EXPIRED_MSG: &str = "KWICセッションが期限切れです。再ログインしてください。";
pub const KWIC_AUTH_REQUIRED_MSG: &str = "KWICポータルにログインしてください";

/// HTTP client for KWIC Portal (kwic.kwansei.ac.jp)
pub struct KwicClient {
    pub http: Client,
    pub cookie_store: Arc<reqwest_cookie_store::CookieStoreMutex>,
    pub authenticated: bool,
}

impl KwicClient {
    pub fn new() -> Self {
        let (cookie_store, http) = new_cookie_client();
        Self {
            http,
            cookie_store,
            authenticated: false,
        }
    }

    /// Save KWIC Portal cookies to disk
    pub fn save_session(&self) {
        if !self.authenticated {
            log::warn!("KWIC save_session skipped: not authenticated");
            return;
        }
        save_cookie_jar(&self.cookie_store, KWIC_COOKIES_FILE);
        log::info!("KWIC Portal cookies saved");
    }

    /// Try to restore session from disk
    pub fn try_restore_session(&mut self) -> bool {
        match load_cookie_jar(KWIC_COOKIES_FILE) {
            Some(store) => {
                let cookie_store = Arc::new(
                    reqwest_cookie_store::CookieStoreMutex::new(store),
                );
                self.http = build_http_client(cookie_store.clone());
                self.cookie_store = cookie_store;
                self.authenticated = true;
                log::info!("KWIC Portal session restored from disk");
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.authenticated = false;
        if let Err(e) = std::fs::remove_file(data_dir().join(KWIC_COOKIES_FILE)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("KWIC clear: failed to delete cookies file: {}", e);
            }
        }
        let (cookie_store, http) = new_cookie_client();
        self.http = http;
        self.cookie_store = cookie_store;
    }

}
