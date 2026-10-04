//! Source events, print flow, and run-status tests.

use super::*;

#[test]
fn source_events_distinguish_changed_removed_and_transient_missing_download() {
    let config = CourseAutomationConfig::new("course".into(), "Course".into());
    let old_doc = AnalysisDocument {
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "worksheet.pdf".into(),
        path: "/tmp/worksheet-v1.pdf".into(),
        content: "old".into(),
        source_fingerprint: "remote-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let removed_doc = AnalysisDocument {
        kind: "material".into(),
        title: "Week 2".into(),
        filename: "removed.pdf".into(),
        path: "/tmp/removed.pdf".into(),
        content: "removed".into(),
        source_fingerprint: "removed-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let previous = CourseAutomationStatus {
        document_analyses: vec![
            DocumentAnalysis {
                id: document_id(&old_doc),
                kind: old_doc.kind.clone(),
                title: old_doc.title.clone(),
                filename: old_doc.filename.clone(),
                source_fingerprint: old_doc.source_fingerprint.clone(),
                status: "done".into(),
                trigger_decision: "observe".into(),
                summary: "old worksheet summary".into(),
                ..Default::default()
            },
            DocumentAnalysis {
                id: document_id(&removed_doc),
                kind: removed_doc.kind.clone(),
                title: removed_doc.title.clone(),
                filename: removed_doc.filename.clone(),
                source_fingerprint: removed_doc.source_fingerprint.clone(),
                status: "done".into(),
                print_instruction: "印刷して記入".into(),
                summary: "removed worksheet summary".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let new_doc = AnalysisDocument {
        source_fingerprint: "remote-v2".into(),
        path: "/tmp/worksheet-v2.pdf".into(),
        content: "new".into(),
        ..old_doc.clone()
    };
    let events = detect_source_events(
        &previous,
        &[SourceDocumentInfo {
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "worksheet.pdf".into(),
            source_fingerprint: "remote-v2".into(),
        }],
        &[new_doc],
        &config,
    );

    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .any(|event| event.event == "changed"
                && event.previous_summary == "old worksheet summary")
    );
    assert!(events
        .iter()
        .any(|event| event.event == "removed" && event.attention));

    let transient = detect_source_events(
        &previous,
        &[SourceDocumentInfo {
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "worksheet.pdf".into(),
            source_fingerprint: "remote-v1".into(),
        }],
        &[],
        &config,
    );
    assert!(transient
        .iter()
        .all(|event| !(event.event == "removed" && event.filename == "worksheet.pdf")));

    let changed_without_document = detect_source_events(
        &previous,
        &[SourceDocumentInfo {
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "worksheet.pdf".into(),
            source_fingerprint: "remote-v2".into(),
        }],
        &[],
        &config,
    );
    assert!(changed_without_document
        .iter()
        .any(|event| event.event == "changed" && event.filename == "worksheet.pdf"));
}

#[test]
fn source_events_do_not_treat_legacy_identity_migration_as_change() {
    let config = CourseAutomationConfig::new("course".into(), "Course".into());
    let previous = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: "legacy-id".into(),
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "notes.pdf".into(),
            status: "done".into(),
            summary: "legacy summary".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let current = AnalysisDocument {
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: "/tmp/notes.pdf".into(),
        content: "same logical source".into(),
        source_fingerprint: "remote-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };

    let events = detect_source_events(
        &previous,
        &[SourceDocumentInfo {
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "notes.pdf".into(),
            source_fingerprint: "remote-v1".into(),
        }],
        &[current],
        &config,
    );

    assert!(events.is_empty());
}

#[test]
fn detail_error_source_presence_does_not_emit_changed_or_removed_event() {
    let config = CourseAutomationConfig::new("course".into(), "Course".into());
    let previous = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: "announcement-id".into(),
            kind: "announcement".into(),
            title: "LUNA告知".into(),
            filename: String::new(),
            source_fingerprint: "detail-v1".into(),
            status: "done".into(),
            summary: "previous detail".into(),
            trigger_decision: "observe".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let detail_error = json!({
        "status": "detail_error",
        "kind": "announcement",
        "title": "LUNA告知",
        "detail_path": "/lms/coursetop/information/listdetail?idnumber=6072&informationId=A1",
        "source_fingerprint": "list-only-v2",
    });

    let events = detect_source_events(
        &previous,
        &[SourceDocumentInfo {
            kind: "announcement".into(),
            title: "LUNA告知".into(),
            filename: String::new(),
            source_fingerprint: activity_source_fingerprint_for_source_info(&detail_error),
        }],
        &[],
        &config,
    );

    assert!(events.is_empty());
}

#[test]
fn normalizes_agent_trigger_decision() {
    assert_eq!(normalize_trigger_decision(" immediate "), "immediate");
    assert_eq!(normalize_trigger_decision("OBSERVE"), "observe");
    assert_eq!(normalize_trigger_decision("anything else"), "routine");
}

#[test]
fn plus_retry_policy_avoids_likely_duplicate_generations() {
    assert!(!plus_error_allows_retry(
        "AI応答の受信が途中で中断されました。重複生成を避けるため自動再試行しません"
    ));
    assert!(plus_error_allows_retry("リクエスト失敗: connection reset"));
}

#[test]
fn printed_results_are_persistent_and_only_failures_are_retryable() {
    let printed = PrintResult {
        action_key: "print-v1".into(),
        filename: "handout.pdf".into(),
        path: "/tmp/handout.pdf".into(),
        status: "printed".into(),
        detail: "accepted".into(),
        ..Default::default()
    };
    let merged = merge_print_results(
        std::slice::from_ref(&printed),
        vec![PrintResult {
            action_key: "print-v1".into(),
            filename: "handout.pdf".into(),
            path: "/tmp/handout.pdf".into(),
            status: "error".into(),
            detail: "later error".into(),
            ..Default::default()
        }],
    );
    assert_eq!(merged, vec![printed]);
    assert!(!has_retryable_print_failure(&merged));
    assert!(has_retryable_print_failure(&[PrintResult {
        status: "error".into(),
        ..Default::default()
    }]));
    assert!(!has_retryable_print_failure(&[PrintResult {
        status: "skipped_low_confidence".into(),
        ..Default::default()
    }]));
    assert!(!has_retryable_print_failure(&[PrintResult {
        status: "unknown".into(),
        ..Default::default()
    }]));
}

#[test]
fn print_candidates_persist_across_new_summaries() {
    let merged = merge_print_candidates(
        &[PrintCandidate {
            filename: "old.pdf".into(),
            reason: "persist".into(),
            confidence: 0.9,
            ..Default::default()
        }],
        vec![PrintCandidate {
            filename: "new.pdf".into(),
            reason: "new".into(),
            confidence: 0.95,
            ..Default::default()
        }],
    );
    assert_eq!(merged.len(), 2);
    assert!(merged.iter().any(|item| item.filename == "old.pdf"));
    assert!(merged.iter().any(|item| item.filename == "new.pdf"));
}

#[test]
fn print_category_approval_uses_normalized_category() {
    assert_eq!(
        normalize_category(" ワークシート   小テスト "),
        normalize_category("ワークシート 小テスト")
    );
    assert!(["worksheet".to_string()]
        .iter()
        .any(|item| normalize_category(item) == normalize_category("WORKSHEET")));
}

#[test]
fn organize_signature_is_id_based_and_path_independent() {
    use std::collections::BTreeMap;
    let candidate = |path: &str| organize::OrganizeCandidate {
        path: path.to_string(),
        filename: "a.pdf".into(),
        title: "A".into(),
        summary: String::new(),
        kind: "material".into(),
    };
    let mut set: BTreeMap<String, organize::OrganizeCandidate> = BTreeMap::new();
    set.insert("doc1".into(), candidate("/course/a.pdf"));
    set.insert("doc2".into(), candidate("/course/b.pdf"));
    let before = organize_candidate_signature(&set);

    // Filing moves a file into a theme folder: its path changes but its id
    // does not, so the signature must stay the same (no redundant re-plan).
    set.insert("doc1".into(), candidate("/course/第1回/a.pdf"));
    assert_eq!(organize_candidate_signature(&set), before);

    // Adding a new file (e.g. a fresh download or loose note) must flip it.
    set.insert("loose:note.md".into(), candidate("/course/note.md"));
    let after_add = organize_candidate_signature(&set);
    assert_ne!(after_add, before);

    // Removing it returns to the original signature (set is what matters).
    set.remove("loose:note.md");
    assert_eq!(organize_candidate_signature(&set), before);
}

#[test]
fn print_action_key_follows_source_version() {
    let first = print_action_key(
        "worksheet.pdf",
        "/tmp/worksheet.pdf",
        "remote-v1",
        "課題用紙",
    );
    let same = print_action_key(
        "worksheet.pdf",
        "/elsewhere/worksheet.pdf",
        "remote-v1",
        " 課題用紙 ",
    );
    let revised = print_action_key(
        "worksheet.pdf",
        "/tmp/worksheet.pdf",
        "remote-v2",
        "課題用紙",
    );

    assert_eq!(first, same);
    assert_ne!(first, revised);
}

#[test]
fn print_candidate_can_resolve_from_artifact_ledger_without_current_download() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"worksheet").expect("write cached file");
    let artifacts = vec![CourseArtifactRecord {
        kind: "material".into(),
        filename: "worksheet.pdf".into(),
        path: path.to_string_lossy().to_string(),
        status: "downloaded".into(),
        source_fingerprint: "remote-v1".into(),
        ..Default::default()
    }];

    let located = locate_print_candidate(&[], &artifacts, "worksheet.pdf")
        .expect("print candidate should resolve from persisted artifact ledger");

    assert_eq!(located.0, "worksheet.pdf");
    assert_eq!(located.1, path);
    assert_eq!(located.2, "remote-v1");
    let _ = std::fs::remove_file(path);
}

#[test]
fn print_merge_uses_action_key_before_filename_fallback() {
    let first = PrintResult {
        action_key: "source-v1".into(),
        filename: "worksheet.pdf".into(),
        path: "/tmp/worksheet.pdf".into(),
        status: "printed".into(),
        ..Default::default()
    };
    let revised = PrintResult {
        action_key: "source-v2".into(),
        filename: "worksheet.pdf".into(),
        path: "/tmp/worksheet.pdf".into(),
        status: "needs_confirmation".into(),
        ..Default::default()
    };

    let merged = merge_print_results(std::slice::from_ref(&first), vec![revised.clone()]);

    assert_eq!(merged, vec![first, revised]);
}

#[test]
fn stale_dispatching_print_becomes_unknown_without_auto_retry() {
    let mut status = CourseAutomationStatus {
        print_results: vec![PrintResult {
            action_key: "print-action".into(),
            filename: "worksheet.pdf".into(),
            status: "dispatching".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(settle_stale_print_dispatches(&mut status));
    assert_eq!(status.print_results[0].status, "unknown");
    assert!(!has_retryable_print_failure(&status.print_results));
}

#[test]
fn ai_usage_log_is_bounded() {
    let mut status = CourseAutomationStatus::default();
    for index in 0..45 {
        push_ai_usage(
            &mut status,
            AiUsageEstimate {
                label: format!("request-{index}"),
                ..Default::default()
            },
        );
    }

    assert_eq!(status.ai_usage.len(), 40);
    assert_eq!(status.ai_usage[0].label, "request-5");
}

#[test]
fn retryable_failure_count_includes_only_current_failures() {
    let status = CourseAutomationStatus {
        artifacts: vec![
            CourseArtifactRecord {
                status: "downloaded".into(),
                ..Default::default()
            },
            CourseArtifactRecord {
                status: "error".into(),
                ..Default::default()
            },
        ],
        document_analyses: vec![
            DocumentAnalysis {
                status: "done".into(),
                ..Default::default()
            },
            DocumentAnalysis {
                status: "error".into(),
                ..Default::default()
            },
        ],
        print_results: vec![
            PrintResult {
                status: "printed".into(),
                ..Default::default()
            },
            PrintResult {
                status: "not_found".into(),
                ..Default::default()
            },
        ],
        pending_notification_ids: vec!["notify-retry".into()],
        pending_seat_notification: true,
        ..Default::default()
    };

    assert_eq!(retryable_failure_count(&status), 5);
    assert_eq!(analysis_failure_count(&status), 2);
    assert_eq!(
        analysis_failure_summary(1),
        "一部の資料の分析に失敗しました"
    );
    assert_eq!(analysis_failure_summary(2), "2件の資料の分析に失敗しました");
}

#[test]
fn final_run_stage_distinguishes_partial_analysis_failure() {
    assert_eq!(final_run_stage("analyzing", true, 1), "partial_error");
    assert_eq!(final_run_stage("printing", true, 0), "done");
    assert_eq!(
        final_run_stage("pending_summary", true, 0),
        "pending_summary"
    );
    assert_eq!(final_run_stage("unchanged", true, 0), "unchanged");
    assert_eq!(final_run_stage("summarizing", false, 0), "error");
}

#[test]
fn migrate_legacy_status_errors_surfaces_partial_analysis_failure() {
    let mut status = CourseAutomationStatus {
        stage: "done".into(),
        last_ok: Some(false),
        last_error: LEGACY_ALL_DOCUMENTS_FAILED_ERROR.into(),
        document_analyses: vec![DocumentAnalysis {
            title: "第12回　社会言語学基礎：座席表".into(),
            status: "error".into(),
            error: "分析に失敗".into(),
            ..Default::default()
        }],
        run_log: vec![
            RunLogEntry {
                at: 1,
                level: "error".into(),
                message: LEGACY_ALL_DOCUMENTS_FAILED_ERROR.into(),
            },
            RunLogEntry {
                at: 2,
                level: "error".into(),
                message: "『第12回　社会言語学基礎：座席表』の分析に失敗".into(),
            },
        ],
        ..Default::default()
    };

    migrate_legacy_status_errors(&mut status);

    assert_eq!(status.stage, "partial_error");
    assert_eq!(status.last_ok, Some(false));
    assert_eq!(status.last_error, "一部の資料の分析に失敗しました");
    assert_eq!(status.run_log.len(), 1);
    assert_eq!(
        status.run_log[0].message,
        "『第12回　社会言語学基礎：座席表』の分析に失敗"
    );
}

#[test]
fn migrate_legacy_status_errors_normalizes_old_timeout_text() {
    let mut status = CourseAutomationStatus {
        stage: "error".into(),
        last_ok: Some(false),
        last_error: LEGACY_AI_TIMEOUT_180_ERROR.into(),
        run_log: vec![RunLogEntry {
            at: 1,
            level: "error".into(),
            message: LEGACY_AI_TIMEOUT_180_ERROR.into(),
        }],
        ..Default::default()
    };

    migrate_legacy_status_errors(&mut status);

    assert_eq!(status.stage, "error");
    assert_eq!(status.last_error, LEGACY_AI_TIMEOUT_ERROR);
    assert_eq!(status.run_log[0].message, LEGACY_AI_TIMEOUT_ERROR);
}

#[test]
fn proactive_notification_identity_changes_with_document_version() {
    let mut analysis = DocumentAnalysis {
        id: "announcement".into(),
        fingerprint: "v1".into(),
        ..Default::default()
    };
    assert_eq!(
        document_notification_key(&analysis),
        "announcement:v1".to_string()
    );

    analysis.fingerprint = "v2".into();
    assert_eq!(
        document_notification_key(&analysis),
        "announcement:v2".to_string()
    );
}

#[test]
fn proactive_notification_body_is_compact_and_actionable() {
    let body = proactive_notification_body(&AgentCourseAnalysis {
        summary: "Act now".into(),
        findings: ["First", "Second", "Third", "Must not appear"]
            .into_iter()
            .map(|text| Item {
                text: text.to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    });

    assert!(body.starts_with("Act now\n・First"));
    assert!(body.contains("・Third"));
    assert!(!body.contains("Must not appear"));
    assert!(body.chars().count() <= 600);
}
