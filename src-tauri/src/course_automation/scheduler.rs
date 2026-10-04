//! SenseA startup loop and scheduled course cycles.

use super::*;

pub fn start_course_automation_loop(app: &AppHandle) {
    let app = app.clone();

    // Crash/restart recovery: a status left with running=true (e.g. the app
    // quit mid-run) would otherwise wedge the UI with every button disabled.
    reset_stale_running_flags(&app);

    // The unified queue dispatcher. It pulls every Job (and therefore every
    // SenseA AI request) and runs it on a bounded pool, so up to
    // JOB_PARALLELISM jobs proceed at once while the rest queue. Per-job safety
    // (cycle exclusivity, document upsert serialisation) is handled in
    // process_job via the state's locks.
    if let Some(mut rx) = app
        .state::<CourseAutomationState>()
        .job_rx
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
    {
        let dispatch_app = app.clone();
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(JOB_PARALLELISM));
        tauri::async_runtime::spawn(async move {
            while let Some(job) = rx.recv().await {
                let Ok(permit) = permits.clone().acquire_owned().await else {
                    break;
                };
                let job_app = dispatch_app.clone();
                tauri::async_runtime::spawn(async move {
                    let _permit = permit;
                    let result = process_job(&job_app, &job).await;
                    if let Some(tx) = job.respond {
                        let _ = tx.send(result);
                    } else if let Err(error) = result {
                        log::warn!(
                            "[course_automation] course '{}' failed: {}",
                            job.course_name,
                            error
                        );
                    }
                });
            }
        });
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;
        let mut hidden_streak: u32 = 0;
        loop {
            let visible = crate::background_refresh::is_main_window_visible(&app);
            if !visible && hidden_streak < 6 {
                hidden_streak = hidden_streak.saturating_add(1);
            } else {
                hidden_streak = 0;
                if let Err(error) = run_due_courses(&app).await {
                    log::warn!("[course_automation] scheduled check failed: {}", error);
                }
            }
            tokio::time::sleep(Duration::from_secs(CHECK_INTERVAL_SECS)).await;
        }
    });
}

/// Clears any persisted `running` flag at startup. A run can only be active
/// after this point via the live queue, so a stored `true` is always stale.
fn reset_stale_running_flags(app: &AppHandle) {
    let db = app.state::<Database>();
    let Ok(rows) = db.list_data_cache_prefix(STATUS_PREFIX) else {
        return;
    };
    for (_, raw, _) in rows {
        let Ok(mut status) = serde_json::from_str::<CourseAutomationStatus>(&raw) else {
            continue;
        };
        let had_running = status.running;
        if status.running {
            status.running = false;
            status.stage = String::new();
        }
        let settled_prints = settle_stale_print_dispatches(&mut status);
        if had_running || settled_prints {
            let _ = save_status_and_emit(app, &db, &status);
        }
    }
}

async fn run_due_courses(app: &AppHandle) -> Result<(), String> {
    // Like every other automatic AI feature, SenseA only runs while logged in.
    // Skip the whole scheduled pass when the Luna session is absent.
    if !luna_is_authenticated(app).await {
        return Ok(());
    }
    let db = app.state::<Database>();
    let configs = db.list_data_cache_prefix(CONFIG_PREFIX)?;
    let now = epoch_secs();
    for (_, raw, _) in configs {
        let Ok(config) = serde_json::from_str::<CourseAutomationConfig>(&raw) else {
            continue;
        };
        if !config.enabled {
            continue;
        }
        let status = load_status(&db, &config.luna_id, &config.course_name);
        if !scheduled_cycle_is_due(status.last_run, config.interval_minutes, now) {
            continue;
        }
        // Hand the due course to the unified queue; the worker logs failures.
        let _ = app.state::<CourseAutomationState>().job_tx.send(Job {
            luna_id: config.luna_id,
            course_name: config.course_name,
            trigger: "scheduled".to_string(),
            kind: JobKind::Cycle { force_all: false },
            respond: None,
        });
    }
    Ok(())
}

pub(super) fn schedule_deferred_delta_followup(app: &AppHandle, luna_id: &str, course_name: &str) {
    let app = app.clone();
    let luna_id = luna_id.to_string();
    let course_name = course_name.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(DEFERRED_DELTA_FOLLOWUP_SECS)).await;
        let job = Job {
            luna_id,
            course_name,
            trigger: "deferred".into(),
            kind: JobKind::Cycle { force_all: false },
            respond: None,
        };
        if let Err(error) = app.state::<CourseAutomationState>().job_tx.send(job) {
            log::warn!(
                "[course_automation] deferred delta follow-up could not be queued: {}",
                error
            );
        }
    });
}

pub(super) fn should_queue_deferred_delta_followup(
    run_succeeded: bool,
    deferred_delta_followup: bool,
) -> bool {
    run_succeeded && deferred_delta_followup
}

pub(super) fn scheduled_cycle_is_due(
    last_run: Option<i64>,
    interval_minutes: u32,
    now: i64,
) -> bool {
    let due_after = i64::from(interval_minutes.max(5)) * 60;
    last_run.is_none_or(|last_run| now.saturating_sub(last_run) >= due_after)
}

/// SenseA's automatic cycles. User-initiated ("manual") jobs and single-document
/// re-analyses are not part of this set: the user explicitly asked for those.
pub(super) fn is_automatic_trigger(trigger: &str) -> bool {
    matches!(trigger, "scheduled" | "deferred")
}

/// SenseA shares the same basic precondition as every other automatic AI feature
/// (see background_refresh's `luna_authenticated` gating): it only runs while the
/// user is logged in to Luna. Automatic cycles are skipped entirely when logged
/// out; manual jobs are left to fail loudly if the session is gone.
pub(super) async fn luna_is_authenticated(app: &AppHandle) -> bool {
    app.state::<crate::LunaState>()
        .client
        .lock()
        .await
        .authenticated
}

pub(super) fn should_skip_automatic_cycle(
    config_enabled: bool,
    trigger: &str,
    last_run: Option<i64>,
    interval_minutes: u32,
    now: i64,
) -> bool {
    if !config_enabled && is_automatic_trigger(trigger) {
        return true;
    }
    trigger == "scheduled" && !scheduled_cycle_is_due(last_run, interval_minutes, now)
}
