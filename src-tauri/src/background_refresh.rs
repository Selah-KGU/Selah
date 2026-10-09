use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::epoch_secs;

#[path = "background_refresh/calendar.rs"]
mod calendar;
#[path = "background_refresh/commands.rs"]
mod commands;
#[path = "background_refresh/data.rs"]
mod data;
#[path = "background_refresh/session.rs"]
mod session;

pub use commands::*;
pub use data::{refresh_backend_data_now, refresh_backend_now, refresh_on_window_focus};
pub use session::sync_backend_session_status;

const INITIAL_REFRESH_DELAY: Duration = Duration::from_secs(15);
const REFRESH_TICK: Duration = Duration::from_secs(5 * 60);
pub(crate) const FOCUSED_FAST_CACHE_MAX_AGE_SECS: i64 = 5 * 60;
pub(crate) const IDLE_FAST_CACHE_MAX_AGE_SECS: i64 = 15 * 60;
const WEATHER_CACHE_MAX_AGE_SECS: i64 = 60 * 60;
const STABLE_CACHE_MAX_AGE_SECS: i64 = 12 * 60 * 60;
const ACADEMIC_RECORD_CACHE_MAX_AGE_SECS: i64 = 72 * 60 * 60;
const SCHEDULE_CACHE_MAX_AGE_SECS: i64 = 6 * 60 * 60;
const GCAL_AUTO_SYNC_LAST_RUN_KEY: &str = "gcal_auto_sync_last_run";
const GCAL_SYNC_MIN_HOURS: u32 = 6;
const GCAL_SYNC_MAX_HOURS: u32 = 72;
const GCAL_SYNC_DEFAULT_HOURS: u32 = 12;

pub struct BackendRefreshState {
    running: AtomicBool,
    last_emitted_session: Mutex<Option<BackendSessionStatusPayload>>,
}

impl BackendRefreshState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            last_emitted_session: Mutex::new(None),
        }
    }

    fn session_status_unchanged(&self, payload: &BackendSessionStatusPayload) -> bool {
        let last = self
            .last_emitted_session
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        last.as_ref() == Some(payload)
    }

    fn store_session_status(&self, payload: BackendSessionStatusPayload) {
        let mut last = self
            .last_emitted_session
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        *last = Some(payload);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendCacheUpdatePayload {
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct BackendSessionStatusPayload {
    pub(crate) university: Option<crate::session_coordinator::SessionDiagnostics>,
    pub generation: u64,
    pub signed_out: bool,
    pub kgc_session_present: bool,
    pub session_expired: bool,
    pub username: String,
    pub display_name: String,
    pub student_id: String,
    pub faculty: String,
    pub department: String,
    pub luna_authenticated: bool,
    pub kwic_authenticated: bool,
    pub mail_authenticated: bool,
    pub mail_generation: u64,
    pub mail_connection_id: Option<String>,
    pub mail_email: String,
    pub mail_display_name: String,
}

#[derive(Debug, Clone, Default)]
struct BackendRefreshRequest {
    keys: Option<BTreeSet<String>>,
    force: bool,
}

impl BackendRefreshRequest {
    fn new(keys: Option<&[String]>, force: bool) -> Self {
        let keys = keys.map(|items| {
            items
                .iter()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty())
                .collect::<BTreeSet<_>>()
        });
        Self { keys, force }
    }

    fn wants(&self, key: &str) -> bool {
        self.keys
            .as_ref()
            .map(|keys| keys.contains(key))
            .unwrap_or(true)
    }

    fn wants_any(&self, keys: &[&str]) -> bool {
        keys.iter().any(|key| self.wants(key))
    }
}

/// Returns true when the main window is visible. Used to gate background
/// data refreshes so a hidden window doesn't keep the network/CPU spinning.
pub(crate) fn is_main_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Focused windows keep the 5-minute campus poll. An open but inactive window
/// stretches that to 15 minutes. If focus cannot be queried, a visible window
/// stays on the faster cadence so a platform quirk cannot silently stall data.
pub(crate) fn is_app_focused(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    match window.is_focused() {
        Ok(focused) => focused,
        Err(_) => window.is_visible().unwrap_or(false),
    }
}

pub(crate) fn fast_cache_max_age_secs(focused: bool) -> i64 {
    if focused {
        FOCUSED_FAST_CACHE_MAX_AGE_SECS
    } else {
        IDLE_FAST_CACHE_MAX_AGE_SECS
    }
}

pub fn start_background_refresh_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let compact_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let db = compact_app.state::<crate::db::Database>().scope();
            let compacted = crate::kwic_commands::compact_cached_kwic_details(&db);
            if compacted > 0 {
                log::info!(
                    "compacted {compacted} oversized KWIC detail cache entr{}",
                    if compacted == 1 { "y" } else { "ies" }
                );
            }
        });
        tokio::time::sleep(INITIAL_REFRESH_DELAY).await;

        if let Err(e) = refresh_backend_data_now(&app).await {
            log::warn!("background refresh failed: {}", e);
        }

        let mut interval = tokio::time::interval(REFRESH_TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        // Tracks consecutive ticks the window was hidden — we skip refresh
        // while hidden, but still run at most once per ~30min so caches do not
        // grow stale forever.
        let hidden_skip_max: u32 = 5; // 5 ticks = 25 min of skipping, then force one refresh
        let mut hidden_streak: u32 = 0;
        let mut last_tick = epoch_secs();
        loop {
            interval.tick().await;
            let now = epoch_secs();
            let woke = now.saturating_sub(last_tick) > (REFRESH_TICK.as_secs() * 2) as i64;
            last_tick = now;
            let visible = is_main_window_visible(&app);
            if !woke && !visible && hidden_streak < hidden_skip_max {
                hidden_streak = hidden_streak.saturating_add(1);
                continue;
            }
            hidden_streak = 0;
            if let Err(e) = refresh_backend_data_now(&app).await {
                log::warn!("background refresh failed: {}", e);
            }
        }
    });
}

pub fn emit_cache_updates(app: &AppHandle, keys: Vec<String>) {
    let deduped = dedup_keys(keys);
    if deduped.is_empty() {
        return;
    }
    if let Err(e) = app.emit(
        "backend-cache-updated",
        BackendCacheUpdatePayload { keys: deduped },
    ) {
        log::warn!("backend-cache-updated emit failed: {}", e);
    }
}

fn emit_session_status(app: &AppHandle, payload: &BackendSessionStatusPayload) {
    if let Err(e) = app.emit("backend-session-status", payload) {
        log::warn!("backend-session-status emit failed: {}", e);
    }
}

fn dedup_keys(keys: Vec<String>) -> Vec<String> {
    keys.into_iter()
        .filter(|key| !key.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::fast_cache_max_age_secs;

    #[test]
    fn idle_window_polls_slower_than_focused_window() {
        assert_eq!(fast_cache_max_age_secs(true), 5 * 60);
        assert_eq!(fast_cache_max_age_secs(false), 15 * 60);
    }
}
