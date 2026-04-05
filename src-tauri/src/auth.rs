use reqwest::Client;
use serde::{Deserialize, Serialize};
use regex::Regex;
use url::Url;

use crate::client::KwicClient;
use crate::parser;

const BASE_URL: &str = "https://kg-course.kwansei.ac.jp";

/// Generate a SAML intercept initialization script for a login webview.
/// `callback_host` determines the fake hostname used to intercept the SAML response
/// (e.g. "kwic-saml-callback.localhost" or "luna-saml-callback.localhost").
pub fn saml_intercept_script(callback_host: &str) -> String {
    format!(
        r#"
(function() {{
    const origSubmit = HTMLFormElement.prototype.submit;
    HTMLFormElement.prototype.submit = function() {{
        const saml = this.querySelector('input[name="SAMLResponse"]');
        if (saml) {{
            const relay = this.querySelector('input[name="RelayState"]');
            const params = new URLSearchParams();
            params.set('saml_response', saml.value);
            params.set('relay_state', relay ? relay.value : '');
            params.set('acs_url', this.action);
            window.location.href = 'http://{host}/callback?' + params.toString();
            return;
        }}
        origSubmit.call(this);
    }};
    document.addEventListener('submit', function(e) {{
        const form = e.target;
        if (!(form instanceof HTMLFormElement)) return;
        const saml = form.querySelector('input[name="SAMLResponse"]');
        if (saml) {{
            e.preventDefault();
            e.stopPropagation();
            const relay = form.querySelector('input[name="RelayState"]');
            const params = new URLSearchParams();
            params.set('saml_response', saml.value);
            params.set('relay_state', relay ? relay.value : '');
            params.set('acs_url', form.action);
            window.location.href = 'http://{host}/callback?' + params.toString();
        }}
    }}, true);
}})();
"#,
        host = callback_host,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub username: String,
    pub display_name: String,
    pub student_id: String,
    pub faculty: String,
    pub department: String,
}

/// Data intercepted from the SAML auto-submit form in the login webview
#[derive(Debug, Clone)]
pub struct SamlCallbackData {
    pub saml_response: String,
    pub relay_state: String,
    pub acs_url: String,
}

/// Step 1: Initiate SP-side auth with reqwest to get the Okta SAML SSO URL.
/// This also establishes SP session cookies in the reqwest cookie jar,
/// which are needed when we later POST the SAMLResponse back to the SP.
pub async fn initiate_sp_auth(http: &Client) -> Result<String, String> {
    let entry_url = format!("{}/uniasv2/UnSSOLoginControl2", BASE_URL);
    let mut current_url = entry_url;

    for i in 0..10 {
        log::info!("initiate_sp_auth step {}: GET {}", i, &current_url[..120.min(current_url.len())]);
        let resp = http
            .get(&current_url)
            .send()
            .await
            .map_err(|e| format!("SP接続失敗: {}", e))?;

        let status = resp.status();
        let headers = resp.headers().clone();
        log::info!("initiate_sp_auth step {}: status={}", i, status);

        if status.is_redirection() {
            if let Some(loc) = headers.get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                log::info!("initiate_sp_auth step {}: Location={}", i, &loc_str[..200.min(loc_str.len())]);
                let next_url = if loc_str.starts_with('/') {
                    let parsed = Url::parse(&current_url).unwrap();
                    format!("{}://{}{}", parsed.scheme(), parsed.host_str().unwrap(), loc_str)
                } else {
                    loc_str.to_string()
                };

                // Capture the full Okta SAML URL before visiting Okta
                if next_url.contains("sso.kwansei.ac.jp") && next_url.contains("/sso/saml") {
                    log::info!("Captured Okta SAML URL: {}", &next_url[..120.min(next_url.len())]);
                    return Ok(next_url);
                }

                current_url = next_url;
                continue;
            }
        }

        // Check for meta refresh redirect
        let body = resp.text().await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        let re = Regex::new(r#"meta\s+http-equiv="refresh"\s+content="\d+;URL=([^"]+)""#).unwrap();
        if let Some(caps) = re.captures(&body) {
            let redirect_path = &caps[1];
            let parsed = Url::parse(&current_url).unwrap();
            current_url = if redirect_path.starts_with("http") {
                redirect_path.to_string()
            } else {
                format!("{}://{}/{}", parsed.scheme(), parsed.host_str().unwrap(), redirect_path)
            };
            continue;
        }

        // Check if body contains an Okta SAML URL
        if body.contains("sso.kwansei.ac.jp") {
            let re_saml = Regex::new(r#"(https://sso\.kwansei\.ac\.jp/app/[^"'\s]+)"#).unwrap();
            if let Some(caps) = re_saml.captures(&body) {
                return Ok(caps[1].to_string());
            }
        }

        return Err(format!("SAMLリダイレクトを見つけられませんでした (Status: {})", status));
    }

    Err("リダイレクトループが発生しました".into())
}

/// Step 2: After the webview intercepts the SAMLResponse form,
/// submit it to the SP's ACS endpoint via reqwest to establish
/// the Shibboleth session in our HTTP client.
pub async fn complete_saml_login(
    kwic: &mut KwicClient,
    data: &SamlCallbackData,
) -> Result<AuthSession, String> {
    log::info!("Submitting SAMLResponse to ACS: {}", &data.acs_url[..80.min(data.acs_url.len())]);
    log::info!("SAMLResponse length: {}, RelayState length: {}", data.saml_response.len(), data.relay_state.len());

    // POST SAMLResponse to the SP's ACS URL using the same cookie jar
    // that was used in initiate_sp_auth (contains SP session cookies)
    let mut params = vec![("SAMLResponse", data.saml_response.as_str())];
    if !data.relay_state.is_empty() {
        params.push(("RelayState", data.relay_state.as_str()));
    }

    let resp = kwic.http
        .post(&data.acs_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("ACS POST失敗: {}", e))?;

    let status = resp.status();
    log::info!("ACS response status: {}", status);

    if !status.is_redirection() && !status.is_success() {
        return Err(format!("ACSエンドポイントがエラーを返しました: {}", status));
    }

    // Follow post-login redirects to fully establish the session
    let mut current_url = format!("{}/uniasv2/UnSSOLoginControl2", BASE_URL);
    for _ in 0..15 {
        let resp = kwic.http
            .get(&current_url)
            .send()
            .await
            .map_err(|e| format!("リダイレクト追跡失敗: {}", e))?;

        let st = resp.status();
        if st.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                let next = if loc_str.starts_with('/') {
                    let parsed = Url::parse(&current_url).unwrap();
                    format!("{}://{}{}", parsed.scheme(), parsed.host_str().unwrap(), loc_str)
                } else {
                    loc_str.to_string()
                };
                if next.contains("sso.kwansei.ac.jp") {
                    break;
                }
                current_url = next;
                continue;
            }
        }
        break;
    }

    // Temporarily mark as authenticated so fetch_page works
    kwic.session = Some(AuthSession {
        username: String::new(),
        display_name: "ユーザー".to_string(),
        student_id: String::new(),
        faculty: String::new(),
        department: String::new(),
    });

    // Fetch timetable page to parse basic student info
    let student_info = match kwic.fetch_page("/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014").await {
        Ok(html) => {
            let info = parser::parse_student_info(&html);
            log::info!("Student info: id={}, name={}, faculty={}", info.student_id, info.name, info.faculty);
            info
        }
        Err(e) => {
            log::warn!("Failed to fetch student info: {}", e);
            parser::StudentInfo::default()
        }
    };

    let session = AuthSession {
        username: student_info.student_id.clone(),
        display_name: if student_info.name.is_empty() { "ユーザー".to_string() } else { student_info.name },
        student_id: student_info.student_id,
        faculty: student_info.faculty,
        department: student_info.department,
    };
    kwic.session = Some(session.clone());

    // Persist session and cookies to disk
    kwic.save_session();

    log::info!("Login complete, session established for {}", session.display_name);
    Ok(session)
}
