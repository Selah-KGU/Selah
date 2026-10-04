//! KWIC portal session check and home fetch.

use super::*;
use tauri::State;

use crate::KwicState;

/// Check KWIC Portal session
#[tauri::command]
pub async fn kwic_check_session(state: State<'_, KwicState>) -> Result<bool, String> {
    let (http, authenticated) = {
        let kwic = state.client.lock().await;
        (kwic.http.clone(), kwic.authenticated)
    };
    if !authenticated {
        return Ok(false);
    }
    // Validate against server without holding the lock
    let url = format!("{}/portal/home", crate::config::KWIC_BASE);
    match crate::client::fetch_with_redirect(
        &http,
        &url,
        crate::config::KWIC_BASE,
        crate::kwic_client::KWIC_SESSION_EXPIRED_MSG,
        crate::kwic_client::is_kwic_session_expired,
    )
    .await
    {
        Ok(_) => {
            let kwic = state.client.lock().await;
            kwic.save_session();
            Ok(true)
        }
        Err(e) if e == crate::kwic_client::KWIC_SESSION_EXPIRED_MSG => {
            let mut kwic = state.client.lock().await;
            kwic.authenticated = false;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Fetch and parse the KWIC Portal home page
#[tauri::command]
pub async fn kwic_fetch_home(
    state: State<'_, KwicState>,
    db: State<'_, crate::db::Database>,
) -> Result<KwicPortalHome, String> {
    match kwic_http(&state).await {
        Ok(http) => match kwic_get(&http, "/portal/home").await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let _ = std::fs::write(
                            std::env::temp_dir().join("kwic-portal-home.html"),
                            &html,
                        );
                    }
                }

                let information_list_pages = [
                    ("/portal/home/information/list", "10"),
                    (
                        "/portal/home/information/list?informationType=12&categoryCd=0",
                        "12",
                    ),
                ];
                let mut list_pages = Vec::with_capacity(information_list_pages.len());
                let mut lists_complete = true;
                for (path, fallback_information_type) in information_list_pages {
                    match kwic_get(&http, path).await {
                        Ok(list_html) => list_pages.push((fallback_information_type, list_html)),
                        Err(e) => {
                            lists_complete = false;
                            log::info!(
                                "kwic_home: information list {} fetch skipped ({})",
                                fallback_information_type,
                                e
                            );
                        }
                    }
                }

                if lists_complete {
                    let mut parts = Vec::with_capacity(1 + list_pages.len());
                    parts.push(html.as_str());
                    for (_, list_html) in &list_pages {
                        parts.push(list_html.as_str());
                    }
                    let hash = crate::db::source_hash(&parts);
                    if db.cached_source_matches("kwic_home", &hash) {
                        if let Ok(Some((json, _))) = db.get_data_cache("kwic_home") {
                            if let Ok(cached) = serde_json::from_str(&json) {
                                let _ = db.touch_data_cache("kwic_home");
                                log::debug!("kwic_home: unchanged, skipped parse");
                                return Ok(cached);
                            }
                        }
                    }
                    let result = assemble_kwic_home(&html, &list_pages);
                    if let Ok(json) = serde_json::to_string(&result) {
                        let _ = db.save_data_cache("kwic_home", &json);
                    }
                    db.store_source_hash("kwic_home", &hash);
                    return Ok(result);
                }

                let result = assemble_kwic_home(&html, &list_pages);
                if let Ok(json) = serde_json::to_string(&result) {
                    let _ = db.save_data_cache("kwic_home", &json);
                }
                Ok(result)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache("kwic_home") {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("kwic_home: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache("kwic_home") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("kwic_home: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

fn assemble_kwic_home(html: &str, list_pages: &[(&str, String)]) -> KwicPortalHome {
    let mut sections = parse_portal_home(html);
    for (fallback_information_type, list_html) in list_pages {
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump_name = format!(
                    "kwic-portal-information-list-{}.html",
                    fallback_information_type
                );
                let _ = std::fs::write(std::env::temp_dir().join(dump_name), list_html);
            }
        }
        let (parsed, added) = merge_information_list_sections(
            &mut sections,
            list_html,
            Some(fallback_information_type),
        );
        log::info!(
            "kwic_home: information list {} parsed {} item(s), added {} item(s)",
            fallback_information_type,
            parsed,
            added
        );
    }
    KwicPortalHome {
        sections,
        #[cfg(debug_assertions)]
        raw_html_debug: Some(crate::client::safe_truncate(html, 5000).to_string()),
        #[cfg(not(debug_assertions))]
        raw_html_debug: None,
    }
}
