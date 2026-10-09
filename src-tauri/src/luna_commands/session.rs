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
pub(crate) async fn luna_check_session(
    _state: State<'_, LunaState>,
) -> Result<crate::session_coordinator::ServiceStatus, crate::session_coordinator::SessionError> {
    let service = crate::session_coordinator::Service::Luna;
    crate::session_coordinator::verify(service).await?;
    Ok(crate::session_coordinator::SESSIONS.snapshot().services[service.index()].clone())
}

/// Fetch parsed TODO list
#[tauri::command]
pub async fn luna_fetch_todo(
    state: State<'_, LunaState>,
    db: crate::db::AccountDb,
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
    db: crate::db::AccountDb,
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
