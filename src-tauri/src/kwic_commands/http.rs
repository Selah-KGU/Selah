//! KWIC HTTP helpers. The client lock is released before each request.

use crate::client;
use crate::config;
use crate::kwic_client;
use crate::KwicState;

/// Briefly lock KWIC client, check auth and clone http. Releases lock immediately.
pub(in crate::kwic_commands) async fn kwic_http(
    state: &KwicState,
) -> Result<reqwest::Client, String> {
    if crate::session_coordinator::SESSIONS.signed_out() {
        return Err(crate::session_coordinator::CANCELLED.into());
    }

    let kwic = state.session();
    if !kwic.has_credentials() {
        return Err(kwic_client::KWIC_AUTH_REQUIRED_MSG.into());
    }
    Ok(kwic.http().clone())
}

/// KWIC GET: fetch a page without holding the lock.
pub(in crate::kwic_commands) async fn kwic_get(
    http: &reqwest::Client,
    path: &str,
) -> Result<String, String> {
    let url = format!("{}{}", config::KWIC_BASE, path);
    client::fetch_with_redirect(
        http,
        &url,
        config::KWIC_BASE,
        kwic_client::KWIC_SESSION_EXPIRED_MSG,
        kwic_client::is_kwic_session_expired,
    )
    .await
}

/// KWIC POST: submit a form without holding the lock.
pub(in crate::kwic_commands) async fn kwic_post(
    http: &reqwest::Client,
    path: &str,
    params: &[(&str, &str)],
) -> Result<String, String> {
    let url = format!("{}{}", config::KWIC_BASE, path);
    client::post_form_with_redirect(
        http,
        &url,
        config::KWIC_BASE,
        kwic_client::KWIC_SESSION_EXPIRED_MSG,
        kwic_client::is_kwic_session_expired,
        params.iter().copied(),
        &[],
    )
    .await
}
