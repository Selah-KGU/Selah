//! Artifact ledger, summary input, and course memory tests.

use super::*;

#[test]
fn previous_document_analysis_matches_stable_identity() {
    let document = AnalysisDocument {
        kind: "announcement".into(),
        title: "座席変更".into(),
        filename: String::new(),
        path: String::new(),
        content: "new content".into(),
        source_fingerprint: "announcement-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let prior = DocumentAnalysis {
        id: document_id(&document),
        status: "done".into(),
        summary: "old summary".into(),
        ..Default::default()
    };
    let status = CourseAutomationStatus {
        document_analyses: vec![prior],
        ..Default::default()
    };
    assert_eq!(
        previous_document_analysis(&status, &document).map(|item| item.summary.as_str()),
        Some("old summary")
    );
}

#[test]
fn unchanged_remote_source_reuses_existing_download() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"cached").expect("write cached file");
    let artifacts = vec![CourseArtifactRecord {
        id: artifact_id("material", "notes.pdf"),
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: path.to_string_lossy().to_string(),
        status: "downloaded".into(),
        source_fingerprint: "remote-v1".into(),
        ..Default::default()
    }];

    assert_eq!(
        reusable_artifact_path(&artifacts, "material", "Week 1", "notes.pdf", "remote-v1"),
        Some((path.clone(), "remote-v1".into()))
    );
    assert!(
        reusable_artifact_path(&artifacts, "material", "Week 1", "notes.pdf", "remote-v2")
            .is_none()
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_analysis_migrates_to_persistent_artifact_ledger() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"cached").expect("write cached file");
    let mut status = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "notes.pdf".into(),
            path: path.to_string_lossy().to_string(),
            status: "done".into(),
            source_fingerprint: "remote-v1".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    migrate_legacy_artifacts(&mut status);
    assert_eq!(status.artifacts.len(), 1);
    assert_eq!(status.artifacts[0].status, "downloaded");
    assert_eq!(status.artifacts[0].path, path.to_string_lossy());
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_downloaded_file_path_migrates_and_is_reused() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"cached").expect("write cached file");
    let mut status = CourseAutomationStatus {
        downloaded_files: vec![path.to_string_lossy().to_string()],
        ..Default::default()
    };

    migrate_legacy_artifacts(&mut status);

    assert_eq!(status.artifacts.len(), 1);
    assert_eq!(status.artifacts[0].kind, "legacy");
    assert_eq!(
        reusable_artifact_path(
            &status.artifacts,
            "material",
            "Week 1",
            path.file_name().and_then(|name| name.to_str()).unwrap(),
            "remote-v1",
        )
        .map(|item| item.0),
        Some(path.clone())
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_success_is_reused_and_migrated_without_retry() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"cached").expect("write cached file");
    let prior = DocumentAnalysis {
        id: "legacy-id".into(),
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: path.to_string_lossy().to_string(),
        status: "done".into(),
        summary: "persisted".into(),
        ..Default::default()
    };
    let status = CourseAutomationStatus {
        document_analyses: vec![prior.clone()],
        ..Default::default()
    };
    let document = AnalysisDocument {
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: path.to_string_lossy().to_string(),
        content: "cached".into(),
        source_fingerprint: "remote-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };

    let matched = previous_document_analysis(&status, &document).expect("legacy match");
    let migrated = migrate_successful_analysis(matched, &document, "content-fingerprint".into());
    assert_eq!(migrated.summary, "persisted");
    assert_eq!(migrated.source_fingerprint, "remote-v1");
    assert_eq!(migrated.id, document_id(&document));
    let _ = std::fs::remove_file(path);
}

#[test]
fn new_source_version_is_appended_without_deleting_success_history() {
    let mut analyses = vec![DocumentAnalysis {
        id: "version-1".into(),
        status: "done".into(),
        summary: "old success".into(),
        ..Default::default()
    }];
    upsert_document_analysis(
        &mut analyses,
        DocumentAnalysis {
            id: "version-2".into(),
            status: "done".into(),
            summary: "new success".into(),
            ..Default::default()
        },
    );

    assert_eq!(analyses.len(), 2);
    assert!(analyses.iter().any(|item| item.summary == "old success"));
    assert!(analyses.iter().any(|item| item.summary == "new success"));
}

#[test]
fn completed_summary_batch_removes_only_consumed_pending_ids() {
    let mut pending = vec!["a".into(), "b".into(), "c".into(), "d".into()];
    remove_consumed_ids(&mut pending, &["b".into(), "d".into()]);
    assert_eq!(pending, vec!["a", "c"]);
}

#[test]
fn summary_generation_id_tracks_actual_delta_content() {
    let memory = AgentCourseAnalysis {
        summary: "current memory".into(),
        ..Default::default()
    };
    let first = DocumentAnalysis {
        id: "doc-1".into(),
        fingerprint: "v1".into(),
        ..Default::default()
    };
    let changed = DocumentAnalysis {
        id: "doc-1".into(),
        fingerprint: "v2".into(),
        ..Default::default()
    };
    let event = SourceEvent {
        id: "event-1".into(),
        ..Default::default()
    };
    let empty_todos: Vec<context::ExistingCourseTodo> = Vec::new();
    let todo_context = vec![existing_todo("第3回 レポート課題")];

    let stable_a = summary_generation_id("course", 0, &[&first], &[], &memory, &empty_todos);
    let stable_b = summary_generation_id("course", 0, &[&first], &[], &memory, &empty_todos);
    let changed_doc = summary_generation_id("course", 0, &[&changed], &[], &memory, &empty_todos);
    let changed_event =
        summary_generation_id("course", 0, &[&first], &[&event], &memory, &empty_todos);
    let changed_todos = summary_generation_id("course", 0, &[&first], &[], &memory, &todo_context);

    assert_eq!(stable_a, stable_b);
    assert_ne!(stable_a, changed_doc);
    assert_ne!(stable_a, changed_event);
    assert_ne!(stable_a, changed_todos);
    assert!(stable_a.starts_with("course-automation-course-summary-0-"));
}

#[test]
fn final_summary_input_uses_delta_and_excludes_ledger_metadata() {
    let previous = AgentCourseAnalysis {
        summary: "compressed working memory".into(),
        standing_context: vec![Item {
            text: "keep this context".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let delta = DocumentAnalysis {
        id: "new-success".into(),
        fingerprint: "must-not-be-sent".into(),
        path: "/must/not/be/sent.pdf".into(),
        status: "done".into(),
        summary: "new context".into(),
        ..Default::default()
    };
    let student = json!({"studentNumber": "1234"});
    let existing_todos = vec![existing_todo("第3回 レポート課題")];
    let source_event = SourceEvent {
        id: "event-1".into(),
        event: "removed".into(),
        document_id: "old-document-id".into(),
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "old.pdf".into(),
        previous_summary: "old context".into(),
        detail: "old.pdf: removed".into(),
        attention: true,
    };
    let input = context::SummaryInput {
        current_local_time: "2026-06-15 10:00".into(),
        course_id: "course",
        course_name: "Course",
        student: &student,
        existing_course_todos: &existing_todos,
        previous_course_analysis: &previous,
        new_or_changed_documents: vec![context::CompactAnalysis::from(&delta)],
        source_events: vec![context::CompactSourceEvent::from(&source_event)],
    };
    let serialized = serde_json::to_string(&input).expect("serialize compact input");

    assert!(serialized.contains("compressed working memory"));
    assert!(serialized.contains("existingCourseTodos"));
    assert!(serialized.contains("第3回 レポート課題"));
    assert!(serialized.contains("new-success"));
    assert!(serialized.contains("old context"));
    assert!(!serialized.contains("must-not-be-sent"));
    assert!(!serialized.contains("/must/not/be/sent.pdf"));
    assert!(!serialized.contains("old-document-id"));
}

#[test]
fn normalizes_model_output_before_persisting_it() {
    let analysis = normalize_course_analysis(
        AgentCourseAnalysis {
            summary: "a".repeat(400),
            findings: [
                "same",
                "same",
                &"b".repeat(400),
                "third",
                "fourth",
                "fifth",
                "sixth",
                "seventh",
                "eighth",
            ]
            .into_iter()
            .map(|text| Item {
                text: text.to_string(),
                ..Default::default()
            })
            .collect(),
            standing_context: (0..20)
                .map(|index| Item {
                    text: format!("context-{index}"),
                    ..Default::default()
                })
                .collect(),
            seat: SeatConclusion {
                assignment: "s".repeat(100),
                evidence: (0..10).map(|index| format!("evidence-{index}")).collect(),
                confidence: 0.9,
            },
            ..Default::default()
        },
        &[],
        "2026-06-17",
    );

    assert_eq!(analysis.summary.chars().count(), 240);
    // No cap any more: 8 unique survive ("same" deduped). Exceeding the old
    // limit of 6 proves there is no item cap; the 400-char one is kept too.
    assert_eq!(analysis.findings.len(), 8);
    assert_eq!(analysis.standing_context.len(), 20);
    assert_eq!(analysis.seat.assignment.chars().count(), 80);
    assert_eq!(analysis.seat.evidence.len(), 6);
}

#[test]
fn memory_partition_separates_expired_from_active() {
    let memory = |text: &str, expires_at: &str| Item {
        text: text.into(),
        expires_at: expires_at.into(),
        ..Default::default()
    };
    let (active, expired) = items::partition_by_expiry(
        vec![
            memory("expired", "2026-06-10"),
            memory("today still valid", "2026-06-17"),
            memory("future", "2026-12-01"),
            memory("standing rule", ""),
        ],
        "memory",
        "2026-06-17",
        280,
    );

    let active_texts: Vec<&str> = active.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        active_texts,
        vec!["today still valid", "future", "standing rule"]
    );
    let expired_texts: Vec<&str> = expired.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(expired_texts, vec!["expired"]);
}

#[test]
fn consolidate_archive_merges_labels_and_folds_stale() {
    let groups = vec![
        ArchivedGroup {
            label: "完了した課題".into(),
            items: vec!["第1回レポート".into(), "第1回レポート".into()],
        },
        ArchivedGroup {
            label: "完了した課題".into(),
            items: vec!["第2回レポート".into()],
        },
        ArchivedGroup {
            label: "終了したイベント".into(),
            items: vec![],
        },
    ];
    let archive = consolidate_archive(groups, vec!["第3回レポート".into()]);

    // Same-label groups merge and items dedupe; the empty group is dropped.
    assert_eq!(archive.len(), 2);
    let assignments = &archive[0];
    assert_eq!(assignments.label, "完了した課題");
    assert_eq!(assignments.items, vec!["第1回レポート", "第2回レポート"]);
    // The freshly-expired memory lands under the fallback heading.
    assert_eq!(archive[1].label, ARCHIVE_FALLBACK_LABEL);
    assert_eq!(archive[1].items, vec!["第3回レポート"]);
}

#[test]
fn archive_floor_restores_history_when_model_omits_it() {
    let previous = vec![ArchivedGroup {
        label: "完了した課題".into(),
        items: vec!["第1回レポート".into()],
    }];
    // Model returned an analysis with NO archivedContext — history must survive.
    let analysis =
        normalize_course_analysis(AgentCourseAnalysis::default(), &previous, "2026-06-19");
    assert_eq!(analysis.archived_context.len(), 1);
    assert_eq!(analysis.archived_context[0].label, "完了した課題");
    assert_eq!(analysis.archived_context[0].items, vec!["第1回レポート"]);
}

#[test]
fn memory_items_accept_legacy_string_form() {
    let parsed: Vec<Item> =
        serde_json::from_str(r#"["legacy", {"text": "new", "expiresAt": "2026-09-01"}]"#)
            .expect("deserialize mixed standing context");
    assert_eq!(parsed[0].text, "legacy");
    assert_eq!(parsed[0].expires_at, "");
    assert_eq!(parsed[1].text, "new");
    assert_eq!(parsed[1].expires_at, "2026-09-01");
}
