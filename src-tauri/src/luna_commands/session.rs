use tauri::State;

use crate::luna_parser;
use crate::LunaState;

use super::{luna_fetch_cached, luna_get, luna_http};

/// Fetch a Luna page (generic)
#[tauri::command]
pub async fn luna_fetch_page(state: State<'_, LunaState>, path: String) -> Result<String, String> {
    // Only allow known Luna paths
    if path.contains("://") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let allowed_prefixes = [
        "/top",
        "/lms/",
        "/course/",
        "/notification",
        "/updateinfo",
        "/message",
        "/attend",
        "/report",
        "/survey",
        "/material",
    ];
    if !allowed_prefixes.iter().any(|p| path.starts_with(p)) {
        return Err("許可されていないパスです".into());
    }
    let http = luna_http(&state).await?;
    luna_get(&http, &path).await
}

/// Check if Luna session is valid
#[tauri::command]
pub async fn luna_check_session(state: State<'_, LunaState>) -> Result<bool, String> {
    let (http, authenticated) = {
        let luna = state.client.lock().await;
        (luna.http.clone(), luna.authenticated)
    };
    if !authenticated {
        return Ok(false);
    }
    // Validate against server without holding the lock
    let url = format!("{}/lms/timetable", crate::config::LUNA_BASE);
    match crate::client::fetch_with_redirect(
        &http,
        &url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    {
        Ok(_) => {
            let luna = state.client.lock().await;
            luna.save_session();
            Ok(true)
        }
        Err(e) if e == crate::luna_client::LUNA_SESSION_EXPIRED_MSG => {
            let mut luna = state.client.lock().await;
            luna.authenticated = false;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Fetch parsed TODO list
#[tauri::command]
pub async fn luna_fetch_todo(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaTodoItem>, String> {
    luna_fetch_cached(
        &state,
        &db,
        "/lms/todo",
        "luna_todo",
        luna_parser::parse_luna_todo,
    )
    .await
}

/// Fetch parsed notifications
#[tauri::command]
pub async fn luna_fetch_updates(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaNotification>, String> {
    luna_fetch_cached(
        &state,
        &db,
        "/updateinfo",
        "luna_updates",
        luna_parser::parse_luna_notifications,
    )
    .await
}
