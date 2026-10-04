use reqwest::Client;
use std::sync::Arc;

use crate::client::{
    fresh_cookie_client_clearing, new_cookie_client, save_service_cookie_jar,
    try_restore_cookie_client,
};

pub(crate) const LUNA_COOKIES_KEY: &str = "luna_cookie_jar";

/// Check if Luna response body indicates session expired
pub(crate) fn is_luna_session_expired(body: &str) -> bool {
    // Redirected to login page
    if body.contains("linkCommonLogin") && body.contains("class=\"login-body\"") {
        return true;
    }
    // SAML redirect
    if body.contains("sso.kwansei.ac.jp") && body.contains("SAMLRequest") {
        return true;
    }
    false
}

pub const LUNA_SESSION_EXPIRED_MSG: &str = "Lunaセッションが期限切れです。再ログインしてください。";
pub const LUNA_AUTH_REQUIRED_MSG: &str = "Lunaにログインしてください";

/// HTTP client for Luna LMS
pub struct LunaClient {
    pub http: Client,
    pub cookie_store: Arc<reqwest_cookie_store::CookieStoreMutex>,
    pub authenticated: bool,
}

impl LunaClient {
    pub fn new() -> Self {
        let (cookie_store, http) = new_cookie_client();
        Self {
            http,
            cookie_store,
            authenticated: false,
        }
    }

    /// Save Luna cookies to disk
    pub fn save_session(&self) {
        save_service_cookie_jar(
            self.authenticated,
            &self.cookie_store,
            LUNA_COOKIES_KEY,
            "Luna",
        );
    }

    /// Try to restore Luna session from disk.
    /// Returns true if cookies were loaded (session still needs server validation).
    pub fn try_restore_session(&mut self) -> bool {
        let Some(parts) = try_restore_cookie_client(LUNA_COOKIES_KEY) else {
            return false;
        };
        self.http = parts.http;
        self.cookie_store = parts.cookie_store;
        self.authenticated = true;
        log::info!("Luna session restored from disk");
        true
    }

    pub fn clear(&mut self) {
        self.authenticated = false;
        let parts = fresh_cookie_client_clearing(LUNA_COOKIES_KEY);
        self.http = parts.http;
        self.cookie_store = parts.cookie_store;
    }
}
