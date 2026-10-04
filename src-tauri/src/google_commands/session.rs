use crate::google_calendar::{CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus};
use crate::GCalState;
use tauri::State;

#[tauri::command]
pub async fn gcal_check_session(state: State<'_, GCalState>) -> Result<GoogleCalStatus, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.status())
}

#[tauri::command]
pub async fn gcal_get_config(state: State<'_, GCalState>) -> Result<GoogleCalConfig, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.config.clone())
}

#[tauri::command]
pub async fn gcal_save_config(
    state: State<'_, GCalState>,
    config: GoogleCalConfig,
) -> Result<(), String> {
    let mut gcal = state.client.lock().await;
    // Empty fields mean "use built-in default" — persist the user's choice
    // (empty on disk) but keep the resolved defaults in memory so OAuth works
    // immediately without a restart.
    crate::google_calendar::save_config(&config)?;
    gcal.config = crate::google_calendar::resolve_with_defaults(config);
    Ok(())
}

#[tauri::command]
pub async fn gcal_disconnect(state: State<'_, GCalState>) -> Result<(), String> {
    let mut gcal = state.client.lock().await;
    gcal.disconnect();
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
