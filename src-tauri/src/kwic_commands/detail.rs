//! KWIC notification detail fetch.

use super::*;
use tauri::State;

use crate::KwicState;

/// Agent-accessible variant of `kwic_fetch_detail`. Resolves state from the
/// AppHandle and skips the cache-on-error fallback so the agent always sees
/// the most accurate result (or a real error message).
pub async fn kwic_fetch_detail_internal(
    app: &tauri::AppHandle,
    information_id: &str,
    information_type: &str,
    person_category_cd: &str,
    category_cd: &str,
) -> Result<KwicNotificationDetail, String> {
    use tauri::Manager;
    let state = app.state::<KwicState>();
    let http = kwic_http(&state).await?;
    let home_html = kwic_get(&http, "/portal/home").await?;
    let csrf = extract_csrf_token(&home_html)
        .ok_or_else(|| "CSRFトークンが取得できませんでした".to_string())?;
    let detail_html = kwic_post(
        &http,
        "/portal/home/information/detail",
        &[
            ("_csrf", &csrf),
            ("informationId", information_id),
            ("informationType", information_type),
            ("personCategoryCd", person_category_cd),
            ("categoryCd", category_cd),
            ("selectCategoryCd", category_cd),
            ("pageViewListNum", "10"),
        ],
    )
    .await?;
    Ok(parse_detail_html(&detail_html))
}

/// Fetch and parse a KWIC Portal notification detail inline (no webview).
/// The detail page is fetched via POST to /portal/home/information/detail
/// using the same form parameters as the portal's #PortalinformationDtl form.
#[tauri::command]
pub async fn kwic_fetch_detail(
    state: State<'_, KwicState>,
    db: crate::db::AccountDb,
    information_id: String,
    information_type: String,
    person_category_cd: String,
    category_cd: String,
) -> Result<KwicNotificationDetail, String> {
    let cache_key = format!("kwic_detail:{}", information_id);
    match kwic_http(&state).await {
        Ok(http) => {
            // 1. Get home page to extract CSRF token
            let home_html = match kwic_get(&http, "/portal/home").await {
                Ok(h) => h,
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback ({})", cache_key, e);
                            return Ok(cached);
                        }
                    }
                    return Err(e);
                }
            };
            let csrf = match extract_csrf_token(&home_html) {
                Some(token) => token,
                None => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback (CSRF extraction failed)", cache_key);
                            return Ok(cached);
                        }
                    }
                    return Err("CSRFトークンが取得できませんでした".to_string());
                }
            };

            // 2. POST to portal detail endpoint
            match kwic_post(
                &http,
                "/portal/home/information/detail",
                &[
                    ("_csrf", &csrf),
                    ("informationId", &information_id),
                    ("informationType", &information_type),
                    ("personCategoryCd", &person_category_cd),
                    ("categoryCd", &category_cd),
                    ("selectCategoryCd", &category_cd),
                    ("pageViewListNum", "10"),
                ],
            )
            .await
            {
                Ok(detail_html) => {
                    #[cfg(debug_assertions)]
                    {
                        if crate::should_dump_debug_html() {
                            let _ = std::fs::write(
                                std::env::temp_dir().join("kwic-portal-detail.html"),
                                &detail_html,
                            );
                        }
                    }

                    let data = parse_detail_html(&detail_html);
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback ({})", cache_key, e);
                            return Ok(cached);
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// One-shot shrink of persisted KWIC notice bodies that embedded multi-megabyte images.
pub fn compact_cached_kwic_details(db: &crate::db::Database) -> usize {
    let Ok(rows) = db.oversized_cache_rows("kwic_detail:", 100_000) else {
        return 0;
    };
    let mut compacted = 0;
    for (key, json) in rows {
        let Ok(mut detail) = serde_json::from_str::<KwicNotificationDetail>(&json) else {
            continue;
        };
        let next = compact_inline_images(&detail.body_html);
        if next.len() >= detail.body_html.len() {
            continue;
        }
        detail.body_html = next.into_owned();
        let Ok(stored) = serde_json::to_string(&detail) else {
            continue;
        };
        if db
            .save_data_cache_if_changed(&key, &stored)
            .unwrap_or(false)
        {
            compacted += 1;
        }
    }
    if compacted > 0 {
        db.checkpoint_passive();
    }
    compacted
}
