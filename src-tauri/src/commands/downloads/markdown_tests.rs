use super::*;
use tauri::ipc::IpcResponse;

#[tokio::test]
async fn startup_command_returns_full_raw_json_then_null_and_stops_retry() {
    let label = "fixture-markdown-startup-command";
    let delivery = PENDING_MARKDOWN_PAYLOADS.reserve(label);
    let expected = serde_json::json!({
        "path": "/fixture/note.md", "filename": "note.md",
        "markdown": "# 授業\n\"引用\" 👩🏽‍💻\n".repeat(10000),
        "error": null, "deliveryRevision": delivery.revision.to_string(),
    });
    delivery.publish(expected.clone());
    let response = get_pending_markdown_payload(label.into()).await.unwrap();
    assert_eq!(
        response
            .body()
            .unwrap()
            .deserialize::<serde_json::Value>()
            .unwrap(),
        expected
    );
    assert!(delivery.snapshot().is_none());
    let response = get_pending_markdown_payload(label.into()).await.unwrap();
    assert_eq!(
        response
            .body()
            .unwrap()
            .deserialize::<serde_json::Value>()
            .unwrap(),
        serde_json::Value::Null
    );
}

#[test]
fn acknowledgment_command_only_releases_the_matching_delivery() {
    let label = "fixture-markdown-ack-command";
    let old = PENDING_MARKDOWN_PAYLOADS.reserve(label);
    let current = PENDING_MARKDOWN_PAYLOADS.reserve(label);
    current.publish(serde_json::json!({"markdown": "全文"}));
    ack_markdown_payload(label.into(), "invalid".into());
    ack_markdown_payload(label.into(), old.revision.to_string());
    assert!(current.snapshot().is_some());
    ack_markdown_payload(label.into(), current.revision.to_string());
    assert!(current.snapshot().is_none());
    assert!(!current.publish(serde_json::json!("late")));
}
