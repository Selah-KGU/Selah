use crate::client;
use crate::config;
use crate::luna_client;
use crate::LunaState;

/// Briefly lock Luna client, check auth and clone http. Releases lock immediately.
pub(super) async fn luna_http(state: &LunaState) -> Result<reqwest::Client, String> {
    if crate::session_coordinator::SESSIONS.signed_out() {
        return Err(crate::session_coordinator::CANCELLED.into());
    }

    let luna = state.session();
    if !luna.has_credentials() {
        return Err(luna_client::LUNA_AUTH_REQUIRED_MSG.into());
    }
    Ok(luna.http().clone())
}

/// Luna GET: fetch a page without holding the lock.
pub(super) async fn luna_get(http: &reqwest::Client, path: &str) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    client::fetch_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
}

/// Luna GET with Referer header — required for form pages that serve CSRF tokens.
pub(super) async fn luna_get_with_referer(
    http: &reqwest::Client,
    path: &str,
    referer_path: &str,
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let referer = format!("{}{}", config::LUNA_BASE, referer_path);
    let mut current_url = url;
    for i in 0..10 {
        let resp = http
            .get(&current_url)
            .header("Referer", &referer)
            .send()
            .await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;
        let status = resp.status();
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                current_url = if loc_str.starts_with('/') {
                    format!("{}{}", config::LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                log::debug!(
                    "luna_get_with_referer redirect #{} -> {}",
                    i + 1,
                    client::safe_truncate(&current_url, 120)
                );
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
                }
                continue;
            }
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }
        let body = resp
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        if luna_client::is_luna_session_expired(&body) {
            return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
        }
        return Ok(body);
    }
    Err("リダイレクトが多すぎます".into())
}

/// Luna POST: submit a form without holding the lock.
pub(super) async fn luna_post(
    http: &reqwest::Client,
    path: &str,
    params: &[(String, String)],
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    client::post_form_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[],
    )
    .await
}

/// Luna POST with a page Referer. Some Luna form endpoints reject otherwise-valid
/// CSRF submissions when the request does not come from the form page.
pub(super) async fn luna_post_with_referer(
    http: &reqwest::Client,
    path: &str,
    referer_path: &str,
    params: &[(String, String)],
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let referer = format!("{}{}", config::LUNA_BASE, referer_path);
    let headers = [("Referer", referer.as_str())];
    client::post_form_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &headers,
    )
    .await
}

/// Luna multipart POST: submit a multipart form without holding the lock.
pub(super) async fn luna_post_multipart(
    http: &reqwest::Client,
    path: &str,
    form: reqwest::multipart::Form,
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let builder = http.post(&url).multipart(form);
    client::send_and_follow_redirect(
        http,
        builder,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
}

/// Luna multipart POST with _cid appended to URL (mimics Luna's AJAX interceptor).
pub(super) async fn luna_post_multipart_with_cid(
    http: &reqwest::Client,
    path: &str,
    cid: &str,
    form: reqwest::multipart::Form,
) -> Result<String, String> {
    let url = format!("{}{}?_cid={}", config::LUNA_BASE, path, cid);
    let builder = http.post(&url).multipart(form);
    client::send_and_follow_redirect(
        http,
        builder,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
}

pub(super) async fn luna_post_multipart_with_optional_cid(
    http: &reqwest::Client,
    path: &str,
    cid: Option<&str>,
    form: reqwest::multipart::Form,
) -> Result<String, String> {
    if let Some(cid) = cid.filter(|s| !s.is_empty()) {
        luna_post_multipart_with_cid(http, path, cid, form).await
    } else {
        luna_post_multipart(http, path, form).await
    }
}

pub(super) fn add_text_fields(
    mut form: reqwest::multipart::Form,
    fields: &[(String, String)],
) -> reqwest::multipart::Form {
    for (key, value) in fields {
        form = form.text(key.clone(), value.clone());
    }
    form
}
