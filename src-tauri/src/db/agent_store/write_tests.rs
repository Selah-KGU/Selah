use super::*;

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-agent-write-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn append_commits_complete_content_and_matching_metadata_together() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "manual title").unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_conversations SET updated_at=123 WHERE id='c'",
            [],
        )
        .unwrap();
    let image = r#"[{"mime":"image/png","data_base64":"AA=="}]"#;
    let tool = r#"{"body":"日本語","image":{"mime":"image/png","data_base64":"BB=="}}"#;
    let id = db
        .agent_append_message(
            "c",
            "tool",
            "全文 🌕",
            Some(image),
            Some("example"),
            Some(tool),
        )
        .unwrap();
    let reopened = Database::open(&temporary.0).unwrap();
    let history = reopened.agent_load_messages("c").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, id);
    assert_eq!(history[0].content, "全文 🌕");
    assert_eq!(history[0].images_json.as_deref(), Some(image));
    assert_eq!(history[0].tool_result_json.as_deref(), Some(tool));
    let conversation = &reopened.agent_list_conversations().unwrap()[0];
    assert_eq!(conversation.title, "manual title");
    assert_eq!(conversation.updated_at, history[0].created_at);
}

#[test]
fn timestamp_update_failure_rolls_back_the_message_and_allows_retry() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "manual").unwrap();
    db.conn.lock().unwrap().execute_batch("UPDATE agent_conversations SET updated_at=123 WHERE id='c'; CREATE TRIGGER reject_timestamp BEFORE UPDATE ON agent_conversations BEGIN SELECT RAISE(ABORT, 'timestamp failure'); END;").unwrap();
    let error = db
        .agent_append_message("c", "assistant", "complete answer", None, None, None)
        .unwrap_err();
    assert!(error.contains("timestamp failure"));
    assert!(db.agent_load_messages("c").unwrap().is_empty());
    assert_eq!(db.agent_list_conversations().unwrap()[0].updated_at, 123);
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_timestamp;")
        .unwrap();
    db.agent_append_message("c", "assistant", "complete answer", None, None, None)
        .unwrap();
    assert_eq!(db.agent_load_messages("c").unwrap().len(), 1);
}

#[test]
fn failed_message_insert_keeps_the_previous_conversation_timestamp() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "manual").unwrap();
    db.conn.lock().unwrap().execute_batch("UPDATE agent_conversations SET updated_at=123 WHERE id='c'; CREATE TRIGGER reject_message BEFORE INSERT ON agent_messages BEGIN SELECT RAISE(ABORT, 'message failure'); END;").unwrap();
    assert!(db
        .agent_append_message("c", "user", "speech", None, None, None)
        .unwrap_err()
        .contains("message failure"));
    assert!(db.agent_load_messages("c").unwrap().is_empty());
    assert_eq!(db.agent_list_conversations().unwrap()[0].updated_at, 123);
}

#[test]
fn missing_and_deleted_conversations_reject_late_messages_without_foreign_keys() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.conn
        .lock()
        .unwrap()
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    db.agent_create_conversation("deleted", "manual").unwrap();
    db.agent_create_conversation("other", "other").unwrap();
    db.agent_append_message("deleted", "user", "saved input", None, None, None)
        .unwrap();
    db.agent_delete_conversation("deleted").unwrap();
    for id in ["missing", "deleted"] {
        for role in ["user", "assistant", "tool"] {
            assert!(db
                .agent_append_message(id, role, "late content", None, None, None)
                .is_err());
        }
        assert!(db.agent_load_messages(id).unwrap().is_empty());
    }
    assert_eq!(db.agent_list_conversations().unwrap().len(), 1);
    assert_eq!(db.agent_list_conversations().unwrap()[0].id, "other");
}

#[test]
fn either_delete_failure_keeps_the_entire_conversation_and_can_retry() {
    for table in ["agent_messages", "agent_conversations"] {
        let temporary = Temporary::new();
        let db = Database::open(&temporary.0).unwrap();
        db.agent_create_conversation("c", "keep me").unwrap();
        db.agent_create_conversation("other", "other").unwrap();
        db.agent_append_message("c", "user", "full input", Some("[]"), None, None)
            .unwrap();
        db.agent_append_message(
            "c",
            "tool",
            "",
            None,
            Some("example"),
            Some(r#"{"body":"full result"}"#),
        )
        .unwrap();
        let before = db.agent_load_messages("c").unwrap();
        db.conn.lock().unwrap().execute_batch(&format!("CREATE TRIGGER reject_delete BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT, 'delete failure'); END;")).unwrap();
        assert!(db
            .agent_delete_conversation("c")
            .unwrap_err()
            .contains("delete failure"));
        assert_eq!(
            serde_json::to_value(db.agent_load_messages("c").unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert_eq!(db.agent_list_conversations().unwrap().len(), 2);
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_delete;")
            .unwrap();
        db.agent_delete_conversation("c").unwrap();
        assert!(db.agent_load_messages("c").unwrap().is_empty());
        assert_eq!(db.agent_list_conversations().unwrap()[0].id, "other");
        db.agent_delete_conversation("c").unwrap();
    }
}

#[test]
fn independent_sqlite_writers_cannot_leave_a_late_message_after_deletion() {
    use std::sync::{Arc, Barrier};
    let temporary = Temporary::new();
    let append_db = Arc::new(Database::open(&temporary.0).unwrap());
    let delete_db = Arc::new(Database::open(&temporary.0).unwrap());
    for db in [&append_db, &delete_db] {
        db.conn
            .lock()
            .unwrap()
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
    }
    for round in 0..32 {
        let id = format!("race-{round}");
        append_db.agent_create_conversation(&id, "race").unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let writer = append_db.clone();
        let writer_id = id.clone();
        let writer_gate = barrier.clone();
        let write = std::thread::spawn(move || {
            writer_gate.wait();
            writer.agent_append_message(&writer_id, "assistant", "late answer", None, None, None)
        });
        let deleter = delete_db.clone();
        let delete_id = id.clone();
        let delete = std::thread::spawn(move || {
            barrier.wait();
            deleter.agent_delete_conversation(&delete_id)
        });
        // Either the append commits first and deletion removes it, or the
        // deletion commits first and the guarded append returns an error.
        let _ = write.join().unwrap();
        delete.join().unwrap().unwrap();
        assert!(append_db.agent_load_messages(&id).unwrap().is_empty());
    }
    assert!(append_db.agent_list_conversations().unwrap().is_empty());
}

#[test]
#[ignore = "manual SQLite write comparison, not application CPU or storage durability"]
fn benchmark_agent_message_writes() {
    fn old_append(db: &Database, id: &str, text: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO agent_messages (conv_id, role, content, images_json, tool_name, tool_result_json, created_at) VALUES (?1, 'assistant', ?2, NULL, NULL, NULL, ?3)", params![id,text,epoch_secs()]).unwrap();
        conn.execute(
            "UPDATE agent_conversations SET updated_at=?2 WHERE id=?1",
            params![id, epoch_secs()],
        )
        .unwrap();
    }
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    for id in ["old", "transaction"] {
        db.agent_create_conversation(id, "bench").unwrap();
    }
    let text = "日本語 text ".repeat(100);
    let mut before = Vec::new();
    let mut after = Vec::new();
    for round in 0..8 {
        for optimized in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let start = std::time::Instant::now();
            for _ in 0..500 {
                if optimized {
                    db.agent_append_message("transaction", "assistant", &text, None, None, None)
                        .unwrap();
                } else {
                    old_append(&db, "old", &text);
                }
            }
            if round > 0 {
                if optimized {
                    after.push(start.elapsed());
                } else {
                    before.push(start.elapsed());
                }
            }
        }
    }
    before.sort();
    after.sort();
    assert_eq!(db.agent_load_messages("old").unwrap().len(), 4000);
    assert_eq!(db.agent_load_messages("transaction").unwrap().len(), 4000);
    println!("SQLite 500-message batch, 1500-byte text, WAL/NORMAL, debug build, 7 alternating samples: old independent statements {:?}, cached guarded transaction {:?}", before[3], after[3]);
}
