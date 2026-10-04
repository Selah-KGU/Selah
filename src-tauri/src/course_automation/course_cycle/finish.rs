use super::*;

pub(super) async fn summarize_and_print(
    app: &AppHandle,
    luna_id: &str,
    db: &Database,
    config: &CourseAutomationConfig,
    previous: &CourseAutomationStatus,
    mut status: &mut CourseAutomationStatus,
    downloaded: Vec<(String, String, String, PathBuf, String)>,
    fingerprint: String,
    source_events: Vec<SourceEvent>,
    pending_source_event_ids: Vec<String>,
    document_analyses: Vec<DocumentAnalysis>,
    newly_analyzed_ids: Vec<String>,
    student: Value,
) -> Result<(), String> {
    let mut pending_summary_ids = previous.pending_summary_ids.to_vec();
    let mut pending_notification_ids = previous.pending_notification_ids.to_vec();
    for id in &newly_analyzed_ids {
        if !pending_summary_ids.iter().any(|existing| existing == id) {
            pending_summary_ids.push(id.clone());
        }
    }
    for analysis in &document_analyses {
        let notification_key = document_notification_key(analysis);
        if analysis.status == "done"
            && analysis.trigger_decision == "immediate"
            && pending_summary_ids.iter().any(|id| id == &analysis.id)
            && !previous
                .notified_document_ids
                .iter()
                .any(|id| id == &notification_key)
            && !pending_notification_ids
                .iter()
                .any(|id| id == &notification_key)
        {
            pending_notification_ids.push(notification_key);
        }
    }
    for event in &source_events {
        if event.attention
            && pending_source_event_ids
                .iter()
                .any(|pending_id| pending_id == &event.id)
            && !previous
                .notified_document_ids
                .iter()
                .any(|id| id == &event.id)
            && !pending_notification_ids.iter().any(|id| id == &event.id)
        {
            pending_notification_ids.push(event.id.clone());
        }
    }
    status.pending_notification_ids = pending_notification_ids;
    let agent_requests_immediate_summary = document_analyses.iter().any(|item| {
        pending_summary_ids.iter().any(|id| id == &item.id) && item.trigger_decision == "immediate"
    });
    let agent_requests_observe_followup = has_observe_followup(
        &document_analyses,
        &pending_summary_ids,
        &newly_analyzed_ids,
    );
    let should_summarize = should_refresh_summary(
        &previous.analysis.summary,
        pending_summary_ids.len(),
        agent_requests_immediate_summary,
        agent_requests_observe_followup,
        pending_source_event_ids.len(),
    );
    let analysis = if should_summarize {
        status.stage = "summarizing".into();
        status.pending_summary_ids = pending_summary_ids.clone();
        status.pending_source_event_ids = pending_source_event_ids.clone();
        save_status_and_emit(app, &db, &status)?;
        let mut new_items = document_analyses
            .iter()
            .filter(|item| pending_summary_ids.iter().any(|id| id == &item.id))
            .cloned()
            .collect::<Vec<_>>();
        prioritize_summary_items(&mut new_items);
        let pending_source_events = source_events
            .iter()
            .filter(|event| pending_source_event_ids.iter().any(|id| id == &event.id))
            .cloned()
            .collect::<Vec<_>>();
        let provider = AgentProvider::resolve().map_err(|error| error.to_string())?;
        let configured_max_tokens = crate::ai::load_ai_config().max_tokens;
        refresh_luna_todo_cache_for_agent(app).await;
        let existing_course_todos = load_existing_course_todos(&db, &status.course_name);
        let mut result = previous.analysis.clone();
        let mut batches = context::summary_batches(&new_items);
        if batches.is_empty() && !pending_source_events.is_empty() {
            batches.push(Vec::new());
        }
        for (batch_index, batch) in batches.into_iter().enumerate() {
            let consumed_ids = batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
            let event_batch = if batch_index == 0 {
                pending_source_events.iter().collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let consumed_event_ids = event_batch
                .iter()
                .map(|event| event.id.clone())
                .collect::<Vec<_>>();
            let (next_result, usage) = summarize_batch_with_agent(
                &provider,
                luna_id,
                &status.course_name,
                &batch,
                &event_batch,
                &student,
                &result,
                &existing_course_todos,
                configured_max_tokens,
                batch_index,
            )
            .await?;
            push_ai_usage(&mut status, usage);
            result = next_result;
            result.print_candidates = merge_print_candidates(
                &previous.analysis.print_candidates,
                result.print_candidates,
            );
            remove_consumed_ids(&mut status.pending_summary_ids, &consumed_ids);
            remove_consumed_ids(&mut status.pending_source_event_ids, &consumed_event_ids);
            status.last_summary_document_ids =
                merge_unique_ids(&status.last_summary_document_ids, consumed_ids);
            status.analysis = result.clone();
            save_status_and_emit(app, &db, &status)?;
        }
        push_run_log(
            &mut status,
            "ok",
            if pending_source_events.is_empty() {
                format!("{} 件の摘要から記憶を更新", new_items.len())
            } else if new_items.is_empty() {
                format!("{} 件の資料変更から記憶を更新", pending_source_events.len())
            } else {
                format!(
                    "{} 件の摘要と {} 件の資料変更から記憶を更新",
                    new_items.len(),
                    pending_source_events.len()
                )
            },
        );
        result
    } else {
        status.pending_summary_ids = pending_summary_ids;
        status.pending_source_event_ids = pending_source_event_ids;
        normalize_course_analysis(
            previous.analysis.clone(),
            &previous.analysis.archived_context,
            &chrono::Local::now().format("%Y-%m-%d").to_string(),
        )
    };

    status.pending_seat_notification = config.notify_seat_changes
        && !analysis.seat.assignment.trim().is_empty()
        && analysis.seat.assignment != status.last_notified_seat_assignment;
    if status.pending_seat_notification {
        let body = format!(
            "{}\n根拠: {}",
            analysis.seat.assignment,
            analysis.seat.evidence.join(" / ")
        );
        match crate::ai::send_native_notification(
            app,
            &format!(
                "{}: 座席情報が更新されました",
                crate::commands::simplify_course_name(&status.course_name)
            ),
            &body,
        ) {
            Ok(_) => {
                status.last_notified_seat_assignment = analysis.seat.assignment.clone();
                status.pending_seat_notification = false;
            }
            Err(error) => {
                log::warn!(
                    "[course_automation] seat notification failed; retrying next run: {}",
                    error
                );
            }
        }
    }

    status.analysis = analysis;
    prune_item_states(&mut status);
    sync_action_todos(app, &db, &status);
    if !status.pending_notification_ids.is_empty() && !status.analysis.summary.trim().is_empty() {
        let pending_ids = status.pending_notification_ids.clone();
        match crate::ai::send_native_notification(
            app,
            &format!(
                "{}: 自動検知からのお知らせ",
                crate::commands::simplify_course_name(&status.course_name)
            ),
            &proactive_notification_body(&status.analysis),
        ) {
            Ok(_) => {
                status.notified_document_ids =
                    merge_unique_ids(&previous.notified_document_ids, pending_ids);
                status.pending_notification_ids.clear();
            }
            Err(error) => {
                log::warn!(
                    "[course_automation] proactive notification failed; retrying next run: {}",
                    error
                );
            }
        }
    }
    status.print_results = if config.auto_print
        && (should_summarize || has_retryable_print_failure(&previous.print_results))
    {
        status.stage = "printing".into();
        save_status_and_emit(app, &db, &status)?;
        let candidates = status.analysis.print_candidates.clone();
        let fresh = process_print_candidates(
            app,
            &db,
            &mut status,
            &downloaded,
            &candidates,
            &config.approved_print_categories,
        )
        .await?;
        // Log only print outcomes that changed this run (file + operation).
        for result in &fresh {
            let unchanged = previous
                .print_results
                .iter()
                .any(|item| print_results_match(item, result) && item.status == result.status);
            if unchanged {
                continue;
            }
            let entry = match result.status.as_str() {
                "printed" => Some(("ok", format!("『{}』を印刷", result.filename))),
                "needs_confirmation" => Some((
                    "warn",
                    format!("『{}』の印刷を保留(確認待ち)", result.filename),
                )),
                "error" | "not_found" => {
                    Some(("error", format!("『{}』の印刷に失敗", result.filename)))
                }
                "unknown" => Some((
                    "warn",
                    format!("『{}』の印刷結果を確認してください", result.filename),
                )),
                _ => None,
            };
            if let Some((level, message)) = entry {
                push_run_log(&mut status, level, message);
            }
        }
        merge_print_results(&status.print_results, fresh)
    } else if should_summarize {
        previous.print_results.clone()
    } else {
        status.stage = if status.pending_summary_ids.is_empty() {
            "unchanged".into()
        } else {
            "pending_summary".into()
        };
        previous.print_results.clone()
    };
    status.fingerprint = fingerprint;
    Ok(())
}
