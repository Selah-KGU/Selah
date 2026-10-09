//! One SenseA course cycle: download, analyze, summarize, notify, and print.
//!
//! The queue worker and timeout watchdog stay in the parent. This module only
//! runs the inner cycle and asks the parent to schedule a deferred follow-up.

#[path = "course_cycle/analyze.rs"]
mod analyze;
#[path = "course_cycle/download.rs"]
mod download;
#[path = "course_cycle/finish.rs"]
mod finish;

use super::*;

pub(super) async fn run_course_inner(
    app: &AppHandle,
    luna_id: &str,
    course_name_hint: &str,
    trigger: &str,
    force_all: bool,
) -> Result<(), String> {
    let db = app.state::<Database>().scope();
    let config = load_config(&db, luna_id, course_name_hint);
    let mut previous = load_status(&db, luna_id, course_name_hint);
    migrate_legacy_artifacts(&mut previous);
    let run_started_at = epoch_secs();
    let mut status = CourseAutomationStatus {
        luna_id: luna_id.to_string(),
        course_name: course_name_hint.to_string(),
        running: true,
        stage: "checking".into(),
        trigger: trigger.into(),
        ..previous.clone()
    };
    status.last_error.clear();
    save_status_and_emit(app, &db, &status)?;

    let mut deferred_delta_followup = false;
    let outcome: Result<(), String> = async {
        let (downloaded, activity_documents, current_source_infos, fingerprint) =
            download::download_course_sources(
                app,
                luna_id,
                &db,
                &config,
                &previous,
                force_all,
                run_started_at,
                &mut status,
            )
            .await?;
        let (
            downloaded,
            source_events,
            pending_source_event_ids,
            document_analyses,
            newly_analyzed_ids,
            student,
        ) = analyze::analyze_course_documents(
            app,
            luna_id,
            &db,
            &config,
            &previous,
            force_all,
            &mut status,
            downloaded,
            activity_documents,
            current_source_infos,
            &mut deferred_delta_followup,
        )
        .await?;
        finish::summarize_and_print(
            app,
            luna_id,
            &db,
            &config,
            &previous,
            &mut status,
            downloaded,
            fingerprint,
            source_events,
            pending_source_event_ids,
            document_analyses,
            newly_analyzed_ids,
            student,
        )
        .await?;
        Ok(())
    }
    .await;

    let retryable_failure_count = retryable_failure_count(&status);
    let analysis_failure_count = analysis_failure_count(&status);
    status.running = false;
    status.last_run = Some(epoch_secs());
    status.last_ok = Some(outcome.is_ok() && retryable_failure_count == 0);
    // Print failures are surfaced on the 印刷 capsule (via print_results), not the
    // control capsule, so a successful run with only print failures is not an
    // error here — it does not touch last_error or the error stage.
    status.stage = final_run_stage(
        status.stage.as_str(),
        outcome.is_ok(),
        analysis_failure_count,
    );
    if let Err(error) = &outcome {
        status.last_error = error.clone();
        // Per-operation entries are logged inline during the run; here we add a
        // single run-level entry only when the whole run failed.
        push_run_log(&mut status, "error", error.clone());
    } else if analysis_failure_count > 0 {
        status.last_error = analysis_failure_summary(analysis_failure_count);
    }
    // After a run that actually changed something, file the documents into theme
    // folders (AI grouping over the per-document summaries, heuristic fallback).
    // Gated on a real change so steady "unchanged" cycles cost no extra request.
    if outcome.is_ok() && status.stage == "done" {
        let schedule = course_schedule_text(&db, &status.course_name);
        let filed = organize_course_documents(luna_id, &mut status, &schedule, false).await;
        if filed > 0 {
            status.last_organized = Some(epoch_secs());
            push_run_log(
                &mut status,
                "ok",
                format!("{filed}件の資料をテーマ別に整理"),
            );
        }
    }
    save_status_and_emit(app, &db, &status)?;
    if should_queue_deferred_delta_followup(outcome.is_ok(), deferred_delta_followup) {
        schedule_deferred_delta_followup(app, luna_id, &status.course_name);
    }
    outcome
}
