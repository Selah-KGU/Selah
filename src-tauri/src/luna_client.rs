use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::{Arc, LazyLock};

use crate::client::{build_http_client, data_dir, new_cookie_client};

static SEL_FORM: LazyLock<Selector> = LazyLock::new(|| Selector::parse("form").unwrap());
static SEL_HIDDEN_INPUT: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"input[type="hidden"]"#).unwrap());

const LUNA_BASE: &str = "https://luna.kwansei.ac.jp";
const LUNA_COOKIES_FILE: &str = "luna_cookies.json";

/// Check if Luna response body indicates session expired
fn is_luna_session_expired(body: &str) -> bool {
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
        if !self.authenticated {
            return;
        }
        let dir = data_dir();
        let store = self.cookie_store.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = Vec::new();
        if cookie_store::serde::json::save(&store, &mut buf).is_ok() {
            if let Err(e) = std::fs::write(dir.join(LUNA_COOKIES_FILE), &buf) {
                log::warn!("Failed to save Luna cookies: {}", e);
            } else {
                log::info!("Luna cookies saved to {}", dir.display());
            }
        }
    }

    /// Try to restore Luna session from disk.
    /// Returns true if cookies were loaded (session still needs server validation).
    pub fn try_restore_session(&mut self) -> bool {
        let dir = data_dir();
        let cookies_path = dir.join(LUNA_COOKIES_FILE);
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
                        log::info!("Luna session restored from disk");
                        true
                    }
                    Err(e) => {
                        log::warn!("Failed to load Luna cookies: {}", e);
                        false
                    }
                }
            }
            Err(_) => false,
        }
    }

    /// Initiate Luna SAML auth — returns the Okta SSO URL for Luna
    pub async fn initiate_saml_auth(&self) -> Result<String, String> {
        let url = format!("{}/saml/login?disco=true", LUNA_BASE);
        let resp = self.http.get(&url).send().await
            .map_err(|e| format!("Luna接続失敗: {}", e))?;

        let body = resp.text().await
            .map_err(|e| format!("Luna応答読取失敗: {}", e))?;

        // The page auto-submits a form with SAMLRequest to Okta
        // Extract the form action URL
        if let Some(action_start) = body.find("action=\"") {
            let rest = &body[action_start + 8..];
            if let Some(end) = rest.find('"') {
                let action = &rest[..end];
                // Decode HTML entities
                let action = action
                    .replace("&#x3a;", ":")
                    .replace("&#x2f;", "/")
                    .replace("&#x3d;", "=")
                    .replace("&#x3f;", "?")
                    .replace("&#x26;", "&")
                    .replace("&#x25;", "%")
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"");
                if action.contains("sso.kwansei.ac.jp") {
                    return Ok(action);
                }
            }
        }

        Err("Luna SAML URLを取得できませんでした".into())
    }

    /// Complete Luna login by POSTing SAMLResponse to Luna's ACS endpoint
    pub async fn complete_saml_login(
        &mut self,
        saml_response: &str,
        relay_state: &str,
    ) -> Result<(), String> {
        let acs_url = format!("{}/saml/SSO", LUNA_BASE);
        log::info!("Luna: POSTing SAMLResponse to {}", acs_url);

        let mut params = vec![("SAMLResponse", saml_response)];
        if !relay_state.is_empty() {
            params.push(("RelayState", relay_state));
        }

        let resp = self.http.post(&acs_url)
            .form(&params)
            .send().await
            .map_err(|e| format!("Luna ACS POST失敗: {}", e))?;

        let status = resp.status();
        log::info!("Luna ACS response: {}", status);

        // Follow redirects to establish session
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                let next_url = if loc_str.starts_with('/') {
                    format!("{}{}", LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                // Follow the redirect chain
                let _ = self.fetch_page_internal(&next_url).await;
            }
        }

        self.authenticated = true;
        log::info!("Luna session established");
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
                        format!("{}{}", LUNA_BASE, loc_str)
                    } else {
                        loc_str.to_string()
                    };
                    log::debug!("Luna redirect #{} → {}", i + 1, current_url);
                    if current_url.contains("sso.kwansei.ac.jp") {
                        return Err(LUNA_SESSION_EXPIRED_MSG.into());
                    }
                    continue;
                }
            }

            let body = resp.text().await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?;

            if is_luna_session_expired(&body) {
                return Err(LUNA_SESSION_EXPIRED_MSG.into());
            }

            return Ok(body);
        }
        Err("リダイレクトが多すぎます".into())
    }

    /// Fetch a Luna page (path relative to LUNA_BASE)
    pub async fn fetch_page(&self, path: &str) -> Result<String, String> {
        if !self.authenticated {
            return Err(LUNA_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", LUNA_BASE, path)
        };
        self.fetch_page_internal(&url).await
    }

    /// POST form to Luna
    pub async fn post_form(&self, path: &str, params: &[(String, String)]) -> Result<String, String> {
        if !self.authenticated {
            return Err(LUNA_AUTH_REQUIRED_MSG.into());
        }

        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", LUNA_BASE, path)
        };

        let resp = self.http.post(&url)
            .form(params)
            .send().await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        let status = resp.status();

        // Follow redirects
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                let next_url = if loc_str.starts_with('/') {
                    format!("{}{}", LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                if next_url.contains("sso.kwansei.ac.jp") {
                    return Err(LUNA_SESSION_EXPIRED_MSG.into());
                }
                return self.fetch_page_internal(&next_url).await;
            }
        }

        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }

        let body = resp.text().await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;

        if is_luna_session_expired(&body) {
            return Err(LUNA_SESSION_EXPIRED_MSG.into());
        }

        Ok(body)
    }

    pub fn clear(&mut self) {
        self.authenticated = false;
        let _ = std::fs::remove_file(data_dir().join(LUNA_COOKIES_FILE));
        let (cookie_store, http) = new_cookie_client();
        self.http = http;
        self.cookie_store = cookie_store;
    }

    /// POST multipart form to Luna (for file uploads)
    pub async fn post_multipart(&self, path: &str, form: reqwest::multipart::Form) -> Result<String, String> {
        if !self.authenticated {
            return Err(LUNA_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", LUNA_BASE, path)
        };

        let resp = self.http.post(&url)
            .multipart(form)
            .send().await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        let status = resp.status();
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                let next_url = if loc_str.starts_with('/') {
                    format!("{}{}", LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                if next_url.contains("sso.kwansei.ac.jp") {
                    return Err(LUNA_SESSION_EXPIRED_MSG.into());
                }
                return self.fetch_page_internal(&next_url).await;
            }
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }
        let body = resp.text().await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        if is_luna_session_expired(&body) {
            return Err(LUNA_SESSION_EXPIRED_MSG.into());
        }
        Ok(body)
    }

    /// Download a file from Luna and return the bytes
    pub async fn download_file(&self, path: &str) -> Result<Vec<u8>, String> {
        if !self.authenticated {
            return Err(LUNA_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", LUNA_BASE, path)
        };

        let mut current_url = url;
        for i in 0..10 {
            // Parse URL to see what reqwest actually sends (detect double-encoding)
            match reqwest::Url::parse(&current_url) {
                Ok(parsed) => log::info!("download_file #{}: parsed path='{}', query={:?}", i, parsed.path(), parsed.query()),
                Err(e) => log::warn!("download_file #{}: URL parse error: {}", i, e),
            }

            let resp = self.http.get(&current_url)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Sec-Fetch-Dest", "document")
                .header("Sec-Fetch-Mode", "navigate")
                .header("Sec-Fetch-Site", "same-origin")
                .send().await
                .map_err(|e| format!("ダウンロード失敗: {}", e))?;

            let status = resp.status();
            let content_type = resp.headers().get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown")
                .to_string();
            let content_len = resp.headers().get("content-length")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown")
                .to_string();
            let content_disp = resp.headers().get("content-disposition")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            log::info!("download_file: status={}, content-type={}, content-length={}, content-disposition='{}'",
                status, content_type, content_len, content_disp);

            if status.is_redirection() {
                if let Some(loc) = resp.headers().get("location") {
                    let loc_str = loc.to_str().unwrap_or_default();
                    current_url = if loc_str.starts_with('/') {
                        format!("{}{}", LUNA_BASE, loc_str)
                    } else {
                        loc_str.to_string()
                    };
                    if current_url.contains("sso.kwansei.ac.jp") {
                        return Err(LUNA_SESSION_EXPIRED_MSG.into());
                    }
                    log::info!("download_file: redirect → {}", current_url);
                    continue;
                }
            }

            if !status.is_success() {
                return Err(format!("HTTP {}", status));
            }

            let bytes = resp.bytes().await
                .map_err(|e| format!("ダウンロード読取失敗: {}", e))?;
            log::info!("download_file: received {} bytes", bytes.len());
            return Ok(bytes.to_vec());
        }
        Err("リダイレクトが多すぎます".into())
    }

    /// Download a file using separate path and query params (avoids double-encoding)
    /// Mimics a browser form GET submission with proper headers
    pub async fn download_file_with_params(&self, path: &str, params: &[(&str, &str)], referer: Option<&str>) -> Result<Vec<u8>, String> {
        if !self.authenticated {
            return Err(LUNA_AUTH_REQUIRED_MSG.into());
        }
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", LUNA_BASE, path)
        };

        log::info!("download_file_with_params: GET {} with {} params", url, params.len());

        let mut current_url = url.clone();
        let mut is_first = true;
        for i in 0..10 {
            let mut req = if is_first {
                self.http.get(&current_url).query(params)
            } else {
                self.http.get(&current_url)
            };
            // Add headers that a browser sends for form GET navigation
            req = req
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", "ja,en;q=0.9")
                .header("Sec-Fetch-Dest", "document")
                .header("Sec-Fetch-Mode", "navigate")
                .header("Sec-Fetch-Site", "same-origin");
            if let Some(r) = referer {
                req = req.header("Referer", r);
            }

            // Log the actual URL being sent
            log::info!("download_file_with_params #{}: sending request to {}", i, current_url);

            let resp = req.send().await
                .map_err(|e| format!("ダウンロード失敗: {}", e))?;
            is_first = false;

            let status = resp.status();
            // Log all response headers for debugging
            let headers_str: Vec<String> = resp.headers().iter()
                .map(|(k, v)| format!("{}={}", k, v.to_str().unwrap_or("?")))
                .collect();
            log::info!("download_file_with_params #{}: status={}, headers: [{}]",
                i, status, headers_str.join(", "));

            if status.is_redirection() {
                if let Some(loc) = resp.headers().get("location") {
                    let loc_str = loc.to_str().unwrap_or_default();
                    current_url = if loc_str.starts_with('/') {
                        format!("{}{}", LUNA_BASE, loc_str)
                    } else {
                        loc_str.to_string()
                    };
                    if current_url.contains("sso.kwansei.ac.jp") {
                        return Err(LUNA_SESSION_EXPIRED_MSG.into());
                    }
                    continue;
                }
            }

            if !status.is_success() {
                return Err(format!("HTTP {}", status));
            }

            let bytes = resp.bytes().await
                .map_err(|e| format!("ダウンロード読取失敗: {}", e))?;
            log::info!("download_file_with_params: received {} bytes", bytes.len());
            return Ok(bytes.to_vec());
        }
        Err("リダイレクトが多すぎます".into())
    }

    /// Launch an LTI tool (Zoom, Panopto, etc.)
    /// Fetches the LTI launch page, extracts the auto-submit form,
    /// POSTs to the third-party platform and returns the final URL.
    pub async fn launch_lti(&self, path: &str) -> Result<String, String> {
        if !self.authenticated {
            return Err(LUNA_AUTH_REQUIRED_MSG.into());
        }

        // Step 1: GET the LTI launch page from Luna
        let html = self.fetch_page(path).await?;

        // Step 2: Parse auto-submit form (sync — keep scraper types off await points)
        let (action, params) = parse_lti_form(&html)?;

        log::info!("LTI launch: POST to {} with {} params", action, params.len());

        // Step 3: POST form to third-party platform
        let resp = self.http.post(&action)
            .form(&params)
            .send().await
            .map_err(|e| format!("LTI POST失敗: {}", e))?;

        let status = resp.status();
        let mut current_url = if status.is_redirection() {
            resp.headers().get("location")
                .and_then(|l| l.to_str().ok())
                .unwrap_or(&action)
                .to_string()
        } else if status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            // Some LTI flows return another auto-submit form
            match parse_lti_form(&body) {
                Ok((action2, params2)) => {
                    let resp2 = self.http.post(&action2)
                        .form(&params2)
                        .send().await
                        .map_err(|e| format!("LTI POST(2)失敗: {}", e))?;
                    if resp2.status().is_redirection() {
                        resp2.headers().get("location")
                            .and_then(|l| l.to_str().ok())
                            .unwrap_or(&action2)
                            .to_string()
                    } else {
                        action2
                    }
                }
                Err(_) => action,
            }
        } else {
            return Err(format!("LTIリクエスト失敗: HTTP {}", status));
        };

        // Follow remaining redirects
        for _ in 0..5 {
            let resp = self.http.get(&current_url).send().await
                .map_err(|e| format!("LTIリダイレクト失敗: {}", e))?;
            if resp.status().is_redirection() {
                if let Some(loc) = resp.headers().get("location") {
                    current_url = loc.to_str().unwrap_or_default().to_string();
                    continue;
                }
            }
            break;
        }

        log::info!("LTI final URL: {}", current_url);
        Ok(current_url)
    }
}

/// Parse an LTI auto-submit form: returns (action_url, params)
fn parse_lti_form(html: &str) -> Result<(String, Vec<(String, String)>), String> {
    let doc = Html::parse_document(html);
    let form_sel = &*SEL_FORM;
    let input_sel = &*SEL_HIDDEN_INPUT;

    let form = doc.select(&form_sel).next()
        .ok_or("LTIフォームが見つかりません")?;

    let action = form.value().attr("action").unwrap_or_default().to_string();
    if action.is_empty() {
        return Err("LTIフォームのアクションURLが見つかりません".into());
    }

    let mut params: Vec<(String, String)> = Vec::new();
    for input in form.select(&input_sel) {
        let name = input.value().attr("name").unwrap_or_default().to_string();
        let value = input.value().attr("value").unwrap_or_default().to_string();
        if !name.is_empty() {
            params.push((name, value));
        }
    }

    Ok((action, params))
}
