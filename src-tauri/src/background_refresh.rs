use chrono::{Datelike, Local, TimeZone, Weekday};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::client;
use crate::db::{epoch_secs, Database};
use crate::{KgcState, KwicState, LunaState, MailState};

const INITIAL_REFRESH_DELAY: Duration = Duration::from_secs(15);
const REFRESH_TICK: Duration = Duration::from_secs(5 * 60);
const FAST_CACHE_MAX_AGE_SECS: i64 = 5 * 60;
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
        }
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

#[derive(Debug, Clone, Serialize, Default)]
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
fn is_main_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

pub fn start_background_refresh_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
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

pub async fn refresh_backend_now(
    app: &AppHandle,
    keys: Option<&[String]>,
    force: bool,
) -> Result<Vec<String>, String> {
    let request = BackendRefreshRequest::new(keys, force);
    let mut updated = Vec::new();

    if request.wants_any(&["notifications", "luna_updates", "kwic_home", "mail_inbox"]) {
        updated.extend(crate::notifier::sync_notifications_now(app).await?);
    }

    if request.wants_any(&[
        "schedule_data",
        "luna_todo",
        "weather",
        "grades",
        "registration",
        "cancellations",
        "makeup",
        "rooms",
        "student_profile",
        "exams",
    ]) {
        updated.extend(refresh_backend_data_with_request(app, &request).await?);
    }

    Ok(dedup_keys(updated))
}

pub async fn refresh_backend_data_now(app: &AppHandle) -> Result<Vec<String>, String> {
    refresh_backend_data_with_request(app, &BackendRefreshRequest::default()).await
}

async fn refresh_backend_data_with_request(
    app: &AppHandle,
    request: &BackendRefreshRequest,
) -> Result<Vec<String>, String> {
    let state = app.state::<BackendRefreshState>();
    if state.running.swap(true, Ordering::SeqCst) {
        return Ok(Vec::new());
    }

    struct RunningGuard<'a>(&'a AtomicBool);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RunningGuard(&state.running);

    refresh_backend_data_inner(app, request).await
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

async fn refresh_backend_data_inner(
    app: &AppHandle,
    request: &BackendRefreshRequest,
) -> Result<Vec<String>, String> {
    maybe_renew_sessions(app).await;

    let db = app.state::<Database>();
    let session_status = sync_backend_session_status(app, true).await?;
    let mut kgc_authenticated = session_status.kgc_session_present;
    let luna_authenticated = session_status.luna_authenticated;
    let mut updated_keys = Vec::new();
    let mut schedule_changed = false;

    // KGC is not kept alive on a timer. When an automatic KGC-backed data task
    // is actually due, allow that run to make one hidden-login attempt.
    if kgc_data_request_due(request, &db) {
        kgc_authenticated = crate::commands::ensure_kgc_session_for_automatic_request(app).await;
    }

    if request.wants("luna_todo")
        && luna_authenticated
        && (request.force || cache_is_stale(&db, "luna_todo", FAST_CACHE_MAX_AGE_SECS))
    {
        match crate::luna_commands::luna_fetch_todo(
            app.state::<LunaState>(),
            app.state::<Database>(),
        )
        .await
        {
            Ok(_) => updated_keys.push("luna_todo".to_string()),
            Err(e) => log::warn!("background refresh: luna_todo failed: {}", e),
        }
    }

    if request.wants("weather")
        && (request.force || cache_is_stale(&db, "weather", WEATHER_CACHE_MAX_AGE_SECS))
    {
        match crate::commands::fetch_weather().await {
            Ok(data) => {
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache("weather", &json);
                }
                updated_keys.push("weather".to_string());
            }
            Err(e) => log::warn!("background refresh: weather failed: {}", e),
        }
    }

    // Luna timetable must refresh even when KGC is down or logged out.
    if request.wants("schedule_data")
        && (kgc_authenticated || luna_authenticated)
        && (request.force || schedule_refresh_is_stale(&db))
    {
        match crate::timetable::sync_schedule_data(
            app.state::<KgcState>(),
            app.state::<LunaState>(),
            app.state::<Database>(),
        )
        .await
        {
            Ok(_) => {
                updated_keys.push("schedule_data".to_string());
                schedule_changed = true;
            }
            Err(e) => log::warn!("background refresh: schedule sync failed: {}", e),
        }
    }

    if request.wants("schedule_data") && luna_authenticated {
        match crate::timetable::refresh_luna_counts_internal(
            &app.state::<LunaState>(),
            &db,
            request.force,
        )
        .await
        {
            Ok(updated) if updated > 0 => {
                updated_keys.push("schedule_data".to_string());
                schedule_changed = true;
            }
            Ok(_) => {}
            Err(e) => log::warn!("background refresh: luna counts failed: {}", e),
        }
    }

    if kgc_authenticated {
        if request.wants("grades")
            && (request.force || cache_is_stale(&db, "grades", ACADEMIC_RECORD_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_grades(app.state::<KgcState>(), app.state::<Database>())
                .await
            {
                Ok(_) => updated_keys.push("grades".to_string()),
                Err(e) => log::warn!("background refresh: grades failed: {}", e),
            }
        }
        if request.wants("registration")
            && (request.force
                || cache_is_stale(&db, "registration", ACADEMIC_RECORD_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_registration(
                app.state::<KgcState>(),
                app.state::<Database>(),
            )
            .await
            {
                Ok(_) => updated_keys.push("registration".to_string()),
                Err(e) => log::warn!("background refresh: registration failed: {}", e),
            }
        }
        if request.wants("cancellations")
            && (request.force || cache_is_stale(&db, "cancellations", STABLE_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_cancellations(
                app.state::<KgcState>(),
                app.state::<Database>(),
            )
            .await
            {
                Ok(_) => updated_keys.push("cancellations".to_string()),
                Err(e) => log::warn!("background refresh: cancellations failed: {}", e),
            }
        }
        if request.wants("makeup")
            && (request.force || cache_is_stale(&db, "makeup", STABLE_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_makeup_classes(
                app.state::<KgcState>(),
                app.state::<Database>(),
            )
            .await
            {
                Ok(_) => updated_keys.push("makeup".to_string()),
                Err(e) => log::warn!("background refresh: makeup failed: {}", e),
            }
        }
        if request.wants("rooms")
            && (request.force || cache_is_stale(&db, "rooms", STABLE_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_room_changes(
                app.state::<KgcState>(),
                app.state::<Database>(),
            )
            .await
            {
                Ok(_) => updated_keys.push("rooms".to_string()),
                Err(e) => log::warn!("background refresh: rooms failed: {}", e),
            }
        }
        if request.wants("student_profile")
            && (request.force || cache_is_stale(&db, "student_profile", STABLE_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_student_profile(
                app.state::<KgcState>(),
                app.state::<Database>(),
            )
            .await
            {
                Ok(_) => updated_keys.push("student_profile".to_string()),
                Err(e) => log::warn!("background refresh: student_profile failed: {}", e),
            }
        }
        if request.wants("exams")
            && (request.force || cache_is_stale(&db, "exam_timetable", STABLE_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_exam_timetable(
                app.state::<KgcState>(),
                app.state::<Database>(),
            )
            .await
            {
                Ok(_) => updated_keys.push("exams".to_string()),
                Err(e) => log::warn!("background refresh: exams failed: {}", e),
            }
        }
    }

    if request.wants("schedule_data") {
        maybe_auto_sync_calendars(app, &db, schedule_changed, request.force).await;
    }

    if !updated_keys.is_empty() {
        emit_cache_updates(app, updated_keys.clone());
    }

    Ok(dedup_keys(updated_keys))
}

pub async fn sync_backend_session_status(
    app: &AppHandle,
    attempt_recovery: bool,
) -> Result<BackendSessionStatusPayload, String> {
    let state = app.state::<BackendRefreshState>();
    let owns_recovery =
        attempt_recovery && !state.session_sync_running.swap(true, Ordering::SeqCst);
    struct RecoveryGuard<'a>(Option<&'a AtomicBool>);
    impl Drop for RecoveryGuard<'_> {
        fn drop(&mut self) {
            if let Some(running) = self.0 {
                running.store(false, Ordering::SeqCst);
            }
        }
    }
    let _guard = RecoveryGuard(owns_recovery.then_some(&state.session_sync_running));

    let payload = sync_backend_session_status_inner(app, owns_recovery).await?;
    emit_session_status(app, &payload);
    Ok(payload)
}

async fn sync_backend_session_status_inner(
    app: &AppHandle,
    attempt_recovery: bool,
) -> Result<BackendSessionStatusPayload, String> {
    let kgc_had_session = is_kgc_authenticated(app).await;
    let luna_had_session = is_luna_authenticated(app).await;
    let kwic_had_session = is_kwic_authenticated(app).await;

    // KGC is a short-lived, request-driven session. Background status sync must
    // never touch its server; actual KGC requests and explicit user checks are
    // responsible for confirming whether the stored session still works.
    let kgc_status = kgc_status_from_memory(app, kgc_had_session).await;
    let mut luna_valid = if luna_had_session && attempt_recovery {
        match crate::luna_commands::luna_check_session(app.state::<LunaState>()).await {
            Ok(valid) => valid,
            Err(e) => {
                log::warn!(
                    "session status: Luna validation transient failure, retaining session: {}",
                    e
                );
                true
            }
        }
    } else {
        luna_had_session
    };
    let mut kwic_valid = if kwic_had_session && attempt_recovery {
        match crate::kwic_commands::kwic_check_session(app.state::<KwicState>()).await {
            Ok(valid) => valid,
            Err(e) => {
                log::warn!(
                    "session status: KWIC validation transient failure, retaining session: {}",
                    e
                );
                true
            }
        }
    } else {
        kwic_had_session
    };
    if attempt_recovery {
        // Proactive recovery is restricted to the core services. KGC is only
        // recovered after an actual KGC request fails or a user asks for it.
        let has_session_evidence = kgc_had_session || luna_had_session || kwic_had_session;
        if has_session_evidence
            && !luna_valid
            && attempt_service_recovery(app, SessionService::Luna).await
        {
            luna_valid = crate::luna_commands::luna_check_session(app.state::<LunaState>())
                .await
                .unwrap_or(true);
        }
        if has_session_evidence
            && !kwic_valid
            && attempt_service_recovery(app, SessionService::Kwic).await
        {
            kwic_valid = crate::kwic_commands::kwic_check_session(app.state::<KwicState>())
                .await
                .unwrap_or(true);
        }
    }

    let mail_status = crate::mail_commands::mail_check_session(app.state::<MailState>())
        .await
        .unwrap_or(crate::mail_commands::MailSessionStatus {
            authenticated: false,
            email: String::new(),
            display_name: String::new(),
        });

    // Luna and KWIC are the core app sessions. KGC enriches timetable and
    // academic-record features, but its isolated failure must not put the
    // whole app into the user-facing re-authentication state.
    let core_session_expired = !(luna_valid && kwic_valid);

    Ok(BackendSessionStatusPayload {
        kgc_session_present: kgc_status.valid,
        session_expired: core_session_expired,
        username: if kgc_status.valid {
            kgc_status.username
        } else {
            String::new()
        },
        display_name: if kgc_status.valid {
            kgc_status.display_name
        } else {
            String::new()
        },
        student_id: if kgc_status.valid {
            kgc_status.student_id
        } else {
            String::new()
        },
        faculty: if kgc_status.valid {
            kgc_status.faculty
        } else {
            String::new()
        },
        department: if kgc_status.valid {
            kgc_status.department
        } else {
            String::new()
        },
        luna_authenticated: luna_valid,
        kwic_authenticated: kwic_valid,
        mail_authenticated: mail_status.authenticated,
        mail_email: mail_status.email,
        mail_display_name: mail_status.display_name,
    })
}

async fn kgc_status_from_memory(app: &AppHandle, valid: bool) -> crate::commands::SessionStatus {
    let state = app.state::<KgcState>();
    let client = state.client.lock().await;
    match client.session.as_ref() {
        Some(session) => crate::commands::SessionStatus {
            valid,
            username: session.username.clone(),
            display_name: session.display_name.clone(),
            student_id: session.student_id.clone(),
            faculty: session.faculty.clone(),
            department: session.department.clone(),
        },
        None => crate::commands::SessionStatus {
            valid: false,
            username: String::new(),
            display_name: String::new(),
            student_id: String::new(),
            faculty: String::new(),
            department: String::new(),
        },
    }
}

async fn attempt_service_recovery(app: &AppHandle, service: SessionService) -> bool {
    let state = app.state::<BackendRefreshState>();
    let now = epoch_secs();
    if !state.recovery_due(service, now) {
        log::debug!(
            "session recovery: {} skipped during backoff",
            service.name()
        );
        return false;
    }

    log::info!("session recovery: attempting {}", service.name());
    let succeeded = match crate::commands::sync_session(
        app.clone(),
        app.state::<KgcState>(),
        app.state::<LunaState>(),
        app.state::<KwicState>(),
        service.name().to_string(),
    )
    .await
    {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => {
            log::warn!("session recovery: {} failed: {}", service.name(), e);
            false
        }
    };
    state.record_recovery(service, now, succeeded);
    succeeded
}

async fn maybe_renew_sessions(app: &AppHandle) {
    let state = app.state::<BackendRefreshState>();
    if state.session_sync_running.swap(true, Ordering::SeqCst) {
        return;
    }

    struct RunningGuard<'a>(&'a AtomicBool);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RunningGuard(&state.session_sync_running);

    if let Err(e) = maybe_renew_sessions_inner(app).await {
        log::warn!("background session renew failed: {}", e);
    }
}

async fn maybe_renew_sessions_inner(app: &AppHandle) -> Result<(), String> {
    // Reactive trigger: a core-service cookie is near its explicit expiry.
    let expiry_due = soonest_core_session_expiry_secs(app)
        .await
        .is_some_and(|secs| secs <= SESSION_RENEW_THRESHOLD_SECS);

    // Time-based keep-alive: only meaningful if we currently believe we're
    // logged in to at least one service (otherwise renewal would just spawn a
    // hidden Okta-login webview for nothing). This covers the session-only
    // cookie case where expiry_due can never fire.
    let any_session = current_core_session_present(app).await;
    let state = app.state::<BackendRefreshState>();
    let now = epoch_secs();
    let last = state.last_session_keepalive.load(Ordering::Relaxed);
    let keepalive_due = any_session && now.saturating_sub(last) >= SESSION_KEEPALIVE_INTERVAL_SECS;

    if !expiry_due && !keepalive_due {
        return Ok(());
    }
    if now.saturating_sub(last) < SESSION_RENEW_MIN_INTERVAL_SECS {
        return Ok(());
    }

    log::info!(
        "background refresh: headless session renew (expiry_due={}, keepalive_due={})",
        expiry_due,
        keepalive_due
    );
    // Record the attempt time up front so a dead Okta session doesn't make us
    // retry every tick — the reactive path handles real usage in the meantime.
    state.last_session_keepalive.store(now, Ordering::Relaxed);
    let _ = crate::commands::sync_session(
        app.clone(),
        app.state::<KgcState>(),
        app.state::<LunaState>(),
        app.state::<KwicState>(),
        "all".to_string(),
    )
    .await?;
    Ok(())
}

/// Whether we currently believe at least one core service is authenticated.
async fn current_core_session_present(app: &AppHandle) -> bool {
    if app.state::<LunaState>().client.lock().await.authenticated {
        return true;
    }
    app.state::<KwicState>().client.lock().await.authenticated
}

async fn soonest_core_session_expiry_secs(app: &AppHandle) -> Option<i64> {
    let luna_exp =
        client::soonest_cookie_expiry(&app.state::<LunaState>().client.lock().await.cookie_store);
    let kwic_exp =
        client::soonest_cookie_expiry(&app.state::<KwicState>().client.lock().await.cookie_store);
    [luna_exp, kwic_exp].into_iter().flatten().min()
}

fn cache_is_stale(db: &Database, key: &str, max_age_secs: i64) -> bool {
    match db.get_data_cache(key) {
        Ok(Some((_, updated_at))) => epoch_secs().saturating_sub(updated_at) >= max_age_secs,
        Ok(None) => true,
        Err(_) => true,
    }
}

fn kgc_data_request_due(request: &BackendRefreshRequest, db: &Database) -> bool {
    (request.wants("schedule_data") && (request.force || schedule_refresh_is_stale(db)))
        || (request.wants("grades")
            && (request.force || cache_is_stale(db, "grades", ACADEMIC_RECORD_CACHE_MAX_AGE_SECS)))
        || (request.wants("registration")
            && (request.force
                || cache_is_stale(db, "registration", ACADEMIC_RECORD_CACHE_MAX_AGE_SECS)))
        || (request.wants("cancellations")
            && (request.force || cache_is_stale(db, "cancellations", STABLE_CACHE_MAX_AGE_SECS)))
        || (request.wants("makeup")
            && (request.force || cache_is_stale(db, "makeup", STABLE_CACHE_MAX_AGE_SECS)))
        || (request.wants("rooms")
            && (request.force || cache_is_stale(db, "rooms", STABLE_CACHE_MAX_AGE_SECS)))
        || (request.wants("student_profile")
            && (request.force || cache_is_stale(db, "student_profile", STABLE_CACHE_MAX_AGE_SECS)))
        || (request.wants("exams")
            && (request.force || cache_is_stale(db, "exam_timetable", STABLE_CACHE_MAX_AGE_SECS)))
}

fn schedule_refresh_is_stale(db: &Database) -> bool {
    let now = epoch_secs();
    let Some(snapshot) = db.get_snapshot_state().ok().flatten() else {
        return true;
    };

    if snapshot.updated_at <= 0
        || now.saturating_sub(snapshot.updated_at) >= SCHEDULE_CACHE_MAX_AGE_SECS
    {
        return true;
    }

    if Local::now().weekday() != Weekday::Sun {
        return false;
    }

    let snapshot_day = chrono::Utc
        .timestamp_opt(snapshot.updated_at, 0)
        .single()
        .map(|dt| dt.with_timezone(&Local).date_naive());
    snapshot_day != Some(Local::now().date_naive())
}

async fn is_kgc_authenticated(app: &AppHandle) -> bool {
    app.state::<KgcState>()
        .client
        .lock()
        .await
        .is_authenticated()
}

async fn is_luna_authenticated(app: &AppHandle) -> bool {
    app.state::<LunaState>().client.lock().await.authenticated
}

async fn is_kwic_authenticated(app: &AppHandle) -> bool {
    app.state::<KwicState>().client.lock().await.authenticated
}

fn read_numeric_cache(db: &Database, key: &str) -> Option<i64> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str::<i64>(&json).ok())
}

fn save_numeric_cache(db: &Database, key: &str, value: i64) {
    if let Ok(json) = serde_json::to_string(&value) {
        let _ = db.save_data_cache(key, &json);
    }
}

fn gcal_sync_interval_secs() -> i64 {
    let cfg = crate::commands::load_calendar_config();
    let hours = cfg
        .cal_sync_interval
        .clamp(GCAL_SYNC_MIN_HOURS, GCAL_SYNC_MAX_HOURS);
    let hours = if hours == 0 {
        GCAL_SYNC_DEFAULT_HOURS
    } else {
        hours
    };
    i64::from(hours) * 60 * 60
}

fn build_calendar_entries(
    entries: &[crate::db::KgcCourseRow],
) -> Vec<crate::google_calendar::CalendarSyncEntry> {
    entries
        .iter()
        .map(|entry| crate::google_calendar::CalendarSyncEntry {
            day: match entry.day {
                1 => "月",
                2 => "火",
                3 => "水",
                4 => "木",
                5 => "金",
                6 => "土",
                _ => "",
            }
            .to_string(),
            period: entry.period,
            course_name: entry.name.clone(),
            room: entry.room.clone(),
            is_cancelled: entry.is_cancelled,
        })
        .filter(|entry| !entry.day.is_empty())
        .collect()
}

fn build_sync_weeks(
    raw: &crate::db::ScheduleRawData,
) -> Vec<(String, Vec<crate::google_calendar::CalendarSyncEntry>)> {
    let candidates = [
        (&raw.current_week_label, &raw.kgc_entries_current),
        (&raw.next_week_label, &raw.kgc_entries_next),
    ];
    let mut seen = BTreeSet::new();
    let mut weeks = Vec::new();

    for (label, entries) in candidates {
        let label = label.trim();
        if label.is_empty() || entries.is_empty() || !seen.insert(label.to_string()) {
            continue;
        }
        weeks.push((label.to_string(), build_calendar_entries(entries)));
    }

    weeks
}

async fn maybe_auto_sync_calendars(
    app: &AppHandle,
    db: &Database,
    schedule_changed: bool,
    force: bool,
) {
    let cal_cfg = crate::commands::load_calendar_config();
    if !cal_cfg.gcal_auto_sync {
        return;
    }

    let last_run = read_numeric_cache(db, GCAL_AUTO_SYNC_LAST_RUN_KEY).unwrap_or(0);
    let due = epoch_secs().saturating_sub(last_run) >= gcal_sync_interval_secs();
    if !force && !schedule_changed && !due {
        return;
    }

    let Some(snapshot) = db.get_snapshot_state().ok().flatten() else {
        return;
    };

    let scope = crate::academic_period::visible_weeks(
        &snapshot.current_week_label,
        &snapshot.next_week_label,
        &snapshot.luna_year,
        &snapshot.luna_term,
        chrono::Local::now().date_naive(),
    );
    let raw = match db.build_raw_data(
        &scope.current,
        &scope.next,
        snapshot.luna_communities.clone(),
    ) {
        Ok(raw) => raw,
        Err(e) => {
            log::warn!(
                "background refresh: build raw schedule for gcal sync failed: {}",
                e
            );
            return;
        }
    };
    let weeks = build_sync_weeks(&raw);
    if weeks.is_empty() {
        return;
    }

    let gcal_state = app.state::<crate::GCalState>();
    let mut gcal = gcal_state.client.lock().await;
    if !gcal.status().authenticated {
        return;
    }

    for (label, entries) in weeks {
        if entries.is_empty() {
            continue;
        }
        if let Err(e) = gcal.sync_timetable(entries, label).await {
            log::warn!("background refresh: gcal auto-sync failed: {}", e);
            return;
        }
    }

    drop(gcal);
    save_numeric_cache(db, GCAL_AUTO_SYNC_LAST_RUN_KEY, epoch_secs());
}
