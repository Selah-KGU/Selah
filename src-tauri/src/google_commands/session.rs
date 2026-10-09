use crate::google_calendar::{CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus};
use crate::GCalState;
use tauri::State;

#[tauri::command]
pub async fn gcal_check_session(state: State<'_, GCalState>) -> Result<GoogleCalStatus, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.status())
}

#[tauri::command]
pub async fn gcal_get_config(
    state: State<'_, GCalState>,
) -> Result<GoogleCalConfig, crate::keychain::StoreError> {
    let gcal = state.client.lock().await;
    gcal.ensure_config()?;
    Ok(gcal.config.clone())
}

#[tauri::command]
pub async fn gcal_save_config(
    state: State<'_, GCalState>,
    config: GoogleCalConfig,
) -> Result<(), String> {
    state.cancellation.cancel();
    let mut gcal = state.client.lock().await;
    gcal.cancel_requests();
    let next = crate::google_calendar::resolve_with_defaults(config.clone());
    // Retire credentials before committing a different OAuth client. A failed
    // save may leave the old configuration, but never a new client with old tokens.
    if next.client_id != gcal.config.client_id || next.client_secret != gcal.config.client_secret {
        gcal.disconnect()?;
    }
    crate::google_calendar::save_config(&config)?;
    gcal.config = next;
    gcal.config_error = None;
    Ok(())
}

#[tauri::command]
pub async fn gcal_disconnect(state: State<'_, GCalState>) -> Result<(), String> {
    state.cancellation.cancel();
    let mut gcal = state.client.lock().await;
    gcal.disconnect()?;
    log::info!("Google Calendar disconnected");
    Ok(())
}

/// Sync this week's timetable to Google Calendar
#[tauri::command]
pub async fn gcal_sync_timetable(
    state: State<'_, GCalState>,
    entries: Vec<CalendarSyncEntry>,
    week_label: String,
) -> Result<String, String> {
    let mut gcal = state.client.lock().await;
    gcal.sync_timetable(entries, week_label).await
}

#[tauri::command]
pub async fn gcal_clear_calendar(
    state: State<'_, GCalState>,
    delete_calendar: bool,
) -> Result<String, String> {
    let mut gcal = state.client.lock().await;
    gcal.clear_calendar(delete_calendar).await
}
