//! KWIC subportal and cabinet fetches.

use super::*;
use tauri::State;

use crate::KwicState;

/// Fetch and parse a KWIC Portal subportal page (e.g. /portal/subportal?tagCd=1)
#[tauri::command]
pub async fn kwic_fetch_subportal(
    state: State<'_, KwicState>,
    db: crate::db::AccountDb,
    tag_cd: String,
) -> Result<KwicSubportalData, String> {
    if !tag_cd.chars().all(|c| c.is_ascii_digit()) {
        return Err("\u{7121}\u{52b9}\u{306a}tagCd\u{3067}\u{3059}".into());
    }
    let cache_key = format!("kwic_subportal:{}", tag_cd);
    match kwic_http(&state).await {
        Ok(http) => {
            let path = format!("/portal/subportal?tagCd={}", tag_cd);
            match kwic_get(&http, &path).await {
                Ok(html) => {
                    #[cfg(debug_assertions)]
                    {
                        if crate::should_dump_debug_html() {
                            let _ = std::fs::write(
                                std::env::temp_dir()
                                    .join(format!("kwic-portal-subportal-{}.html", tag_cd)),
                                &html,
                            );
                        }
                    }

                    let data = parse_subportal(&html);
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

/// Fetch and parse the KWIC student cabinet reference page.
#[tauri::command]
pub async fn kwic_fetch_cabinet_reference(
    state: State<'_, KwicState>,
    db: crate::db::AccountDb,
) -> Result<KwicCabinetReference, String> {
    let cache_key = "kwic_cabinet_reference";
    match kwic_http(&state).await {
        Ok(http) => match kwic_get(&http, "/cabinet/reference").await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let _ = std::fs::write(
                            std::env::temp_dir().join("kwic-cabinet-reference.html"),
                            &html,
                        );
                    }
                }

                #[cfg(debug_assertions)]
                let data = {
                    let mut data = parse_cabinet_reference(&html);
                    data.raw_html_debug =
                        Some(crate::client::safe_truncate(&html, 5000).to_string());
                    data
                };
                #[cfg(not(debug_assertions))]
                let data = parse_cabinet_reference(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("{}: cache fallback ({})", cache_key, e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}
