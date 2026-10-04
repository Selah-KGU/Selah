//! JSON extraction, course config, and delta-cycle tests.

use super::*;

#[test]
fn extracts_json_from_fenced_response() {
    let raw = "```json\n{\"summary\":\"ok\"}\n```";
    assert_eq!(extract_json_object(raw), Some("{\"summary\":\"ok\"}"));
}

#[test]
fn extracts_first_complete_json_without_trailing_model_text() {
    let raw = "{\"summary\":\"ok\",\"nested\":{\"text\":\"brace } in string\"}}\n追加説明";
    assert_eq!(
        extract_json_object(raw),
        Some("{\"summary\":\"ok\",\"nested\":{\"text\":\"brace } in string\"}}")
    );
}

#[test]
fn config_defaults_to_disabled_and_full_monitoring() {
    let config = CourseAutomationConfig::new("C1".into(), "Course".into());
    assert!(!config.enabled);
    assert!(config.monitor_materials);
    assert!(config.monitor_announcements);
    assert!(config.monitor_assignments);
    assert!(!config.analyze_all);
    assert!(config.auto_print);
}

#[test]
fn loads_existing_course_todos_for_ai_context() {
    let root = std::env::temp_dir().join(format!("course-todos-{}", uuid::Uuid::new_v4()));
    let db = Database::open(&root).expect("temp db");
    save_json(
        &db,
        LUNA_TODO_CACHE_KEY,
        &vec![
            luna_todo(
                "国際学部/International Studies 34001001 キリスト教学A　１",
                "第3回 レポート課題",
                "2026/07/03 23:59",
            ),
            luna_todo("別の授業", "別授業の課題", "2026/07/03"),
        ],
    )
    .expect("save luna todos");
    save_json(
        &db,
        DETAIL_TODO_CACHE_KEY,
        &vec![
            DetailTodo {
                id: "detail-active".into(),
                title: "確認済みの追加課題".into(),
                course_name: "キリスト教学A １".into(),
                content_type: "課題".into(),
                deadline: "2026-07-10".into(),
                ..Default::default()
            },
            DetailTodo {
                id: "detail-completed".into(),
                title: "完了済みの追加課題".into(),
                course_name: "キリスト教学A １".into(),
                completed_at: Some("2026-07-03T00:00:00Z".into()),
                ..Default::default()
            },
        ],
    )
    .expect("save detail todos");

    let todos = load_existing_course_todos(&db, "キリスト教学A １");
    let titles = todos
        .iter()
        .map(|todo| todo.title.as_str())
        .collect::<Vec<_>>();

    assert!(titles.contains(&"第3回 レポート課題"));
    assert!(titles.contains(&"確認済みの追加課題"));
    assert!(!titles.contains(&"別授業の課題"));
    assert!(!titles.contains(&"完了済みの追加課題"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn document_fingerprint_changes_with_content() {
    let mut document = AnalysisDocument {
        kind: "material".into(),
        title: "座席表".into(),
        filename: "seats.xlsx".into(),
        path: "/tmp/seats.xlsx".into(),
        content: "A-1".into(),
        source_fingerprint: "remote-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let before = document_fingerprint(&document).expect("fingerprint");
    document.content = "A-2".into();
    let after = document_fingerprint(&document).expect("fingerprint");
    assert_ne!(before, after);
}

#[test]
fn delta_cycle_processes_only_new_changed_or_retryable_documents() {
    let mut document = AnalysisDocument {
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: "/tmp/notes.pdf".into(),
        content: "same".into(),
        source_fingerprint: "remote-v1".into(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let fingerprint = document_fingerprint(&document).expect("fingerprint");
    let id = document_id(&document);
    let previous_done = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: id.clone(),
            fingerprint: fingerprint.clone(),
            source_fingerprint: "remote-v1".into(),
            status: "done".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(!should_process_document_delta(&previous_done, &document));

    let previous_skipped = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: id.clone(),
            fingerprint: fingerprint.clone(),
            source_fingerprint: "remote-v1".into(),
            status: "skipped".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(!should_process_document_delta(&previous_skipped, &document));

    let previous_error = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id,
            fingerprint,
            source_fingerprint: "remote-v1".into(),
            status: "error".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(should_process_document_delta(&previous_error, &document));

    document.content = "changed".into();
    document.source_fingerprint = "remote-v2".into();
    assert!(should_process_document_delta(&previous_done, &document));
}

#[test]
fn organize_signal_folds_findings_into_summary() {
    let signal = organize_signal(
        "配布資料です",
        &["第3回の小テスト範囲".into(), "".into(), "提出は来週".into()],
    );
    // Findings are merged into the signal (empty ones skipped), so a session
    // marker present only in findings is still visible to theme detection
    // (theme_of searches the summary — see session_marker_groups_across_kinds).
    assert!(signal.contains("配布資料です"));
    assert!(signal.contains("第3回の小テスト範囲"));
    assert!(signal.contains("提出は来週"));
    // Whitespace-collapsed and bounded.
    assert!(!signal.contains("  "));
    assert!(signal.chars().count() <= 600);
}

#[test]
fn active_live_notes_maps_sidecars_to_md_names() {
    let root = std::env::temp_dir().join(format!("live-active-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    // Open-session sidecars (cache + deltas), incl. a build-cache-tagged one.
    std::fs::write(root.join(".20260623_キリスト教学_live.cache.json"), b"{}").unwrap();
    std::fs::write(root.join(".20260624_キリスト教学_live.lines.ndjson"), b"").unwrap();
    std::fs::write(
        root.join(".20260625_キリスト教学_live-debug.cache.json"),
        b"{}",
    )
    .unwrap();
    // Finished note: md present but no sidecar → organizable.
    std::fs::write(root.join("20260601_キリスト教学_live.md"), b"x").unwrap();

    let active = active_live_notes(&root);
    assert!(active.contains("20260623_キリスト教学_live.md"));
    assert!(active.contains("20260624_キリスト教学_live.md"));
    assert!(active.contains("20260625_キリスト教学_live.md")); // tag tolerated
    assert!(!active.contains("20260601_キリスト教学_live.md"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn organize_notices_keeps_only_session_pinning_announcements() {
    let status = CourseAutomationStatus {
        document_analyses: vec![
            DocumentAnalysis {
                kind: "announcement".into(),
                title: "5月26日 第7回 座席表".into(),
                summary: "本日の座席指定".into(),
                status: "done".into(),
                ..Default::default()
            },
            DocumentAnalysis {
                kind: "announcement".into(),
                title: "システムメンテナンス".into(),
                summary: "停止のお知らせ".into(),
                status: "done".into(),
                ..Default::default()
            },
            DocumentAnalysis {
                kind: "material".into(),
                title: "第3回資料".into(),
                status: "done".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let notices = organize_notices(&status);
    // Only the session-pinning announcement; the no-session one and the
    // material file (grouped directly as a candidate) are excluded.
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("第7回"));
}

#[test]
fn moving_a_file_does_not_retrigger_analysis() {
    // No source_fingerprint: identity and version must both be path-free, so
    // the organizer relocating the file never looks like a new/changed doc.
    let mut document = AnalysisDocument {
        kind: "material".into(),
        title: "第01回資料".into(),
        filename: "shiryo.pdf".into(),
        path: "/Selah/course/shiryo.pdf".into(),
        content: "BODY".into(),
        source_fingerprint: String::new(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let previous = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: document_id(&document),
            fingerprint: document_fingerprint(&document).expect("fingerprint"),
            source_fingerprint: String::new(),
            status: "done".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(!should_process_document_delta(&previous, &document));
    // Organizer files it into a theme folder: same content, new path.
    document.path = "/Selah/course/第01回/shiryo.pdf".into();
    assert!(!should_process_document_delta(&previous, &document));
    // A genuine content change still re-triggers.
    document.content = "BODY-revised".into();
    assert!(should_process_document_delta(&previous, &document));
}

#[test]
fn downloaded_delta_skips_same_source_success_without_rehashing_content() {
    let entry: (String, String, String, PathBuf, String) = (
        "material".to_string(),
        "Week 1".to_string(),
        "notes.pdf".to_string(),
        PathBuf::from("/tmp/notes.pdf"),
        "remote-v1".to_string(),
    );
    let document =
        analysis_document_from_download_entry(&entry.0, &entry.1, &entry.2, &entry.3, &entry.4);
    let previous = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: document_id(&document),
            fingerprint: "old-content-fingerprint".into(),
            source_fingerprint: "remote-v1".into(),
            status: "done".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(!should_process_downloaded_delta(&previous, &entry));
}

#[test]
fn downloaded_delta_allows_legacy_success_identity_migration() {
    let entry: (String, String, String, PathBuf, String) = (
        "material".to_string(),
        "Week 1".to_string(),
        "notes.pdf".to_string(),
        PathBuf::from("/tmp/notes.pdf"),
        "remote-v1".to_string(),
    );
    let previous = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: "legacy-id".into(),
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "notes.pdf".into(),
            status: "done".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(should_process_downloaded_delta(&previous, &entry));
}

#[test]
fn activity_detail_cache_records_only_successful_detail_fetches() {
    let value = json!({
        "status": "detail",
        "kind": "announcement",
        "title": "LUNA告知",
        "detail_path": "/lms/coursetop/information/listdetail?idnumber=6072&informationId=A1",
        "list_fingerprint": "list-v1",
        "source_fingerprint": "detail-v1",
    });

    let record = activity_detail_cache_record_from_value(&value, 1200)
        .expect("successful detail should become cache record");
    assert_eq!(record.kind, "announcement");
    assert_eq!(record.list_fingerprint, "list-v1");
    assert_eq!(record.source_fingerprint, "detail-v1");
    assert_eq!(record.checked_at, 1200);

    let reusable = reusable_activity_details(std::slice::from_ref(&record));
    let cached = reusable
        .get("/lms/coursetop/information/listdetail?idnumber=6072&informationId=A1")
        .expect("cache should be keyed by detail path");
    assert_eq!(cached.list_fingerprint, "list-v1");
    assert_eq!(cached.source_fingerprint, "detail-v1");

    let cached_marker = json!({
        "status": "detail_cached",
        "kind": "announcement",
        "title": "LUNA告知",
        "detail_path": record.detail_path,
        "list_fingerprint": "list-v1",
        "source_fingerprint": "detail-v1",
    });
    assert!(activity_detail_cache_record_from_value(&cached_marker, 1300).is_none());
}

#[test]
fn activity_detail_snapshot_treats_cached_and_fetched_detail_as_same_source() {
    let fetched = json!({
        "status": "detail",
        "kind": "announcement",
        "title": "LUNA告知",
        "detail_path": "/lms/coursetop/information/listdetail?idnumber=6072&informationId=A1",
        "list_fingerprint": "list-v1",
        "source_fingerprint": "detail-v1",
    });
    let cached = json!({
        "status": "detail_cached",
        "stale": true,
        "kind": "announcement",
        "title": "LUNA告知",
        "detail_path": "/lms/coursetop/information/listdetail?idnumber=6072&informationId=A1",
        "list_fingerprint": "list-v1",
        "source_fingerprint": "detail-v1",
        "error": "temporary fetch failure",
    });

    assert_eq!(
        activity_detail_snapshot_value(&fetched),
        activity_detail_snapshot_value(&cached)
    );
    assert_eq!(
        sha256_json(&activity_detail_snapshot_value(&fetched)).expect("fetched hash"),
        sha256_json(&activity_detail_snapshot_value(&cached)).expect("cached hash"),
    );
}

#[test]
fn activity_attachment_failures_force_detail_revalidation() {
    let material_failure = failed_artifact(
        "material",
        "Week 1",
        "notes.pdf",
        "material-v1",
        "download failed",
    );
    assert!(!has_retryable_activity_artifact_failure(&[
        material_failure
    ]));

    let announcement_failure = failed_artifact(
        "announcement",
        "LUNA告知",
        "worksheet.pdf",
        "attachment-v1",
        "download failed",
    );
    assert!(has_retryable_activity_artifact_failure(&[
        announcement_failure
    ]));

    let report_failure = failed_artifact(
        "report",
        "提出課題",
        "submission.pdf",
        "attachment-v2",
        "download failed",
    );
    assert!(has_retryable_activity_artifact_failure(&[report_failure]));
}

#[test]
fn delta_cycle_pauses_after_new_immediate_but_full_sweep_continues() {
    let previous_done = DocumentAnalysis {
        id: "old".into(),
        status: "done".into(),
        trigger_decision: "immediate".into(),
        ..Default::default()
    };
    let immediate = DocumentAnalysis {
        id: "new".into(),
        status: "done".into(),
        trigger_decision: "immediate".into(),
        ..Default::default()
    };
    let routine = DocumentAnalysis {
        id: "routine".into(),
        status: "done".into(),
        trigger_decision: "routine".into(),
        ..Default::default()
    };

    assert!(should_pause_delta_cycle_after_analysis(
        false, None, &immediate
    ));
    assert!(!should_pause_delta_cycle_after_analysis(
        true, None, &immediate
    ));
    assert!(!should_pause_delta_cycle_after_analysis(
        false,
        Some(&previous_done),
        &immediate
    ));
    assert!(!should_pause_delta_cycle_after_analysis(
        false, None, &routine
    ));
}

#[test]
fn deferred_delta_followup_is_queued_only_after_successful_pause() {
    assert!(should_queue_deferred_delta_followup(true, true));
    assert!(!should_queue_deferred_delta_followup(true, false));
    assert!(!should_queue_deferred_delta_followup(false, true));
    assert!(!should_queue_deferred_delta_followup(false, false));
}

#[test]
fn disabled_course_skips_only_automatic_cycles() {
    assert!(should_skip_automatic_cycle(
        false,
        "scheduled",
        None,
        30,
        1_000
    ));
    assert!(should_skip_automatic_cycle(
        false, "deferred", None, 30, 1_000
    ));
    assert!(!should_skip_automatic_cycle(
        false, "manual", None, 30, 1_000
    ));
    assert!(!should_skip_automatic_cycle(
        true,
        "scheduled",
        None,
        30,
        1_000
    ));
    assert!(!should_skip_automatic_cycle(
        true,
        "deferred",
        Some(990),
        30,
        1_000
    ));
}

#[test]
fn scheduled_cycle_rechecks_due_at_execution_time() {
    assert!(scheduled_cycle_is_due(None, 30, 2_000));
    assert!(scheduled_cycle_is_due(Some(0), 30, 2_000));
    assert!(!scheduled_cycle_is_due(Some(900), 30, 2_000));
    assert!(should_skip_automatic_cycle(
        true,
        "scheduled",
        Some(900),
        30,
        2_000
    ));
    assert!(!should_skip_automatic_cycle(
        true,
        "scheduled",
        Some(0),
        30,
        2_000
    ));
    assert!(!should_skip_automatic_cycle(
        true,
        "deferred",
        Some(1_999),
        30,
        2_000
    ));
}
