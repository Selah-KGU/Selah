use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::{Arc, LazyLock};

use crate::client::{
    build_http_client, data_dir, fetch_with_redirect, load_cookie_jar, new_cookie_client,
    save_cookie_jar,
};
use crate::config;

static SEL_FORM: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("form").expect("valid selector"));
static SEL_HIDDEN_INPUT: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"input[type="hidden"]"#).expect("valid selector"));

const LUNA_COOKIES_FILE: &str = "luna_cookies.json";

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
        if !self.authenticated {
            log::warn!("Luna save_session skipped: not authenticated");
            return;
        }
        save_cookie_jar(&self.cookie_store, LUNA_COOKIES_FILE);
        log::info!("Luna cookies saved");
    }

    /// Try to restore Luna session from disk.
    /// Returns true if cookies were loaded (session still needs server validation).
    pub fn try_restore_session(&mut self) -> bool {
        match load_cookie_jar(LUNA_COOKIES_FILE) {
            Some(store) => {
                let cookie_store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(store));
                self.http = build_http_client(cookie_store.clone());
                self.cookie_store = cookie_store;
                self.authenticated = true;
                log::info!("Luna session restored from disk");
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.authenticated = false;
        if let Err(e) = std::fs::remove_file(data_dir().join(LUNA_COOKIES_FILE)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Luna clear: failed to delete cookies file: {}", e);
            }
        }
        let (cookie_store, http) = new_cookie_client();
        self.http = http;
        self.cookie_store = cookie_store;
    }
}

/// Launch an LTI tool (Zoom, Panopto, etc.) without holding the mutex.
/// Fetches the LTI launch page, extracts the auto-submit form,
/// POSTs to the third-party platform and returns the final URL.
pub async fn launch_lti(http: &Client, path: &str) -> Result<String, String> {
    // Step 1: GET the LTI launch page from Luna
    let url = format!("{}{}", config::LUNA_BASE, path);
    let html = fetch_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        LUNA_SESSION_EXPIRED_MSG,
        is_luna_session_expired,
    )
    .await?;

    // Step 2: Parse auto-submit form (sync — keep scraper types off await points)
    let (action, params) = parse_lti_form(&html)?;

    log::info!(
        "LTI launch: POST to {} with {} params",
        action,
        params.len()
    );

    // Step 3: POST form to third-party platform
    let resp = http
        .post(&action)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("LTI POST失敗: {}", e))?;

    let status = resp.status();
    let mut current_url = if status.is_redirection() {
        resp.headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
            .unwrap_or(&action)
            .to_string()
    } else if status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // Some LTI flows return another auto-submit form
        match parse_lti_form(&body) {
            Ok((action2, params2)) => {
                let resp2 = http
                    .post(&action2)
                    .form(&params2)
                    .send()
                    .await
                    .map_err(|e| format!("LTI POST(2)失敗: {}", e))?;
                if resp2.status().is_redirection() {
                    resp2
                        .headers()
                        .get("location")
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
        let resp = http
            .get(&current_url)
            .send()
            .await
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

/// Parse an LTI auto-submit form: returns (action_url, params)
fn parse_lti_form(html: &str) -> Result<(String, Vec<(String, String)>), String> {
    let doc = Html::parse_document(html);
    let form_sel = &*SEL_FORM;
    let input_sel = &*SEL_HIDDEN_INPUT;

    let form = doc
        .select(form_sel)
        .next()
        .ok_or("LTIフォームが見つかりません")?;

    let action = form.value().attr("action").unwrap_or_default().to_string();
    if action.is_empty() {
        return Err("LTIフォームのアクションURLが見つかりません".into());
    }

    let mut params: Vec<(String, String)> = Vec::new();
    for input in form.select(input_sel) {
        let name = input.value().attr("name").unwrap_or_default().to_string();
        let value = input.value().attr("value").unwrap_or_default().to_string();
        if !name.is_empty() {
            params.push((name, value));
        }
    }

    Ok((action, params))
}
