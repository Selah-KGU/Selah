use tauri::State;

use crate::LunaState;

use super::{luna_get, luna_http};

/// Fetch a Luna page, parse it, and cache with fallback.
pub(super) async fn luna_fetch_cached<T: serde::Serialize + serde::de::DeserializeOwned>(
    state: &State<'_, LunaState>,
    db: &crate::db::AccountDb,
    path: &str,
    cache_key: &str,
    parse: fn(&str) -> T,
) -> Result<T, String> {
    let try_cache = |e: String| -> Result<T, String> {
        if let Ok(Some((json, _))) = db.get_data_cache(cache_key) {
            if let Ok(cached) = serde_json::from_str(&json) {
                log::info!("{}: cache fallback ({})", cache_key, e);
                return Ok(cached);
            }
        }
        Err(e)
    };
    let http = match luna_http(state).await {
        Ok(h) => h,
        Err(e) => return try_cache(e),
    };
    match luna_get(&http, path).await {
        Ok(html) => {
            let hash = crate::db::source_hash(&[&html]);
            if db.cached_source_matches(cache_key, &hash) {
                if let Ok(Some((json, _))) = db.get_data_cache(cache_key) {
                    if let Ok(cached) = serde_json::from_str::<T>(&json) {
                        let _ = db.touch_data_cache(cache_key);
                        log::debug!("{cache_key}: unchanged, skipped parse");
                        return Ok(cached);
                    }
                }
            }
            let data = parse(&html);
            if let Ok(json) = serde_json::to_string(&data) {
                let changed = db.store_data_cache(cache_key, &json, true).unwrap_or(true);
                db.store_source_hash(cache_key, &hash);
                if changed && cache_key == "luna_todo" {
                    crate::widget_bridge::publish(db.inner());
                }
            }
            Ok(data)
        }
        Err(e) => try_cache(e),
    }
}

/// Escape HTML special characters to prevent XSS in server-side rendered content
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Validate that a string looks like a simple numeric/alphanumeric ID
pub(super) fn is_safe_param(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 20
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
