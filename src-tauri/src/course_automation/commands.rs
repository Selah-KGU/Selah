//! SenseA course automation commands.

use super::*;

#[tauri::command]
pub fn course_automation_get(
    db: State<'_, Database>,
    luna_id: String,
    course_name: String,
) -> Result<CourseAutomationView, String> {
    Ok(load_view(&db, &luna_id, &course_name))
}

#[tauri::command]
pub fn course_automation_set_enabled(
    db: State<'_, Database>,
    luna_id: String,
    course_name: String,
    enabled: bool,
) -> Result<CourseAutomationView, String> {
    let mut config = load_config(&db, &luna_id, &course_name);
    config.enabled = enabled;
    if !course_name.trim().is_empty() {
        config.course_name = course_name;
    }
    save_json(&db, &config_key(&luna_id), &config)?;
    Ok(load_view(&db, &luna_id, &config.course_name))
}

#[tauri::command]
pub async fn course_automation_run_now(
    app: AppHandle,
    luna_id: String,
    course_name: String,
) -> Result<CourseAutomationView, String> {
    if !load_config(&app.state::<Database>(), &luna_id, &course_name).enabled {
        return Err("このコースの SenseA を先に有効にしてください".into());
    }
    enqueue_job(
        &app,
        luna_id,
        course_name,
        "manual",
        JobKind::Cycle { force_all: false },
    )
    .await
}

#[tauri::command]
pub async fn course_automation_reanalyze_all(
    app: AppHandle,
    luna_id: String,
    course_name: String,
) -> Result<CourseAutomationView, String> {
    if !load_config(&app.state::<Database>(), &luna_id, &course_name).enabled {
        return Err("このコースの SenseA を先に有効にしてください".into());
    }
    enqueue_job(
        &app,
        luna_id,
        course_name,
        "manual",
        JobKind::Cycle { force_all: true },
    )
    .await
}

#[tauri::command]
pub async fn course_automation_rebuild_memory(
    app: AppHandle,
    luna_id: String,
    course_name: String,
) -> Result<CourseAutomationView, String> {
    if !load_config(&app.state::<Database>(), &luna_id, &course_name).enabled {
        return Err("このコースの SenseA を先に有効にしてください".into());
    }
    enqueue_job(&app, luna_id, course_name, "manual", JobKind::RebuildMemory).await
}

/// Sets (or clears) the user-state of any item by its stable id. Unified across
/// facets — `state` is e.g. "done" / "known"; an empty string clears it.
#[tauri::command]
pub async fn course_automation_set_item_state(
    app: AppHandle,
    luna_id: String,
    course_name: String,
    id: String,
    state: String,
) -> Result<CourseAutomationView, String> {
    let db = app.state::<Database>();
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("項目が指定されていません".into());
    }
    let automation = app.state::<CourseAutomationState>();
    // Share the read side so this never overlaps a full cycle's status rewrite.
    let _shared = automation.cycle_lock.read().await;
    let _write = automation.status_write.lock().await;
    let mut status = load_status(&db, &luna_id, &course_name);
    let state = state.trim().to_string();
    if state.is_empty() {
        status.item_states.remove(&id);
    } else {
        status.item_states.insert(id, state);
    }
    // Keep the TODO page in sync immediately (done → complete its task).
    sync_action_todos(&app, &db, &status);
    save_status_and_emit(&app, &db, &status)?;
    Ok(load_view(&db, &luna_id, &course_name))
}

/// Re-runs the theme filing on demand (the AI grouping over the per-document
/// summaries, heuristic fallback) without waiting for the next cycle. Mutates
/// status directly, guarded against overlapping a running cycle.
#[tauri::command]
pub async fn course_automation_organize_now(
    app: AppHandle,
    luna_id: String,
    course_name: String,
) -> Result<CourseAutomationView, String> {
    let db = app.state::<Database>();
    if !load_config(&db, &luna_id, &course_name).enabled {
        return Err("このコースの SenseA を先に有効にしてください".into());
    }
    let automation = app.state::<CourseAutomationState>();
    let _shared = automation.cycle_lock.read().await;
    let _write = automation.status_write.lock().await;
    let mut status = load_status(&db, &luna_id, &course_name);
    let schedule = course_schedule_text(&db, &course_name);
    let filed = organize_course_documents(&luna_id, &mut status, &schedule, true).await;
    if filed > 0 {
        status.last_organized = Some(epoch_secs());
        push_run_log(
            &mut status,
            "ok",
            format!("{filed}件の資料をテーマ別に整理"),
        );
    }
    save_status_and_emit(&app, &db, &status)?;
    Ok(load_view(&db, &luna_id, &course_name))
}

/// Reverts the most recent auto-organize batch: moves the filed documents back
/// and removes the empty theme folders. Mutates status directly (not a Job),
/// guarded against overlapping a running cycle's status rewrite.
#[tauri::command]
pub async fn course_automation_undo_organize(
    app: AppHandle,
    luna_id: String,
    course_name: String,
) -> Result<CourseAutomationView, String> {
    let db = app.state::<Database>();
    let automation = app.state::<CourseAutomationState>();
    let _shared = automation.cycle_lock.read().await;
    let _write = automation.status_write.lock().await;
    let mut status = load_status(&db, &luna_id, &course_name);
    if status.organize_undo.is_empty() {
        return Ok(load_view(&db, &luna_id, &course_name));
    }
    let course_root = crate::commands::resolve_download_dir(Some(&course_name));
    let restored = organize::undo_organize(&mut status, &course_root);
    if restored > 0 {
        push_run_log(
            &mut status,
            "ok",
            format!("{restored}件の整理を元に戻しました"),
        );
    }
    save_status_and_emit(&app, &db, &status)?;
    Ok(load_view(&db, &luna_id, &course_name))
}

#[tauri::command]
pub async fn course_automation_confirm_print(
    app: AppHandle,
    luna_id: String,
    course_name: String,
    category: String,
) -> Result<CourseAutomationView, String> {
    if !load_config(&app.state::<Database>(), &luna_id, &course_name).enabled {
        return Err("このコースの SenseA を先に有効にしてください".into());
    }
    enqueue_job(
        &app,
        luna_id,
        course_name,
        "manual",
        JobKind::ConfirmPrint { category },
    )
    .await
}

#[tauri::command]
pub async fn course_automation_reanalyze_document(
    app: AppHandle,
    luna_id: String,
    course_name: String,
    document_id: String,
) -> Result<CourseAutomationView, String> {
    if !load_config(&app.state::<Database>(), &luna_id, &course_name).enabled {
        return Err("このコースの SenseA を先に有効にしてください".into());
    }
    enqueue_job(
        &app,
        luna_id,
        course_name,
        "manual",
        JobKind::ReanalyzeDoc { document_id },
    )
    .await
}
