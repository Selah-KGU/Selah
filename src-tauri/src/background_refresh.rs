use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicI64};
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
pub(crate) const SESSION_PROBE_REUSE_SECS: i64 = 90;
const WEATHER_CACHE_MAX_AGE_SECS: i64 = 60 * 60;
const STABLE_CACHE_MAX_AGE_SECS: i64 = 12 * 60 * 60;
const ACADEMIC_RECORD_CACHE_MAX_AGE_SECS: i64 = 72 * 60 * 60;
const SCHEDULE_CACHE_MAX_AGE_SECS: i64 = 6 * 60 * 60;
const SESSION_RENEW_THRESHOLD_SECS: i64 = 5 * 60;
// Time-based "keep-alive" for the core Luna/KWIC sessions. KGC is deliberately
// excluded because its cookies are sensitive to proactive renewal timing.
const SESSION_KEEPALIVE_INTERVAL_SECS: i64 = 6 * 60 * 60;
const SESSION_RENEW_MIN_INTERVAL_SECS: i64 = 30 * 60;
const SESSION_RECOVERY_SUCCESS_COOLDOWN_SECS: i64 = 30 * 60;
const SESSION_RECOVERY_BASE_DELAY_SECS: i64 = 10 * 60;
const SESSION_RECOVERY_MAX_DELAY_SECS: i64 = 2 * 60 * 60;
const GCAL_AUTO_SYNC_LAST_RUN_KEY: &str = "gcal_auto_sync_last_run";
const GCAL_SYNC_MIN_HOURS: u32 = 6;
const GCAL_SYNC_MAX_HOURS: u32 = 72;
const GCAL_SYNC_DEFAULT_HOURS: u32 = 12;

pub struct BackendRefreshState {
    running: AtomicBool,
    session_sync_running: AtomicBool,
    // Epoch seconds of the last headless keep-alive attempt.
    last_session_keepalive: AtomicI64,
    recovery: Mutex<[SessionRecoveryState; 2]>,
    last_emitted_session: Mutex<Option<BackendSessionStatusPayload>>,
}

#[derive(Clone, Copy, Default)]
struct SessionRecoveryState {
    last_attempt: i64,
    failures: u32,
}

impl BackendRefreshState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            session_sync_running: AtomicBool::new(false),
            // Allow a genuinely near-expiry cookie to renew at startup, but do
            // not make the fixed-cadence keep-alive immediately due.
            last_session_keepalive: AtomicI64::new(
                epoch_secs().saturating_sub(SESSION_RENEW_MIN_INTERVAL_SECS),
            ),
            recovery: Mutex::new([SessionRecoveryState::default(); 2]),
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

    fn recovery_due(&self, service: SessionService, now: i64) -> bool {
        let recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        let entry = recovery[service.index()];
        if entry.last_attempt == 0 {
            return true;
        }
        now.saturating_sub(entry.last_attempt) >= recovery_delay_secs(entry.failures)
    }

    fn record_recovery(&self, service: SessionService, now: i64, succeeded: bool) {
        let mut recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        let entry = &mut recovery[service.index()];
        entry.last_attempt = now;
        entry.failures = if succeeded {
            0
        } else {
            entry.failures.saturating_add(1)
        };
    }
}

#[derive(Clone, Copy)]
enum SessionService {
    Luna,
    Kwic,
}

impl SessionService {
    fn index(self) -> usize {
        match self {
            Self::Luna => 0,
            Self::Kwic => 1,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Luna => "luna",
            Self::Kwic => "kwic",
        }
    }
}

fn recovery_delay_secs(failures: u32) -> i64 {
    if failures == 0 {
        return SESSION_RECOVERY_SUCCESS_COOLDOWN_SECS;
    }
    let multiplier = 1_i64 << failures.saturating_sub(1).min(3);
    (SESSION_RECOVERY_BASE_DELAY_SECS * multiplier).min(SESSION_RECOVERY_MAX_DELAY_SECS)
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendCacheUpdatePayload {
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct BackendSessionStatusPayload {
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

pub(crate) fn recent_success_covers_probe(updated_at: Option<i64>, now: i64) -> bool {
    matches!(
        updated_at,
        Some(ts) if now.saturating_sub(ts) < SESSION_PROBE_REUSE_SECS
    )
}

pub fn start_background_refresh_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let compact_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let db = compact_app.state::<crate::db::Database>();
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
        loop {
            interval.tick().await;
            let visible = is_main_window_visible(&app);
            if !visible && hidden_streak < hidden_skip_max {
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
    use super::{fast_cache_max_age_secs, recent_success_covers_probe};

    #[test]
    fn idle_window_polls_slower_than_focused_window() {
        assert_eq!(fast_cache_max_age_secs(true), 5 * 60);
        assert_eq!(fast_cache_max_age_secs(false), 15 * 60);
    }

    #[test]
    fn recent_fetch_suppresses_only_a_duplicate_probe() {
        assert!(!recent_success_covers_probe(None, 1_000));
        assert!(recent_success_covers_probe(Some(950), 1_000));
        assert!(!recent_success_covers_probe(Some(800), 1_000));
    }
}
