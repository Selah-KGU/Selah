use tauri::Emitter;
use tokio::sync::Mutex;

use super::google_calendar;
use super::mail;

#[path = "app_state/cache.rs"]
mod cache;
pub(crate) use cache::{admission_handler, drain_cache, reopen_cache, seal_cache};

// ── Decoupled per-service states (independent locking, zero cross-service contention) ──

/// KG-Course (KGC) service state.
pub struct KgcState {
    /// Serializes KGC HTTP requests to prevent Struts token races.
    ///
    /// Struts 1 stores ONE token per HTTP session (server-side). Any KGC page
    /// load that renders a form calls `saveToken()`, overwriting the previous
    /// token. When multiple KGC requests execute concurrently (e.g. background
    /// polling + syllabus enrichment), the token extracted from page A is
    /// invalidated by page B's load, causing all subsequent form POSTs to fail.
    pub gate: Mutex<()>,
}

/// Luna LMS service state.
pub struct LunaState;

/// KWIC Portal service handle. University credentials belong to SESSIONS.
pub struct KwicState;

macro_rules! session_handle {
    ($ty:ty, $service:ident) => {
        impl $ty {
            pub(crate) fn session(&self) -> crate::session_coordinator::SessionLease {
                crate::session_coordinator::SESSIONS
                    .lease(crate::session_coordinator::Service::$service)
            }
        }
    };
}
session_handle!(KgcState, Kgc);
session_handle!(LunaState, Luna);
session_handle!(KwicState, Kwic);

/// Microsoft 365 Mail service state.
pub struct MailState {
    pub(crate) cancellation: crate::oauth_http::Cancellation,
    pub client: Mutex<mail::MailClient>,
}

/// Google Calendar service state.
pub struct GCalState {
    pub(crate) cancellation: crate::oauth_http::Cancellation,
    pub client: Mutex<google_calendar::GoogleCalendarClient>,
}

/// Shared theme state so child webviews can read the current theme.
pub struct ThemeState(pub std::sync::Mutex<String>);

#[tauri::command]
pub fn get_app_theme(state: tauri::State<'_, ThemeState>) -> String {
    state.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub fn set_app_theme(app: tauri::AppHandle, state: tauri::State<'_, ThemeState>, theme: String) {
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = theme;
    let _ = app.emit("app-theme-changed", ());
}

#[tauri::command]
pub fn request_app_restart(app: tauri::AppHandle) {
    super::app_shutdown::request_restart(&app);
}
