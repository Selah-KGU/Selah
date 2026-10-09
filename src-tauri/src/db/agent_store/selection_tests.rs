use super::*;

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-agent-selection-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn selection_is_validated_and_repeated_selection_does_not_write() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    assert_eq!(db.agent_active_conversation().unwrap(), None);
    db.agent_create_conversation("A", "A").unwrap();
    assert!(db.agent_set_active_conversation("A").unwrap());
    let revision = db.cache_revision(ACTIVE_CONV_KEY).unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE data_cache SET updated_at=123 WHERE cache_key=?1",
            params![ACTIVE_CONV_KEY],
        )
        .unwrap();
    assert!(!db.agent_set_active_conversation("A").unwrap());
    assert_eq!(db.cache_revision(ACTIVE_CONV_KEY), Some(revision));
    assert_eq!(
        db.get_data_cache(ACTIVE_CONV_KEY).unwrap(),
        Some(("A".into(), 123))
    );
    assert_eq!(
        db.agent_active_conversation().unwrap().as_deref(),
        Some("A")
    );
    assert!(db.agent_set_active_conversation("").unwrap());
    assert_eq!(db.agent_active_conversation().unwrap(), None);
    assert!(db.cache_revision(ACTIVE_CONV_KEY).unwrap() > revision);
    assert!(!db.agent_set_active_conversation("").unwrap());
}

#[test]
fn stale_legacy_pointer_is_not_returned_and_invalid_selection_keeps_current() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.save_data_cache(ACTIVE_CONV_KEY, "missing").unwrap();
    assert_eq!(db.agent_active_conversation().unwrap(), None);
    db.agent_create_conversation("B", "B").unwrap();
    db.agent_set_active_conversation("B").unwrap();
    let revision = db.cache_revision(ACTIVE_CONV_KEY);
    assert!(db.agent_set_active_conversation("missing").is_err());
    assert_eq!(
        db.agent_active_conversation().unwrap().as_deref(),
        Some("B")
    );
    assert_eq!(db.cache_revision(ACTIVE_CONV_KEY), revision);
    assert_eq!(db.agent_list_conversations().unwrap().len(), 1);
}

#[test]
fn deletion_clears_only_its_own_selection_and_preserves_a_newer_selection() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    for id in ["A", "B", "C"] {
        db.agent_create_conversation(id, id).unwrap();
    }
    db.agent_set_active_conversation("A").unwrap();
    let before = db.cache_revision(ACTIVE_CONV_KEY).unwrap();
    db.agent_delete_conversation("A").unwrap();
    assert_eq!(db.agent_active_conversation().unwrap(), None);
    assert_eq!(db.cache_payload(ACTIVE_CONV_KEY).as_deref(), Some(""));
    assert!(db.cache_revision(ACTIVE_CONV_KEY).unwrap() > before);
    assert!(db.agent_set_active_conversation("A").is_err());
    db.agent_set_active_conversation("B").unwrap();
    let revision = db.cache_revision(ACTIVE_CONV_KEY);
    for id in ["C", "A", "C"] {
        db.agent_delete_conversation(id).unwrap();
    }
    assert_eq!(
        db.agent_active_conversation().unwrap().as_deref(),
        Some("B")
    );
    assert_eq!(db.cache_revision(ACTIVE_CONV_KEY), revision);
    let reopened = Database::open(&temporary.0).unwrap();
    assert_eq!(
        reopened.agent_active_conversation().unwrap().as_deref(),
        Some("B")
    );
}

#[test]
fn selection_clear_failure_rolls_back_messages_parent_and_cache_revision() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("A", "keep title").unwrap();
    db.agent_append_message("A", "user", "全文 🌕", Some("[]"), None, None)
        .unwrap();
    db.agent_set_active_conversation("A").unwrap();
    let revision = db.cache_revision(ACTIVE_CONV_KEY);
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_selection_clear BEFORE UPDATE OF data_json ON data_cache WHEN OLD.cache_key='agent_active_conversation' BEGIN SELECT RAISE(ABORT, 'selection clear failure'); END;").unwrap();
    assert!(db
        .agent_delete_conversation("A")
        .unwrap_err()
        .contains("selection clear failure"));
    let reopened = Database::open(&temporary.0).unwrap();
    assert_eq!(
        reopened.agent_list_conversations().unwrap()[0].title,
        "keep title"
    );
    let messages = reopened.agent_load_messages("A").unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "全文 🌕");
    assert_eq!(messages[0].images_json.as_deref(), Some("[]"));
    assert_eq!(
        reopened.agent_active_conversation().unwrap().as_deref(),
        Some("A")
    );
    assert_eq!(reopened.cache_revision(ACTIVE_CONV_KEY), revision);
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_selection_clear;")
        .unwrap();
    db.agent_delete_conversation("A").unwrap();
    assert!(reopened.agent_list_conversations().unwrap().is_empty());
    assert!(reopened.agent_load_messages("A").unwrap().is_empty());
    assert_eq!(reopened.agent_active_conversation().unwrap(), None);
}

#[test]
fn independent_selection_and_delete_writers_never_resurrect_a_deleted_pointer() {
    use std::sync::{Arc, Barrier};
    let temporary = Temporary::new();
    let selector = Arc::new(Database::open(&temporary.0).unwrap());
    let deleter = Arc::new(Database::open(&temporary.0).unwrap());
    for db in [&selector, &deleter] {
        db.conn
            .lock()
            .unwrap()
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
    }
    for round in 0..32 {
        let id = format!("race-{round}");
        selector.agent_create_conversation(&id, "race").unwrap();
        selector.agent_set_active_conversation("").unwrap();
        let gate = Arc::new(Barrier::new(2));
        let writer = selector.clone();
        let writer_id = id.clone();
        let writer_gate = gate.clone();
        let select = std::thread::spawn(move || {
            writer_gate.wait();
            writer.agent_set_active_conversation(&writer_id)
        });
        let delete_db = deleter.clone();
        let delete = std::thread::spawn(move || {
            gate.wait();
            delete_db.agent_delete_conversation(&id)
        });
        let _ = select.join().unwrap();
        delete.join().unwrap().unwrap();
        // Inspect the raw durable pointer too: a validating getter alone could
        // hide a stale pointer written after deletion.
        assert_eq!(selector.cache_payload(ACTIVE_CONV_KEY).as_deref(), Some(""));
        assert_eq!(selector.agent_active_conversation().unwrap(), None);
    }
}
