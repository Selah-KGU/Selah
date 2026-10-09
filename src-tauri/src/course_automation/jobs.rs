//! Queued SenseA jobs and the workers they run.

use super::*;

/// Runs one queued Job to completion. Each branch carries a watchdog so a hung
/// AI request can never tie up a worker slot (and thus stall the queue).
pub(super) async fn process_job(
    app: &AppHandle,
    job: &Job,
) -> Result<CourseAutomationView, String> {
    let state = app.state::<CourseAutomationState>();
    let db = app.state::<Database>().scope();
    match &job.kind {
        JobKind::Cycle { force_all } => {
            // A full cycle rewrites the whole status from a snapshot, so it runs
            // exclusively (write side): no document job writes underneath it.
            let _exclusive = state.cycle_lock.write().await;
            // Re-check automatic jobs after acquiring exclusivity. A scheduled
            // or deferred job may have waited behind another cycle while the
            // user disabled SenseA for this course or another cycle satisfied
            // the scheduled interval.
            let config = load_config(&db, &job.luna_id, &job.course_name);
            let status = load_status(&db, &job.luna_id, &job.course_name);
            if should_skip_automatic_cycle(
                config.enabled,
                &job.trigger,
                status.last_run,
                config.interval_minutes,
                epoch_secs(),
            ) {
                return Ok(CourseAutomationView { config, status });
            }
            // The session can drop between enqueue and execution (notably the
            // deferred follow-up's delay), so re-check login for automatic jobs.
            if is_automatic_trigger(&job.trigger) && !luna_is_authenticated(app).await {
                return Ok(CourseAutomationView { config, status });
            }
            // run_course has its own timeout + status cleanup.
            run_course(
                app,
                &job.luna_id,
                &job.course_name,
                &job.trigger,
                *force_all,
            )
            .await?;
            Ok(load_view(&db, &job.luna_id, &job.course_name))
        }
        JobKind::ReanalyzeDoc { document_id } => {
            // Document jobs share the read side, so several re-analyse in
            // parallel, but none overlaps a full cycle.
            let _shared = state.cycle_lock.read().await;
            match tokio::time::timeout(
                Duration::from_secs(RUN_TIMEOUT_SECS),
                reanalyze_one_document(app, &job.luna_id, &job.course_name, document_id),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err("再分析がタイムアウトしました".into()),
            }
        }
        JobKind::RebuildMemory => {
            // Rebuilding rewrites the whole working memory from the existing
            // per-document analyses, so it runs exclusively like a cycle.
            let _exclusive = state.cycle_lock.write().await;
            match tokio::time::timeout(
                Duration::from_secs(RUN_TIMEOUT_SECS),
                rebuild_memory(app, &job.luna_id, &job.course_name),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err("記憶の再構築がタイムアウトしました".into()),
            }
        }
        JobKind::ConfirmPrint { category } => {
            // Printing a pending file is independent of cycles; share the read
            // side so it never overlaps a full cycle's status rewrite.
            let _shared = state.cycle_lock.read().await;
            confirm_print_category(app, &job.luna_id, &job.course_name, category).await
        }
    }
}

/// Enqueues a job and awaits its result — used by the user-triggered commands.
pub(super) async fn enqueue_job(
    app: &AppHandle,
    luna_id: String,
    course_name: String,
    trigger: &str,
    kind: JobKind,
) -> Result<CourseAutomationView, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let job = Job {
        account: crate::db::capture_account(),
        luna_id,
        course_name,
        trigger: trigger.to_string(),
        kind,
        respond: Some(tx),
    };
    app.state::<CourseAutomationState>()
        .job_tx
        .send(job)
        .map_err(|_| "処理キューに送信できません".to_string())?;
    rx.await.map_err(|_| "処理が中断されました".to_string())?
}

/// Re-analyses a single already-downloaded document. Runs as a queue Job, so it
/// is naturally serialised with every other SenseA operation — no extra locks.
async fn reanalyze_one_document(
    app: &AppHandle,
    luna_id: &str,
    course_name: &str,
    document_id: &str,
) -> Result<CourseAutomationView, String> {
    let db = app.state::<Database>().scope();
    let status_snapshot = load_status(&db, luna_id, course_name);
    let existing = status_snapshot
        .document_analyses
        .iter()
        .find(|item| item.id == document_id)
        .cloned()
        .ok_or_else(|| "対象の資料が見つかりません".to_string())?;

    // Rebuild the document for re-analysis. File-backed documents are re-read
    // from disk (picking up any change); fileless ones (announcements) reuse
    // the text captured when they were first analysed.
    let mut document = AnalysisDocument {
        kind: existing.kind.clone(),
        title: existing.title.clone(),
        filename: existing.filename.clone(),
        path: existing.path.clone(),
        content: String::new(),
        source_fingerprint: existing.source_fingerprint.clone(),
        load_error: String::new(),
        images: Vec::new(),
    };
    if existing.path.is_empty() {
        if existing.content.is_empty() {
            return Err(
                "この資料は本文が保存されていないため、「全て再分析」で再取得してください".into(),
            );
        }
        document.content = existing.content.clone();
    } else {
        let path = PathBuf::from(&existing.path);
        match crate::agent_tools::read_downloaded_text(&path) {
            Ok(text) => document.content = truncate_chars(&text, MAX_FILE_TEXT_CHARS),
            Err(error) => load_document_images_or_error(&path, &mut document, &error),
        }
    }

    let fingerprint = document_fingerprint(&document)?;
    let mut ai_usage = None;
    let mut analysis = if document.load_error == DOC_SKIP_MARKER {
        DocumentAnalysis {
            id: document_id.to_string(),
            fingerprint,
            source_fingerprint: document.source_fingerprint.clone(),
            kind: document.kind.clone(),
            title: document.title.clone(),
            filename: document.filename.clone(),
            path: document.path.clone(),
            status: "skipped".into(),
            error: "本文・画像とも抽出できないためスキップしました".into(),
            ..Default::default()
        }
    } else if !document.load_error.is_empty() {
        return Err(document.load_error);
    } else {
        let student = load_student_profile(&db);
        let (analysis, usage) =
            analyze_document_with_agent(luna_id, course_name, &document, &student).await?;
        ai_usage = Some(usage);
        analysis
    };
    // Preserve the text on the record so fileless docs stay re-analysable.
    if existing.path.is_empty() && !document.content.is_empty() {
        analysis.content = document.content.clone();
    }
    let was_done = analysis.status == "done";

    // Brief critical section (serialised across concurrent document jobs):
    // reload the latest status, upsert this one document, queue it for the next
    // synthesis. The 分析 card reflects the new per-document summary at once;
    // the overall synthesis refreshes on the next cycle.
    let state = app.state::<CourseAutomationState>();
    let _write = state.status_write.lock().await;
    let mut status = load_status(&db, luna_id, course_name);
    if let Some(usage) = ai_usage {
        push_ai_usage(&mut status, usage);
    }
    upsert_document_analysis(&mut status.document_analyses, analysis);
    if was_done
        && !status
            .pending_summary_ids
            .iter()
            .any(|id| id == document_id)
    {
        status.pending_summary_ids.push(document_id.to_string());
    }
    save_status_and_emit(app, &db, &status)?;

    Ok(load_view(&db, luna_id, course_name))
}

/// Rebuilds the whole working memory (summary / findings / 記憶 / seat / print)
/// from a clean slate, re-running the synthesis over every already-analysed
/// document. Re-uses the stored per-document analyses — no re-download or
/// per-document AI — so it only re-derives the consolidated memory. Used when
/// the accumulated memory has drifted and the user wants it reconstructed.
async fn rebuild_memory(
    app: &AppHandle,
    luna_id: &str,
    course_name: &str,
) -> Result<CourseAutomationView, String> {
    let db = app.state::<Database>().scope();
    let status_snapshot = load_status(&db, luna_id, course_name);
    let analysed = status_snapshot
        .document_analyses
        .iter()
        .filter(|item| item.status == "done")
        .cloned()
        .collect::<Vec<_>>();
    if analysed.is_empty() {
        return Err("分析済みの資料がありません。先に確認を実行してください".into());
    }

    let student = load_student_profile(&db);
    refresh_luna_todo_cache_for_agent(app).await;
    let existing_course_todos = load_existing_course_todos(&db, &status_snapshot.course_name);
    // Clean slate: no previous analysis, so the memory is derived purely from
    // the documents themselves rather than carried forward.
    let analysis = summarize_with_agent(
        luna_id,
        &status_snapshot.course_name,
        &analysed,
        &student,
        &existing_course_todos,
        &AgentCourseAnalysis::default(),
    )
    .await?;

    let analysed_ids = analysed.iter().map(|item| item.id.clone()).collect();

    let state = app.state::<CourseAutomationState>();
    let _write = state.status_write.lock().await;
    // Reload to fold the rebuilt memory onto the latest persisted status without
    // clobbering anything a concurrent change touched.
    let mut status = load_status(&db, luna_id, course_name);
    status.analysis = analysis;
    prune_item_states(&mut status);
    status.pending_summary_ids.clear();
    status.last_summary_document_ids = analysed_ids;
    sync_action_todos(app, &db, &status);
    save_status_and_emit(app, &db, &status)?;

    Ok(load_view(&db, luna_id, course_name))
}

/// Approves a print category and prints the files currently waiting under it.
/// Approval is remembered in the config, so subsequent same-category files
/// print automatically without asking again.
async fn confirm_print_category(
    app: &AppHandle,
    luna_id: &str,
    course_name: &str,
    category: &str,
) -> Result<CourseAutomationView, String> {
    let db = app.state::<Database>().scope();
    let category = category.trim().to_string();
    if category.is_empty() {
        return Err("印刷タイプが指定されていません".into());
    }

    // Remember the approval for future runs.
    let mut config = load_config(&db, luna_id, course_name);
    if !config
        .approved_print_categories
        .iter()
        .any(|item| normalize_category(item) == normalize_category(&category))
    {
        config.approved_print_categories.push(category.clone());
        save_json(&db, &config_key(luna_id), &config)?;
    }

    // Print every file currently waiting under this category. The status lock is
    // held across the print so dispatch/final states stay serial and durable.
    let state = app.state::<CourseAutomationState>();
    let _write = state.status_write.lock().await;
    let mut status = load_status(&db, luna_id, course_name);
    let pending: Vec<PrintResult> = status
        .print_results
        .iter()
        .filter(|item| {
            item.status == "needs_confirmation"
                && normalize_category(&item.category) == normalize_category(&category)
        })
        .cloned()
        .collect();
    if pending.is_empty() {
        return Ok(load_view(&db, luna_id, course_name));
    }
    let mut printed = Vec::new();
    for item in pending {
        let path = PathBuf::from(&item.path);
        let action_key = print_result_action_key(&item);
        let dispatching = dispatching_print_result(
            &item.filename,
            item.path.clone(),
            item.category.clone(),
            action_key.clone(),
        );
        status.stage = "printing".into();
        status.print_results = merge_print_results(&status.print_results, vec![dispatching]);
        save_status_and_emit(app, &db, &status)?;
        let result = print_one(
            &item.filename,
            &path,
            item.path.clone(),
            item.category,
            action_key,
        )
        .await;
        status.print_results = merge_print_results(&status.print_results, vec![result.clone()]);
        save_status_and_emit(app, &db, &status)?;
        printed.push(result);
    }
    status.print_results = merge_print_results(&status.print_results, printed);
    status.stage = "done".into();
    save_status_and_emit(app, &db, &status)?;
    Ok(load_view(&db, luna_id, course_name))
}

async fn run_course(
    app: &AppHandle,
    luna_id: &str,
    course_name_hint: &str,
    trigger: &str,
    force_all: bool,
) -> Result<(), String> {
    // Exclusivity is provided by the single queue worker; here we only keep a
    // watchdog so a hung run can't wedge the queue.
    let mut timed_out = false;
    let outcome = match tokio::time::timeout(
        Duration::from_secs(RUN_TIMEOUT_SECS),
        run_course_inner(app, luna_id, course_name_hint, trigger, force_all),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            timed_out = true;
            Err("実行がタイムアウトしました・次回再試行します".to_string())
        }
    };
    // Any failure (timeout OR an early error return from the inner run) must
    // clear the running flag — otherwise the persisted status stays running and
    // the UI wedges with every button disabled.
    if let Err(error) = &outcome {
        let db = app.state::<Database>().scope();
        let mut status = load_status(&db, luna_id, course_name_hint);
        status.running = false;
        status.last_run = Some(epoch_secs());
        status.last_ok = Some(false);
        status.stage = "error".into();
        status.last_error = error.clone();
        settle_stale_print_dispatches(&mut status);
        // On timeout the inner run was cancelled before it could log, so record
        // it here. Inner errors are already logged by run_course_inner.
        if timed_out {
            push_run_log(&mut status, "error", error.clone());
        }
        let _ = save_status_and_emit(app, &db, &status);
    }
    outcome
}
