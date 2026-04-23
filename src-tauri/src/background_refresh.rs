use chrono::{Datelike, Local, TimeZone, Weekday};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
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
const SCHEDULE_CACHE_MAX_AGE_SECS: i64 = 6 * 60 * 60;
const SESSION_RENEW_THRESHOLD_SECS: i64 = 5 * 60;
const GCAL_AUTO_SYNC_LAST_RUN_KEY: &str = "gcal_auto_sync_last_run";
const GCAL_SYNC_MIN_HOURS: u32 = 6;
const GCAL_SYNC_MAX_HOURS: u32 = 72;
const GCAL_SYNC_DEFAULT_HOURS: u32 = 12;

pub struct BackendRefreshState {
    running: AtomicBool,
    session_sync_running: AtomicBool,
}

impl BackendRefreshState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            session_sync_running: AtomicBool::new(false),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendCacheUpdatePayload {
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BackendSessionStatusPayload {
    pub kgc_valid: bool,
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

pub fn start_background_refresh_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_REFRESH_DELAY).await;

        if let Err(e) = refresh_backend_data_now(&app).await {
            log::warn!("background refresh failed: {}", e);
        }

        let mut interval = tokio::time::interval(REFRESH_TICK);
        interval.tick().await;
        loop {
            interval.tick().await;
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
        "mail_profile",
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

    let result = refresh_backend_data_inner(app, request).await;
    state.running.store(false, Ordering::SeqCst);
    result
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
    let kgc_authenticated = session_status.kgc_valid;
    let luna_authenticated = session_status.luna_authenticated;
    let mail_authenticated = session_status.mail_authenticated;
    let mut updated_keys = Vec::new();
    let mut schedule_changed = false;

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

    if request.wants("schedule_data")
        && kgc_authenticated
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
            && (request.force || cache_is_stale(&db, "grades", STABLE_CACHE_MAX_AGE_SECS))
        {
            match crate::commands::fetch_grades(app.state::<KgcState>(), app.state::<Database>())
                .await
            {
                Ok(_) => updated_keys.push("grades".to_string()),
                Err(e) => log::warn!("background refresh: grades failed: {}", e),
            }
        }
        if request.wants("registration")
            && (request.force || cache_is_stale(&db, "registration", STABLE_CACHE_MAX_AGE_SECS))
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

    if request.wants("mail_profile")
        && mail_authenticated
        && (request.force || cache_is_stale(&db, "mail_profile", STABLE_CACHE_MAX_AGE_SECS))
    {
        match crate::mail_commands::mail_fetch_profile(
            app.state::<MailState>(),
            app.state::<Database>(),
        )
        .await
        {
            Ok(_) => updated_keys.push("mail_profile".to_string()),
            Err(e) => log::warn!("background refresh: mail_profile failed: {}", e),
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
    let payload = sync_backend_session_status_inner(app, attempt_recovery).await?;
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

    let mut kgc_status = if kgc_had_session {
        crate::commands::check_session(app.state::<KgcState>()).await?
    } else {
        crate::commands::SessionStatus {
            valid: false,
            username: String::new(),
            display_name: String::new(),
            student_id: String::new(),
            faculty: String::new(),
            department: String::new(),
        }
    };
    let mut luna_valid = if luna_had_session {
        crate::luna_commands::luna_check_session(app.state::<LunaState>())
            .await
            .unwrap_or(false)
    } else {
        false
    };
    let mut kwic_valid = if kwic_had_session {
        crate::kwic_commands::kwic_check_session(app.state::<KwicState>())
            .await
            .unwrap_or(false)
    } else {
        false
    };
    let mut kgc_session_present = is_kgc_authenticated(app).await;

    if attempt_recovery {
        if kgc_had_session && !kgc_status.valid && !kgc_session_present {
            let _ = crate::commands::sync_session(
                app.clone(),
                app.state::<KgcState>(),
                app.state::<LunaState>(),
                app.state::<KwicState>(),
                "all".to_string(),
            )
            .await;
            kgc_status = crate::commands::check_session(app.state::<KgcState>())
                .await
                .unwrap_or(kgc_status);
            kgc_session_present = is_kgc_authenticated(app).await;
            if luna_had_session {
                luna_valid = crate::luna_commands::luna_check_session(app.state::<LunaState>())
                    .await
                    .unwrap_or(false);
            }
            if kwic_had_session {
                kwic_valid = crate::kwic_commands::kwic_check_session(app.state::<KwicState>())
                    .await
                    .unwrap_or(false);
            }
        } else {
            if luna_had_session && !luna_valid {
                let _ = crate::commands::sync_session(
                    app.clone(),
                    app.state::<KgcState>(),
                    app.state::<LunaState>(),
                    app.state::<KwicState>(),
                    "luna".to_string(),
                )
                .await;
                luna_valid = crate::luna_commands::luna_check_session(app.state::<LunaState>())
                    .await
                    .unwrap_or(false);
            }
            if kwic_had_session && !kwic_valid {
                let _ = crate::commands::sync_session(
                    app.clone(),
                    app.state::<KgcState>(),
                    app.state::<LunaState>(),
                    app.state::<KwicState>(),
                    "kwic".to_string(),
                )
                .await;
                kwic_valid = crate::kwic_commands::kwic_check_session(app.state::<KwicState>())
                    .await
                    .unwrap_or(false);
            }
        }
    }

    let mail_status = crate::mail_commands::mail_check_session(app.state::<MailState>())
        .await
        .unwrap_or(crate::mail_commands::MailSessionStatus {
            authenticated: false,
            email: String::new(),
            display_name: String::new(),
        });

    Ok(BackendSessionStatusPayload {
        kgc_valid: kgc_status.valid,
        session_expired: kgc_had_session && !kgc_status.valid && !kgc_session_present,
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

async fn maybe_renew_sessions(app: &AppHandle) {
    let state = app.state::<BackendRefreshState>();
    if state.session_sync_running.swap(true, Ordering::SeqCst) {
        return;
    }

    let result = maybe_renew_sessions_inner(app).await;
    state.session_sync_running.store(false, Ordering::SeqCst);

    if let Err(e) = result {
        log::warn!("background session renew failed: {}", e);
    }
}

async fn maybe_renew_sessions_inner(app: &AppHandle) -> Result<(), String> {
    let Some(expiry_secs) = soonest_session_expiry_secs(app).await else {
        return Ok(());
    };
    if expiry_secs > SESSION_RENEW_THRESHOLD_SECS {
        return Ok(());
    }

    log::info!(
        "background refresh: cookie expiry in {}s, attempting headless session renew",
        expiry_secs
    );
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

async fn soonest_session_expiry_secs(app: &AppHandle) -> Option<i64> {
    let kgc_exp = app
        .state::<KgcState>()
        .client
        .lock()
        .await
        .soonest_cookie_expiry_secs();
    let luna_exp =
        client::soonest_cookie_expiry(&app.state::<LunaState>().client.lock().await.cookie_store);
    let kwic_exp =
        client::soonest_cookie_expiry(&app.state::<KwicState>().client.lock().await.cookie_store);
    [kgc_exp, luna_exp, kwic_exp].into_iter().flatten().min()
}

fn cache_is_stale(db: &Database, key: &str, max_age_secs: i64) -> bool {
    match db.get_data_cache(key) {
        Ok(Some((_, updated_at))) => epoch_secs().saturating_sub(updated_at) >= max_age_secs,
        Ok(None) => true,
        Err(_) => true,
    }
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

    let raw = match db.build_raw_data(
        &snapshot.current_week_label,
        &snapshot.next_week_label,
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
