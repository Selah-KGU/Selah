use super::*;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn conversation_list_reports_invalid_rows_instead_of_silently_hiding_a_chat() {
    let temporary =
        Temporary(std::env::temp_dir().join(format!("selah-list-error-{}", uuid::Uuid::new_v4())));
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("valid", "valid").unwrap();
    db.agent_create_conversation("bad", "bad").unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_conversations SET updated_at=x'ff' WHERE id='bad'",
            [],
        )
        .unwrap();
    assert!(db
        .agent_list_conversations()
        .unwrap_err()
        .starts_with("DB read conversation:"));
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_conversations SET updated_at=1 WHERE id='bad'",
            [],
        )
        .unwrap();
    assert_eq!(db.agent_list_conversations().unwrap().len(), 2);
}

#[test]
fn recent_history_matches_the_full_tail_and_preserves_every_attachment() {
    let temporary =
        Temporary(std::env::temp_dir().join(format!("selah-history-{}", uuid::Uuid::new_v4())));
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("long", "history").unwrap();
    db.agent_create_conversation("other", "other").unwrap();
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for n in 0..200 {
            tx.execute(
                "INSERT INTO agent_messages (conv_id, role, content, images_json, tool_name, tool_result_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![if n % 7 == 0 { "other" } else { "long" },
                    if n % 3 == 0 { "tool" } else { "user" }, format!("日本語 message {n}"),
                    Some("[{\"mime\":\"image/png\",\"data_base64\":\"AA==\"}]"),
                    Some("computer_screenshot"), Some("{\"image\":{\"mime\":\"image/png\",\"data_base64\":\"BB==\"}}"),
                    n / 9],
            ).unwrap();
        }
        tx.commit().unwrap();
    }
    let full = db.agent_load_messages("long").unwrap();
    for limit in [0, 1, 12, 200] {
        let recent = db.agent_load_recent_messages("long", limit).unwrap();
        assert_eq!(
            serde_json::to_value(&recent).unwrap(),
            serde_json::to_value(&full[full.len().saturating_sub(limit)..]).unwrap()
        );
    }
    assert!(db
        .agent_load_recent_messages("missing", 12)
        .unwrap()
        .is_empty());
    assert_eq!(db.agent_load_messages("long").unwrap().len(), full.len());
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare("EXPLAIN QUERY PLAN SELECT id, conv_id, role, content, images_json, tool_name, tool_result_json, created_at FROM agent_messages WHERE conv_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2").unwrap();
    let plan = stmt
        .query_map(params!["long", 12], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("idx_agent_messages_conv"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
}

#[test]
fn history_row_errors_are_reported_instead_of_silently_dropping_messages() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-history-error-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("bad", "history").unwrap();
    db.conn.lock().unwrap().execute(
        "INSERT INTO agent_messages (conv_id, role, content, created_at) VALUES ('bad', 'user', x'ff', 0)", [],
    ).unwrap();
    assert!(db
        .agent_load_messages("bad")
        .unwrap_err()
        .starts_with("DB read message:"));
    assert!(db
        .agent_load_recent_messages("bad", 12)
        .unwrap_err()
        .starts_with("DB read message:"));
}

#[test]
#[ignore = "manual timing of production SQLite queries, not an app performance measurement"]
fn benchmark_recent_agent_history_reads() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-history-bench-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("long", "history").unwrap();
    let payload = "x".repeat(16 * 1024);
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for n in 0..2000 {
            tx.execute("INSERT INTO agent_messages (conv_id, role, content, tool_name, tool_result_json, created_at) VALUES ('long', 'tool', '', 'read_browser_page', ?1, ?2)", params![payload, n / 10]).unwrap();
        }
        tx.commit().unwrap();
    }
    let input = db
        .agent_append_message("long", "user", "current", None, None, None)
        .unwrap();
    for _ in 0..32 {
        db.agent_append_message(
            "long",
            "tool",
            "",
            None,
            Some("read_browser_page"),
            Some(&payload),
        )
        .unwrap();
    }
    let mut full_times = Vec::new();
    let mut recent_times = Vec::new();
    for round in 0..8 {
        for recent in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let start = std::time::Instant::now();
            let rows = if recent {
                db.agent_load_turn_prior_messages("long", input, 11)
            } else {
                db.agent_load_messages("long").map(|mut rows| {
                    rows.retain(|row| row.id < input);
                    rows
                })
            }
            .unwrap();
            let elapsed = start.elapsed();
            assert_eq!(rows.len(), if recent { 11 } else { 2000 });
            std::hint::black_box(&rows);
            if round > 0 {
                if recent {
                    recent_times.push(elapsed)
                } else {
                    full_times.push(elapsed)
                }
            }
        }
    }
    full_times.sort();
    recent_times.sort();
    println!("SQLite history read, 2000 prior rows x 16 KiB + 32 foreign rows, debug build, 7 alternating samples: full read + boundary filter median {:?}, prior(11) including input validation median {:?}; tool JSON returned: {} vs {} bytes", full_times[3], recent_times[3], 2000 * payload.len(), 11 * payload.len());
}

#[test]
fn voice_message_failure_rolls_back_the_conversation_and_can_retry() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-voice-transaction-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.conn.lock().unwrap().execute_batch(
        "CREATE TRIGGER fail_voice BEFORE INSERT ON agent_messages BEGIN SELECT RAISE(ABORT, 'simulated write error'); END;"
    ).unwrap();
    assert!(db
        .agent_create_voice_turn("voice", "complete speech")
        .is_err());
    assert!(db.agent_list_conversations().unwrap().is_empty());
    assert!(db.agent_load_messages("voice").unwrap().is_empty());
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_voice;")
        .unwrap();
    let message_id = db
        .agent_create_voice_turn("voice", "complete speech")
        .unwrap();
    assert_eq!(db.agent_list_conversations().unwrap().len(), 1);
    assert_eq!(
        db.agent_load_messages("voice").unwrap()[0].content,
        "complete speech"
    );
    // Repeating admission cannot append a duplicate or overwrite saved speech.
    assert!(db
        .agent_create_voice_turn("voice", "different speech")
        .is_err());
    let history = db.agent_load_messages("voice").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, message_id);
    assert_eq!(history[0].content, "complete speech");
}

#[test]
fn retry_recovers_failed_or_unacknowledged_voice_commits_without_replacing_other_messages() {
    let temporary =
        Temporary(std::env::temp_dir().join(format!("selah-voice-retry-{}", uuid::Uuid::new_v4())));
    let db = Database::open(&temporary.0).unwrap();
    let message_id = db.agent_retry_voice_turn("voice", "first speech").unwrap();
    assert_eq!(
        db.agent_retry_voice_turn("voice", "first speech").unwrap(),
        message_id
    );
    assert_eq!(db.agent_load_messages("voice").unwrap().len(), 1);
    db.agent_rename_conversation("voice", "manually renamed")
        .unwrap();
    db.agent_append_message("voice", "user", "follow-up", None, None, None)
        .unwrap();
    assert_eq!(
        db.agent_retry_voice_turn("voice", "first speech").unwrap(),
        message_id
    );
    assert!(db.agent_retry_voice_turn("voice", "wrong speech").is_err());
    let history = db.agent_load_messages("voice").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, message_id);
    assert_eq!(history[0].content, "first speech");
    assert_eq!(history[1].content, "follow-up");
    assert_eq!(
        db.agent_list_conversations().unwrap()[0].title,
        "manually renamed"
    );
}
