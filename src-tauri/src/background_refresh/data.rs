//! Background data refresh and cache freshness.

use std::sync::atomic::Ordering;

use chrono::{Datelike, Local, TimeZone, Weekday};

use crate::db::Database;
use crate::{KgcState, LunaState};

use super::*;

pub async fn refresh_backend_now(
    app: &AppHandle,
    keys: Option<&[String]>,
    force: bool,
) -> Result<Vec<String>, String> {
    let request = BackendRefreshRequest::new(keys, force);
    let mut updated = Vec::new();

    if request.wants_any(&["notifications", "luna_updates", "kwic_home", "mail_inbox"]) {
        updated.extend(
            crate::notifier::sync_notification_sources(app, request.keys.as_ref(), request.force)
                .await?,
        );
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

pub async fn refresh_on_window_focus(app: &AppHandle) {
    if !focus_catch_up_allowed(crate::db::epoch_secs()) {
        return;
    }
    let db = app.state::<Database>();
    let max_age = super::fast_cache_max_age_secs(true);
    let keys = ["luna_todo", "luna_updates", "kwic_home", "mail_inbox"]
        .into_iter()
        .filter(|key| cache_is_stale(&db, key, max_age))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return;
    }
    log::debug!(
        "focus catch-up refreshing {} stale fast source(s)",
        keys.len()
    );
    if let Err(err) = refresh_backend_now(app, Some(&keys), false).await {
        log::warn!("focus catch-up refresh failed: {err}");
    }
}

fn focus_catch_up_allowed(now: i64) -> bool {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(0);
    let mut previous = LAST.load(Ordering::Relaxed);
    loop {
        if now.saturating_sub(previous) < 20 {
            return false;
        }
        match LAST.compare_exchange_weak(previous, now, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => previous = actual,
        }
    }
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

async fn refresh_backend_data_inner(
    app: &AppHandle,
    request: &BackendRefreshRequest,
) -> Result<Vec<String>, String> {
    super::session::maybe_renew_sessions(app).await;

    let db = app.state::<Database>();
    let session_status = super::session::sync_backend_session_status_for_refresh(app).await?;
    let mut kgc_authenticated = session_status.kgc_session_present;
    let luna_authenticated = session_status.luna_authenticated;
    let mut updated_keys = Vec::new();
    let mut schedule_changed = false;

    // KGC is not kept alive on a timer. When an automatic KGC-backed data task
    // is actually due, allow that run to make one hidden-login attempt.
    if kgc_data_request_due(request, &db) {
        kgc_authenticated = crate::commands::ensure_kgc_session_for_automatic_request(app).await;
    }

    let fast_max_age = super::fast_cache_max_age_secs(super::is_app_focused(app));
    if request.wants("luna_todo")
        && luna_authenticated
        && (request.force || cache_is_stale(&db, "luna_todo", fast_max_age))
    {
        let before = db.cache_payload("luna_todo");
        match crate::luna_commands::luna_fetch_todo(
            app.state::<LunaState>(),
            app.state::<Database>(),
        )
        .await
        {
            Ok(_) if db.cache_payload("luna_todo") != before => {
                updated_keys.push("luna_todo".to_string())
            }
            Ok(_) => {}
            Err(e) => log::warn!("background refresh: luna_todo failed: {}", e),
        }
    }

    if request.wants("weather")
        && (request.force || cache_is_stale(&db, "weather", WEATHER_CACHE_MAX_AGE_SECS))
    {
        match crate::commands::fetch_weather().await {
            Ok(data) => {
                if let Ok(json) = serde_json::to_string(&data) {
                    if db.store_data_cache("weather", &json, true).unwrap_or(false) {
                        updated_keys.push("weather".to_string());
                    }
                }
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
        super::calendar::maybe_auto_sync_calendars(app, &db, schedule_changed, request.force).await;
    }

    if !updated_keys.is_empty() {
        emit_cache_updates(app, updated_keys.clone());
    }
    db.checkpoint_passive();

    Ok(dedup_keys(updated_keys))
}

fn cache_is_stale(db: &Database, key: &str, max_age_secs: i64) -> bool {
    match db.cache_updated_at(key) {
        Some(updated_at) => epoch_secs().saturating_sub(updated_at) >= max_age_secs,
        None => true,
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
