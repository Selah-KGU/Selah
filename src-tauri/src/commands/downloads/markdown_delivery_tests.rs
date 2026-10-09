use super::*;
use serde_json::json;

#[test]
fn polling_before_publish_keeps_the_delivery_and_take_cancels_retry() {
    let registry = Registry::default();
    let delivery = registry.reserve("reader");
    assert!(registry.take("reader").is_none());
    let text = "# 授業\n👩🏽‍💻".repeat(1000);
    assert!(delivery
        .publish(json!({"markdown": text, "deliveryRevision": delivery.revision.to_string()})));
    let original = delivery.snapshot().unwrap();
    let consumed = registry.take("reader").unwrap();
    assert!(Arc::ptr_eq(&original, &consumed));
    assert_eq!(consumed["markdown"], text);
    assert!(delivery.snapshot().is_none());
    assert!(registry.take("reader").is_none());
    assert!(!delivery.publish(json!({"markdown": "late"})));
    assert!(registry.0.lock().unwrap().readers.is_empty());
}

#[test]
fn reopen_cancels_old_reads_and_old_acknowledgments_cannot_consume_the_new_read() {
    let registry = Registry::default();
    let old = registry.reserve("reader");
    let new = registry.reserve("reader");
    assert!(new.revision > old.revision);
    assert!(!old.publish(json!({"markdown": "stale"})));
    registry.acknowledge("reader", old.revision);
    assert!(new.publish(json!({"markdown": "current"})));
    assert_eq!(registry.take("reader").unwrap()["markdown"], "current");
}

#[test]
fn replacing_ready_text_drops_its_buffer_even_while_the_old_retry_token_survives() {
    let registry = Registry::default();
    let old = registry.reserve("reader");
    old.publish(json!({"markdown": "large".repeat(10000)}));
    let buffer = Arc::downgrade(&old.snapshot().unwrap());
    let new = registry.reserve("reader");
    assert!(buffer.upgrade().is_none());
    assert!(old.snapshot().is_none());
    new.publish(json!({"markdown": "next"}));
    registry.acknowledge("reader", new.revision);
    assert!(new.snapshot().is_none());
    assert!(!new.publish(json!({"markdown": "late"})));
    assert!(registry.0.lock().unwrap().readers.is_empty());
}

#[test]
fn close_cancels_pending_io_and_ready_retry_without_affecting_other_readers() {
    let registry = Registry::default();
    let pending = registry.reserve("pending");
    let ready = registry.reserve("ready");
    let other = registry.reserve("other");
    ready.publish(json!({"markdown": "text"}));
    registry.discard("pending");
    registry.discard("ready");
    registry.discard("missing");
    assert!(!pending.publish(json!("late")));
    assert!(ready.snapshot().is_none());
    assert!(other.publish(json!("other")));
    assert_eq!(*registry.take("other").unwrap(), json!("other"));
    assert!(registry.0.lock().unwrap().readers.is_empty());
}

#[test]
fn full_supported_payload_is_shared_and_serializes_without_truncation() {
    let registry = Registry::default();
    let delivery = registry.reserve("reader");
    let text = "授業👩🏽‍💻\n".repeat(8 * 1024 * 1024 / "授業👩🏽‍💻\n".len());
    assert!(delivery.publish(json!({"markdown": text, "error": null})));
    let first = delivery.snapshot().unwrap();
    let retry = delivery.snapshot().unwrap();
    assert!(Arc::ptr_eq(&first, &retry));
    assert_eq!(Arc::strong_count(&first), 3);
    let decoded: Value = serde_json::from_slice(&serde_json::to_vec(&retry).unwrap()).unwrap();
    assert_eq!(decoded["markdown"], text);
    registry.discard("reader");
    assert_eq!(Arc::strong_count(&first), 2);
}

#[test]
fn concurrent_read_completion_and_reopen_never_publish_into_the_new_slot() {
    let registry = Arc::new(Registry::default());
    for _ in 0..100 {
        let old = registry.reserve("reader");
        let worker = std::thread::spawn(move || {
            old.publish(json!("old"));
            old
        });
        let new = registry.reserve("reader");
        let old = worker.join().unwrap();
        assert!(old.snapshot().is_none());
        assert!(!old.publish(json!("old-after-reopen")));
        new.publish(json!("new"));
        assert_eq!(*registry.take("reader").unwrap(), json!("new"));
    }
    assert!(registry.0.lock().unwrap().readers.is_empty());
}
