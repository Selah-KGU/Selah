use reqwest::Client;
use std::sync::Arc;

use crate::client::{
    fresh_cookie_client_clearing, new_cookie_client, save_service_cookie_jar,
    try_restore_cookie_client,
};

pub(crate) const KWIC_COOKIES_KEY: &str = "kwic_cookie_jar";

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
        save_service_cookie_jar(
            self.authenticated,
            &self.cookie_store,
            KWIC_COOKIES_KEY,
            "KWIC Portal",
        );
    }

    /// Try to restore session from disk
    pub fn try_restore_session(&mut self) -> bool {
        let Some(parts) = try_restore_cookie_client(KWIC_COOKIES_KEY) else {
            return false;
        };
        self.http = parts.http;
        self.cookie_store = parts.cookie_store;
        self.authenticated = true;
        log::info!("KWIC Portal session restored from disk");
        true
    }

    pub fn clear(&mut self) {
        self.authenticated = false;
        let parts = fresh_cookie_client_clearing(KWIC_COOKIES_KEY);
        self.http = parts.http;
        self.cookie_store = parts.cookie_store;
    }
}
