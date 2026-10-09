use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};

use crate::ai;
use crate::background_refresh::BackendSessionStatusPayload;
use crate::db::{epoch_secs, Database};
use crate::timetable;

use super::notify::refresh_notification_analysis;
use super::support::{
    ai_session_block_reason, ai_session_block_reason_from_status, ai_unavailable_reason,
    current_ai_session, emit_cache_updated, is_no_data_error, item_attempted, item_status,
    load_status, record_status, save_status_and_emit, timestamp_is_fresh, update_item_status,
};
use super::types::{
    AiRefreshRequest, AiRefreshState, AiRefreshStatus, AI_NOTIF_CACHE_KEY, AI_REFRESH_CHECK_SECS,
    AI_REFRESH_STARTUP_DELAY_SECS, SCHEDULE_INPUT_MAX_AGE_SECS,
};

pub fn start_ai_refresh_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(AI_REFRESH_STARTUP_DELAY_SECS)).await;
        let mut hidden_streak: u32 = 0;
        loop {
            let visible = crate::background_refresh::is_main_window_visible(&app);
            if !visible && hidden_streak < 6 {
                hidden_streak = hidden_streak.saturating_add(1);
            } else {
                hidden_streak = 0;
                if let Err(err) = run_due_ai_refresh(&app).await {
                    log::warn!("[ai_refresh] scheduled check failed: {}", err);
                }
            }
            tokio::time::sleep(Duration::from_secs(AI_REFRESH_CHECK_SECS)).await;
        }
    });
}

#[tauri::command]
pub async fn get_backend_ai_refresh_status(
    db: crate::db::AccountDb,
    state: State<'_, AiRefreshState>,
) -> Result<AiRefreshStatus, String> {
    let mut status = load_status(&db);
    status.running = state.running.load(Ordering::SeqCst);
    status.interval_minutes = ai::load_ai_config().ai_refresh_interval;
    Ok(status)
}

#[tauri::command]
pub async fn backend_ai_refresh_now(
    app: AppHandle,
    force: bool,
    keys: Option<Vec<String>>,
) -> Result<AiRefreshStatus, String> {
    run_ai_refresh(&app, force, AiRefreshRequest::new(keys), "manual").await
}

async fn run_due_ai_refresh(app: &AppHandle) -> Result<(), String> {
    let config = ai::load_ai_config();
    let db = app.state::<Database>().scope();
    let mut status = load_status(&db);
    status.interval_minutes = config.ai_refresh_interval;

    if config.ai_refresh_interval == 0 {
        save_status_and_emit(app, &db, &status)?;
        return Ok(());
    }

    if let Some(reason) = ai_unavailable_reason(&config) {
        status.running = false;
        status.last_ok = None;
        status.last_error = reason;
        save_status_and_emit(app, &db, &status)?;
        return Ok(());
    }

    if let Some(last_run) = status.last_run {
        let due_after = i64::from(config.ai_refresh_interval) * 60;
        if epoch_secs() - last_run < due_after {
            save_status_and_emit(app, &db, &status)?;
            return Ok(());
        }
    }

    if let Some(reason) = ai_session_block_reason(app).await? {
        status.running = false;
        status.last_ok = None;
        status.last_error = reason;
        save_status_and_emit(app, &db, &status)?;
        return Ok(());
    }

    run_ai_refresh(app, true, AiRefreshRequest::new(None), "scheduled")
        .await
        .map(|_| ())
}

async fn run_ai_refresh(
    app: &AppHandle,
    force: bool,
    request: AiRefreshRequest,
    trigger: &str,
) -> Result<AiRefreshStatus, String> {
    crate::db::account_work(
        crate::db::capture_account(),
        run_ai_refresh_inner(app, force, request, trigger),
    )
    .await
}

async fn run_ai_refresh_inner(
    app: &AppHandle,
    force: bool,
    request: AiRefreshRequest,
    trigger: &str,
) -> Result<AiRefreshStatus, String> {
    let state = app.state::<AiRefreshState>();
    if state.running.swap(true, Ordering::SeqCst) {
        let db = app.state::<Database>().scope();
        let mut status = load_status(&db);
        status.running = true;
        status.interval_minutes = ai::load_ai_config().ai_refresh_interval;
        return Ok(status);
    }

    let db = app.state::<Database>().scope();
    let config = ai::load_ai_config();
    let mut status = load_status(&db);
    let record_scheduler_status = request.is_all();
    status.running = true;
    status.interval_minutes = config.ai_refresh_interval;
    status.last_error.clear();
    status.items.clear();
    if let Err(err) = record_status(app, &db, &status, record_scheduler_status) {
        state.running.store(false, Ordering::SeqCst);
        return Err(err);
    }

    let mut changed_keys: Vec<String> = Vec::new();
    let mut any_error = false;

    let outcome = async {
        if let Some(reason) = ai_unavailable_reason(&config) {
            return Err(reason);
        }
        let session = current_ai_session(app).await?;
        if let Some(reason) = ai_session_block_reason_from_status(&session) {
            return Err(reason);
        }

        log::info!(
            "[ai_refresh] starting {} refresh (force={})",
            trigger,
            force
        );

        if request.wants("ai_notif") {
            status
                .items
                .push(item_status("ai_notif", "AI 通知分析", "running", ""));
            record_status(app, &db, &status, record_scheduler_status)?;
            match refresh_notification_analysis(&db, &config, &session).await {
                Ok(()) => {
                    changed_keys.push(AI_NOTIF_CACHE_KEY.to_string());
                    update_item_status(&mut status, "ai_notif", "done", "");
                }
                Err(err) if is_no_data_error(&err) => {
                    update_item_status(&mut status, "ai_notif", "skipped", &err);
                }
                Err(err) => {
                    any_error = true;
                    update_item_status(&mut status, "ai_notif", "error", &err);
                }
            }
            record_status(app, &db, &status, record_scheduler_status)?;
        }

        // AI 課題分析は定期スケジューラでは実行しない。AI 補助モードの
        // 「再分析」ボタンを押したときだけ ai_analyze_todo が呼ばれる。

        if request.wants("ai_schedule") {
            status
                .items
                .push(item_status("ai_schedule", "AI 時間割分析", "running", ""));
            record_status(app, &db, &status, record_scheduler_status)?;
            match refresh_schedule_analysis(&db, force, &session).await {
                Ok(()) => {
                    changed_keys.push("schedule_data".to_string());
                    update_item_status(&mut status, "ai_schedule", "done", "");
                }
                Err(err) if is_no_data_error(&err) => {
                    update_item_status(&mut status, "ai_schedule", "skipped", &err);
                }
                Err(err) => {
                    any_error = true;
                    update_item_status(&mut status, "ai_schedule", "error", &err);
                }
            }
            record_status(app, &db, &status, record_scheduler_status)?;
        }

        Ok::<(), String>(())
    }
    .await;

    state.running.store(false, Ordering::SeqCst);
    status.running = false;
    let attempted_ai = status.items.iter().any(item_attempted);
    if attempted_ai && request.is_all() {
        status.last_run = Some(epoch_secs());
    }

    if let Err(err) = outcome {
        status.last_ok = Some(false);
        status.last_error = err;
    } else if any_error {
        status.last_ok = Some(false);
        status.last_error = status
            .items
            .iter()
            .find(|item| item.status == "error")
            .map(|item| item.error.clone())
            .unwrap_or_else(|| "AI refresh failed".to_string());
    } else if attempted_ai {
        status.last_ok = Some(true);
        status.last_error.clear();
    } else {
        status.last_ok = None;
        status.last_error = status
            .items
            .iter()
            .find(|item| item.status == "skipped")
            .map(|item| item.error.clone())
            .unwrap_or_default();
    }

    record_status(app, &db, &status, record_scheduler_status)?;
    if !changed_keys.is_empty() {
        emit_cache_updated(app, changed_keys);
    }
    Ok(status)
}

async fn refresh_schedule_analysis(
    db: &Database,
    force: bool,
    session: &BackendSessionStatusPayload,
) -> Result<(), String> {
    if !session.kgc_session_present {
        return Err("KGC未ログインのため時間割AI分析をスキップします".to_string());
    }
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    if scope.current.trim().is_empty() {
        return Err("今学期の時間割がまだありません".to_string());
    }
    if !timestamp_is_fresh(snap.updated_at, SCHEDULE_INPUT_MAX_AGE_SECS) {
        return Err("時間割データが最新ではないためAI分析をスキップします".to_string());
    }
    timetable::ai_generate_schedule_internal(db, scope.current, scope.next, force)
        .await
        .map(|_| ())
}
