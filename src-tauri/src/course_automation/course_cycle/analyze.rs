use super::*;

pub(super) async fn analyze_course_documents(
    app: &AppHandle,
    luna_id: &str,
    db: &Database,
    config: &CourseAutomationConfig,
    previous: &CourseAutomationStatus,
    force_all: bool,
    mut status: &mut CourseAutomationStatus,
    downloaded: Vec<(String, String, String, PathBuf, String)>,
    activity_documents: Vec<AnalysisDocument>,
    current_source_infos: Vec<SourceDocumentInfo>,
    deferred_delta_followup: &mut bool,
) -> Result<
    (
        Vec<(String, String, String, PathBuf, String)>,
        Vec<SourceEvent>,
        Vec<String>,
        Vec<DocumentAnalysis>,
        Vec<String>,
        Value,
    ),
    String,
> {
    status.stage = "analyzing".into();
    let analysis_downloads = if force_all {
        downloaded.clone()
    } else {
        downloaded
            .iter()
            .filter(|entry| should_process_downloaded_delta(&previous, entry))
            .cloned()
            .collect()
    };
    let mut documents = activity_documents;
    documents.extend(build_analysis_documents(&analysis_downloads, &previous));
    let current_source_documents = documents.clone();
    let detected_source_events = detect_source_events(
        &previous,
        &current_source_infos,
        &current_source_documents,
        &config,
    );
    let mut pending_source_event_ids = previous.pending_source_event_ids.clone();
    let previous_source_event_ids = previous
        .source_events
        .iter()
        .map(|event| event.id.clone())
        .collect::<HashSet<_>>();
    for event in &detected_source_events {
        if !previous_source_event_ids.contains(&event.id)
            && !pending_source_event_ids
                .iter()
                .any(|existing| existing == &event.id)
        {
            pending_source_event_ids.push(event.id.clone());
        }
    }
    let source_events = merge_source_events(&previous.source_events, detected_source_events);
    status.source_events = source_events.clone();
    status.pending_source_event_ids = pending_source_event_ids.clone();
    // Normal checks are delta-first: only new, changed, or retryable items
    // enter the document loop. Old `analyze_all` config values are kept for
    // backwards-compatible serialization, but a true full sweep now requires
    // the explicit "re-analyze all" command (`force_all`).
    if !force_all {
        documents.retain(|document| should_process_document_delta(&previous, document));
    }
    let student = load_student_profile(&db);
    status.total_documents = documents.len();
    status.processed_documents = 0;
    status.current_document.clear();
    save_status_and_emit(app, &db, &status)?;

    let mut document_analyses = previous.document_analyses.clone();
    let mut newly_analyzed_ids = Vec::new();
    for document in &documents {
        status.current_document = document_label(document);
        save_status_and_emit(app, &db, &status)?;
        let fingerprint = document_fingerprint(document)?;
        let previous_document = previous_document_analysis(&previous, document);
        let mut analysis = if document.load_error == DOC_SKIP_MARKER {
            DocumentAnalysis {
                id: document_id(document),
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
            DocumentAnalysis {
                id: document_id(document),
                fingerprint,
                source_fingerprint: document.source_fingerprint.clone(),
                kind: document.kind.clone(),
                title: document.title.clone(),
                filename: document.filename.clone(),
                path: document.path.clone(),
                status: "error".into(),
                error: document.load_error.clone(),
                ..Default::default()
            }
        } else if previous_document.is_some_and(|item| item.status == "done") {
            migrate_successful_analysis(
                previous_document.expect("checked above"),
                document,
                fingerprint,
            )
        } else {
            match analyze_document_with_agent(luna_id, &status.course_name, document, &student)
                .await
            {
                Ok((analysis, usage)) => {
                    push_ai_usage(&mut status, usage);
                    analysis
                }
                Err(error) => DocumentAnalysis {
                    id: document_id(document),
                    fingerprint,
                    source_fingerprint: document.source_fingerprint.clone(),
                    kind: document.kind.clone(),
                    title: document.title.clone(),
                    filename: document.filename.clone(),
                    path: document.path.clone(),
                    status: "error".into(),
                    error,
                    ..Default::default()
                },
            }
        };
        // Fileless documents (announcements) have no file to re-read later,
        // so keep the analysed text on the record for single re-analysis.
        if document.path.is_empty() && !document.content.is_empty() {
            analysis.content = document.content.clone();
        }
        let doc_label = if analysis.title.trim().is_empty() {
            analysis.filename.clone()
        } else {
            analysis.title.clone()
        };
        if analysis.status == "done" && previous_document.is_none_or(|item| item.status != "done") {
            newly_analyzed_ids.push(analysis.id.clone());
            push_run_log(&mut status, "ok", format!("『{}』を分析", doc_label));
        } else if analysis.status == "error"
            && previous_document.is_none_or(|item| item.status != "error")
        {
            push_run_log(
                &mut status,
                "error",
                format!("『{}』の分析に失敗", doc_label),
            );
        }
        let should_pause_for_immediate =
            should_pause_delta_cycle_after_analysis(force_all, previous_document, &analysis);
        upsert_document_analysis(&mut document_analyses, analysis);
        status.processed_documents += 1;
        if should_pause_for_immediate {
            let remaining = documents.len().saturating_sub(status.processed_documents);
            if remaining > 0 {
                push_run_log(
                    &mut status,
                    "ok",
                    format!("即時対応を優先し、残り {} 件は次回確認します", remaining),
                );
                status.total_documents = status.processed_documents;
                *deferred_delta_followup = true;
            }
        }
        status.document_analyses = document_analyses.clone();
        save_status_and_emit(app, &db, &status)?;
        if should_pause_for_immediate {
            break;
        }
    }
    status.current_document.clear();
    // Per-document failures are already persisted, shown in the run log,
    // and counted by `retryable_failure_count` so the course is retried on
    // the next pass. Do not also fail the whole run here: a single hard-to-
    // read seat PDF would otherwise produce a duplicate global error.
    Ok((
        downloaded,
        source_events,
        pending_source_event_ids,
        document_analyses,
        newly_analyzed_ids,
        student,
    ))
}
