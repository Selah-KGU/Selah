use reqwest::Client;
use std::sync::Arc;

use crate::client::{build_http_client, data_dir, new_cookie_client};

const KWIC_BASE: &str = "https://kwic.kwansei.ac.jp";
const KWIC_COOKIES_FILE: &str = "kwic_portal_cookies.json";

/// Check if KWIC Portal response indicates session expired
fn is_kwic_session_expired(body: &str) -> bool {
    // Redirected to login page
    if body.contains("linkCommonLogin") && body.contains("kwic_logo") {
        return true;
    }
    // login page with password field
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
            return;
        }
        let dir = data_dir();
        let store = self.cookie_store.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = Vec::new();
        if cookie_store::serde::json::save(&store, &mut buf).is_ok() {
            if let Err(e) = std::fs::write(dir.join(KWIC_COOKIES_FILE), &buf) {
                log::warn!("Failed to save KWIC Portal cookies: {}", e);
            } else {
                log::info!("KWIC Portal cookies saved");
            }
        }
    }

    /// Try to restore session from disk
    pub fn try_restore_session(&mut self) -> bool {
        let dir = data_dir();
        let cookies_path = dir.join(KWIC_COOKIES_FILE);
        if !cookies_path.exists() {
            return false;
        }
        match std::fs::File::open(&cookies_path) {
            Ok(file) => {
                let reader = std::io::BufReader::new(file);
                match cookie_store::serde::json::load(reader) {
                    Ok(store) => {
                        let cookie_store = Arc::new(
                            reqwest_cookie_store::CookieStoreMutex::new(store),
                        );
                        self.http = build_http_client(cookie_store.clone());
                        self.cookie_store = cookie_store;
                        self.authenticated = true;
                        log::info!("KWIC Portal session restored from disk");
                        true
                    }
                    Err(e) => {
                        log::warn!("Failed to load KWIC Portal cookies: {}", e);
                        false
                    }
                }
            }
            Err(_) => false,
        }
    }

    /// Initiate KWIC Portal SAML auth — returns the Okta SSO URL
    pub async fn initiate_saml_auth(&self) -> Result<String, String> {
        // KWIC Portal login redirects to Okta SSO
        let url = format!("{}/login", KWIC_BASE);
        let mut current_url = url;

        for i in 0..10 {
            log::info!("kwic_portal initiate_saml_auth step {}: GET {}", i, &current_url[..120.min(current_url.len())]);
            let resp = self.http.get(&current_url).send().await
                .map_err(|e| format!("KWIC Portal接続失敗: {}", e))?;

            let status = resp.status();
            let headers = resp.headers().clone();

            if status.is_redirection() {
                if let Some(loc) = headers.get("location") {
                    let loc_str = loc.to_str().unwrap_or_default();
                    let next_url = if loc_str.starts_with('/') {
                        format!("{}{}", KWIC_BASE, loc_str)
                    } else {
                        loc_str.to_string()
                    };

                    if next_url.contains("sso.kwansei.ac.jp") && next_url.contains("/sso/saml") {
                        log::info!("KWIC Portal: captured Okta SAML URL");
                        return Ok(next_url);
                    }

                    current_url = next_url;
                    continue;
                }
            }

            let body = resp.text().await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?;

            // Check for SAML form or meta refresh
            if body.contains("sso.kwansei.ac.jp") {
                // Try to extract Okta SAML URL from body
                if let Some(url) = extract_saml_url(&body) {
                    return Ok(url);
                }
            }

            // Check for meta refresh
            if let Some(refresh_url) = extract_meta_refresh(&body) {
                current_url = if refresh_url.starts_with("http") {
                    refresh_url
                } else {
                    format!("{}/{}", KWIC_BASE, refresh_url.trim_start_matches('/'))
                };
                continue;
            }

            // Check for JavaScript redirect (common in KWIC portal)
            if let Some(js_url) = extract_js_redirect(&body) {
                current_url = js_url;
                continue;
            }

            return Err(format!("KWIC Portal SAML URLを取得できませんでした (Status: {})", status));
        }

        Err("リダイレクトループが発生しました".into())
    }

    /// Complete login by POSTing SAMLResponse to KWIC Portal's ACS endpoint
    pub async fn complete_saml_login(
        &mut self,
        saml_response: &str,
        relay_state: &str,
        acs_url: &str,
    ) -> Result<(), String> {
        log::info!("KWIC Portal: POSTing SAMLResponse to ACS: {}", &acs_url[..80.min(acs_url.len())]);

        let mut params = vec![("SAMLResponse", saml_response)];
        if !relay_state.is_empty() {
            params.push(("RelayState", relay_state));
        }

        let resp = self.http.post(acs_url)
            .form(&params)
            .send().await
            .map_err(|e| format!("KWIC Portal ACS POST失敗: {}", e))?;

        let status = resp.status();
        log::info!("KWIC Portal ACS response: {}", status);

        // Follow redirects to establish session
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                let next_url = if loc_str.starts_with('/') {
                    format!("{}{}", KWIC_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                let _ = self.fetch_page_internal(&next_url).await;
            }
        }

        self.authenticated = true;
        log::info!("KWIC Portal session established");
        Ok(())
    }

    /// Internal fetch that follows redirects
    async fn fetch_page_internal(&self, url: &str) -> Result<String, String> {
        let mut current_url = url.to_string();
        for i in 0..10 {
            let resp = self.http.get(&current_url).send().await
                .map_err(|e| format!("リクエスト失敗: {}", e))?;

            let status = resp.status();
            if status.is_redirection() {
                if let Some(loc) = resp.headers().get("location") {
                    let loc_str = loc.to_str().unwrap_or_default();
                    current_url = if loc_str.starts_with('/') {
                        format!("{}{}", KWIC_BASE, loc_str)
                    } else {
                        loc_str.to_string()
                    };
                    log::debug!("KWIC Portal redirect #{} → {}", i + 1, &current_url[..120.min(current_url.len())]);
                    if current_url.contains("sso.kwansei.ac.jp") {
                        return Err(KWIC_SESSION_EXPIRED_MSG.into());
                    }
                    continue;
                }
            }

            let body = resp.text().await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?;

            if is_kwic_session_expired(&body) {
                return Err(KWIC_SESSION_EXPIRED_MSG.into());
            }

            return Ok(body);
        }
        Err("リダイレクトが多すぎます".into())
    }

    /// Fetch a page from KWIC Portal (path relative to KWIC_BASE or absolute URL)
    pub async fn fetch_page(&self, path: &str) -> Result<String, String> {
        if !self.authenticated {
            return Err(KWIC_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", KWIC_BASE, path)
        };
        self.fetch_page_internal(&url).await
    }

    /// Fetch JSON from the KWIC Portal API
    pub async fn fetch_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        if !self.authenticated {
            return Err(KWIC_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", KWIC_BASE, path)
        };

        let resp = self.http.get(&url)
            .header("Accept", "application/json")
            .send().await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        let status = resp.status();
        if status.is_redirection() {
            return Err(KWIC_SESSION_EXPIRED_MSG.into());
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }

        let text = resp.text().await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;

        if is_kwic_session_expired(&text) {
            return Err(KWIC_SESSION_EXPIRED_MSG.into());
        }

        serde_json::from_str(&text)
            .map_err(|e| format!("JSON解析失敗: {} (first 200 chars: {})", e, &text[..200.min(text.len())]))
    }

    /// POST a form to KWIC Portal and return the HTML response body
    pub async fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Result<String, String> {
        if !self.authenticated {
            return Err(KWIC_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", KWIC_BASE, path)
        };

        let resp = self.http.post(&url)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(form)
            .send().await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        let body = resp.text().await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;

        if is_kwic_session_expired(&body) {
            return Err(KWIC_SESSION_EXPIRED_MSG.into());
        }

        Ok(body)
    }

    pub fn clear(&mut self) {
        self.authenticated = false;
        let _ = std::fs::remove_file(data_dir().join(KWIC_COOKIES_FILE));
        let (cookie_store, http) = new_cookie_client();
        self.http = http;
        self.cookie_store = cookie_store;
    }

}

// ============ URL Extraction Helpers ============

fn extract_saml_url(body: &str) -> Option<String> {
    // Look for Okta SAML URL in various formats
    let patterns = [
        r#"action=""#,
        r#"href=""#,
        r#"url=""#,
    ];
    for pattern in &patterns {
        if let Some(start) = body.find(pattern) {
            let rest = &body[start + pattern.len()..];
            if let Some(end) = rest.find('"') {
                let url = &rest[..end];
                let url = url
                    .replace("&#x3a;", ":")
                    .replace("&#x2f;", "/")
                    .replace("&#x3d;", "=")
                    .replace("&#x3f;", "?")
                    .replace("&#x26;", "&")
                    .replace("&amp;", "&");
                if url.contains("sso.kwansei.ac.jp") {
                    return Some(url);
                }
            }
        }
    }
    None
}

fn extract_meta_refresh(body: &str) -> Option<String> {
    let lower = body.to_lowercase();
    if let Some(idx) = lower.find("http-equiv=\"refresh\"") {
        let slice = &body[idx..];
        if let Some(url_start) = slice.to_lowercase().find("url=") {
            let rest = &slice[url_start + 4..];
            let rest = rest.trim_start_matches(['\'', '"']);
            if let Some(end) = rest.find(['"', '\'', '>']) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn extract_js_redirect(body: &str) -> Option<String> {
    // Match patterns like: window.location.href = "..." or location.replace("...")
    for pattern in ["window.location.href", "location.href", "location.replace"] {
        if let Some(idx) = body.find(pattern) {
            let rest = &body[idx..];
            // Find the URL in quotes
            for quote in ['"', '\''] {
                if let Some(start) = rest.find(quote) {
                    let inner = &rest[start + 1..];
                    if let Some(end) = inner.find(quote) {
                        let url = &inner[..end];
                        if url.starts_with("http") {
                            return Some(url.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}
