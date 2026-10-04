//! Download reuse, analysis persistence, and summary refresh tests.

use super::*;

#[test]
fn fingerprinted_download_is_not_reused_when_remote_version_changes() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"persisted").expect("write cached file");
    let artifacts = vec![CourseArtifactRecord {
        id: artifact_id("material", "notes.pdf"),
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: path.to_string_lossy().to_string(),
        status: "downloaded".into(),
        source_fingerprint: "first-success".into(),
        ..Default::default()
    }];

    let reused = reusable_artifact_path(
        &artifacts,
        "material",
        "Renamed Week",
        "notes.pdf",
        "first-success",
    )
    .expect("same remote version can reuse");
    assert_eq!(reused, (path.clone(), "first-success".into()));
    assert!(reusable_artifact_path(
        &artifacts,
        "material",
        "Renamed Week",
        "notes.pdf",
        "changed"
    )
    .is_none());
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_download_without_fingerprint_can_migrate_once() {
    let path = std::env::temp_dir().join(format!("course-plus-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"persisted").expect("write cached file");
    let artifacts = vec![CourseArtifactRecord {
        id: artifact_id("legacy", "notes.pdf"),
        kind: "legacy".into(),
        filename: "notes.pdf".into(),
        path: path.to_string_lossy().to_string(),
        status: "downloaded".into(),
        ..Default::default()
    }];

    let reused =
        reusable_artifact_path(&artifacts, "material", "Week 1", "notes.pdf", "current-v1")
            .expect("legacy no-fingerprint cache can be migrated");
    assert_eq!(reused, (path.clone(), String::new()));
    let _ = std::fs::remove_file(path);
}

#[test]
fn failed_artifact_is_retried_and_replaced_by_success() {
    let mut artifacts = vec![failed_artifact(
        "material",
        "Week 1",
        "notes.pdf",
        "remote-v1",
        "temporary",
    )];
    assert!(
        reusable_artifact_path(&artifacts, "material", "Week 1", "notes.pdf", "remote-v1")
            .is_none()
    );

    upsert_artifact(
        &mut artifacts,
        CourseArtifactRecord {
            id: artifact_id("material", "notes.pdf"),
            kind: "material".into(),
            title: "Week 1".into(),
            filename: "notes.pdf".into(),
            status: "downloaded".into(),
            ..Default::default()
        },
    );
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].error.is_empty());
}

#[test]
fn retry_failure_does_not_delete_download_success_history() {
    let mut artifacts = vec![CourseArtifactRecord {
        id: artifact_id("material", "notes.pdf"),
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.pdf".into(),
        path: "/missing/notes.pdf".into(),
        status: "downloaded".into(),
        source_fingerprint: "first-success".into(),
        ..Default::default()
    }];

    upsert_artifact(
        &mut artifacts,
        failed_artifact(
            "material",
            "Week 1",
            "notes.pdf",
            "remote-v2",
            "temporary retry failure",
        ),
    );

    assert_eq!(artifacts.len(), 2);
    assert!(artifacts.iter().any(|item| item.status == "downloaded"));
    assert!(artifacts.iter().any(|item| item.status == "error"));
}

#[test]
fn later_failure_cannot_overwrite_successful_analysis() {
    let mut analyses = vec![DocumentAnalysis {
        id: "same-version".into(),
        status: "done".into(),
        summary: "persisted success".into(),
        ..Default::default()
    }];
    upsert_document_analysis(
        &mut analyses,
        DocumentAnalysis {
            id: "same-version".into(),
            status: "error".into(),
            error: "temporary read failure".into(),
            ..Default::default()
        },
    );

    assert_eq!(analyses.len(), 1);
    assert_eq!(analyses[0].status, "done");
    assert_eq!(analyses[0].summary, "persisted success");
}

#[test]
fn successful_analysis_does_not_read_file_again() {
    let path =
        std::env::temp_dir().join(format!("course-plus-{}.unsupported", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"already analyzed").expect("write cached file");
    let source_fingerprint = "persisted-source".to_string();
    let document = AnalysisDocument {
        kind: "material".into(),
        title: "Week 1".into(),
        filename: "notes.unsupported".into(),
        path: path.to_string_lossy().to_string(),
        content: String::new(),
        source_fingerprint: source_fingerprint.clone(),
        load_error: String::new(),
        images: Vec::new(),
    };
    let previous = CourseAutomationStatus {
        document_analyses: vec![DocumentAnalysis {
            id: document_id(&document),
            status: "done".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let documents = build_analysis_documents(
        &[(
            "material".into(),
            "Week 1".into(),
            "notes.unsupported".into(),
            path.clone(),
            source_fingerprint,
        )],
        &previous,
    );
    assert_eq!(documents.len(), 1);
    assert!(documents[0].load_error.is_empty());
    assert!(documents[0].content.is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn failed_version_is_replaced_when_retry_succeeds() {
    let mut analyses = vec![DocumentAnalysis {
        id: "same-version".into(),
        status: "error".into(),
        error: "temporary".into(),
        ..Default::default()
    }];
    upsert_document_analysis(
        &mut analyses,
        DocumentAnalysis {
            id: "same-version".into(),
            status: "done".into(),
            summary: "recovered".into(),
            ..Default::default()
        },
    );

    assert_eq!(analyses.len(), 1);
    assert_eq!(analyses[0].status, "done");
    assert_eq!(analyses[0].summary, "recovered");
}

#[test]
fn comprehensive_summary_waits_for_enough_new_items() {
    assert!(!should_refresh_summary(
        "existing context",
        3,
        false,
        false,
        0
    ));
    assert!(should_refresh_summary(
        "existing context",
        4,
        false,
        false,
        0
    ));
    assert!(should_refresh_summary(
        "existing context",
        1,
        true,
        false,
        0
    ));
    assert!(should_refresh_summary(
        "existing context",
        1,
        false,
        true,
        0
    ));
    assert!(should_refresh_summary(
        "existing context",
        0,
        false,
        false,
        1
    ));
    assert!(should_refresh_summary("", 1, false, false, 0));
}

#[test]
fn summary_items_prioritize_immediate_without_dropping_routine_backlog() {
    let mut items = vec![
        DocumentAnalysis {
            id: "routine-old".into(),
            trigger_decision: "routine".into(),
            ..Default::default()
        },
        DocumentAnalysis {
            id: "observe-old".into(),
            trigger_decision: "observe".into(),
            ..Default::default()
        },
        DocumentAnalysis {
            id: "immediate-new".into(),
            trigger_decision: "immediate".into(),
            ..Default::default()
        },
        DocumentAnalysis {
            id: "routine-new".into(),
            trigger_decision: "routine".into(),
            ..Default::default()
        },
    ];

    prioritize_summary_items(&mut items);

    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["immediate-new", "observe-old", "routine-old", "routine-new"]
    );
}

#[test]
fn observe_decision_waits_until_followup_evidence_arrives() {
    let analyses = vec![
        DocumentAnalysis {
            id: "watch".into(),
            status: "done".into(),
            trigger_decision: "observe".into(),
            observation_context: "次回資料と照合".into(),
            ..Default::default()
        },
        DocumentAnalysis {
            id: "later".into(),
            status: "done".into(),
            trigger_decision: "routine".into(),
            ..Default::default()
        },
    ];

    assert!(!has_observe_followup(
        &analyses,
        &["watch".into()],
        &["watch".into()]
    ));
    assert!(has_observe_followup(
        &analyses,
        &["watch".into(), "later".into()],
        &["later".into()]
    ));
}
