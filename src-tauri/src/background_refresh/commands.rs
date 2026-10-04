//! Tauri commands for on-demand backend refresh.

use super::*;

#[tauri::command]
pub async fn backend_refresh_now(
    app: AppHandle,
    keys: Option<Vec<String>>,
    force: Option<bool>,
) -> Result<Vec<String>, String> {
    refresh_backend_now(&app, keys.as_deref(), force.unwrap_or(false)).await
}

#[tauri::command]
pub async fn backend_sync_session_status_now(
    app: AppHandle,
) -> Result<BackendSessionStatusPayload, String> {
    sync_backend_session_status(&app, true).await
}
