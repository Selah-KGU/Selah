//! Cached and fresh Luna detail HTML fetches.

use super::*;

pub async fn fetch_luna_detail_html_cached(
    app: &tauri::AppHandle,
    detail_path: &str,
) -> Result<String, String> {
    fetch_luna_detail_html_inner(app, detail_path)
        .await
        .map(|(html, _age)| html)
}

/// Returns (html, cache_age_secs). cache_age_secs = 0 means it was just fetched from network.
pub async fn fetch_luna_detail_html_with_age(
    app: &tauri::AppHandle,
    detail_path: &str,
) -> Result<(String, i64), String> {
    fetch_luna_detail_html_inner(app, detail_path).await
}

pub async fn fetch_luna_detail_html_fresh(
    app: &tauri::AppHandle,
    detail_path: &str,
) -> Result<String, String> {
    let db = app.state::<Database>().scope();
    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.session();
        if !luna.has_credentials() {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http().clone()
    };
    let url = format!("{}{}", crate::config::LUNA_BASE, detail_path);
    let html = crate::client::fetch_with_redirect(
        &http,
        &url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    .map_err(|error| format!("Luna取得失敗: {}", error))?;
    let _ = db.save_data_cache(&format!("luna_detail_html:{}", detail_path), &html);
    Ok(html)
}

async fn fetch_luna_detail_html_inner(
    app: &tauri::AppHandle,
    detail_path: &str,
) -> Result<(String, i64), String> {
    let db = app.state::<Database>().scope();
    let cache_key = format!("luna_detail_html:{}", detail_path);

    // Check SQLite cache first
    if let Ok(Some((html, updated_at))) = db.get_data_cache(&cache_key) {
        let now = crate::db::epoch_secs();
        let age = now - updated_at;
        // If the cache is within 7 days, let's use it directly to save API hits & allow offline resolve
        if age < 7 * 24 * 3600 {
            log::debug!("Cache hit for Luna details of {}", detail_path);
            return Ok((html, age));
        }

        // Try requesting online, but if session expired/offline, fallback to the expired cache instead of breaking!
        let luna_state = app.state::<crate::LunaState>();
        let http_opt = {
            let luna = luna_state.session();
            if luna.has_credentials() {
                Some(luna.http().clone())
            } else {
                None
            }
        };

        if let Some(http) = http_opt {
            let url = format!("{}{}", crate::config::LUNA_BASE, detail_path);
            if let Ok(fresh_html) = crate::client::fetch_with_redirect(
                &http,
                &url,
                crate::config::LUNA_BASE,
                crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
                crate::luna_client::is_luna_session_expired,
            )
            .await
            {
                let _ = db.save_data_cache(&cache_key, &fresh_html);
                return Ok((fresh_html, 0));
            }
        }
        log::warn!(
            "Failed online fetch for {}, falling back to expired cached HTML",
            detail_path
        );
        return Ok((html, age));
    }

    // Cache miss, must resolve online
    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.session();
        if !luna.has_credentials() {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http().clone()
    };

    let url = format!("{}{}", crate::config::LUNA_BASE, detail_path);
    let html = crate::client::fetch_with_redirect(
        &http,
        &url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    .map_err(|e| format!("Luna取得失敗: {}", e))?;

    let _ = db.save_data_cache(&cache_key, &html);
    Ok((html, 0))
}
