use super::*;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn exact_input_boundary_survives_clock_rewind_and_later_same_conversation_writes() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-turn-history-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "context").unwrap();
    db.agent_create_conversation("other", "other").unwrap();
    let previous = db
        .agent_append_message("c", "assistant", "previous answer", None, None, None)
        .unwrap();
    let input = db
        .agent_append_message("c", "user", "same text", None, None, None)
        .unwrap();
    let late = db
        .agent_append_message("c", "user", "same text", None, None, None)
        .unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "UPDATE agent_messages SET created_at=900 WHERE id=?1",
        [previous],
    )
    .unwrap();
    conn.execute(
        "UPDATE agent_messages SET created_at=100 WHERE id=?1",
        [input],
    )
    .unwrap();
    conn.execute(
        "UPDATE agent_messages SET created_at=50 WHERE id=?1",
        [late],
    )
    .unwrap();
    drop(conn);
    let rows = db.agent_load_turn_prior_messages("c", input, 11).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, previous);
    assert_eq!(rows[0].content, "previous answer");
    assert!(db
        .agent_load_turn_prior_messages("other", input, 11)
        .is_err());
    assert!(db
        .agent_load_turn_prior_messages("c", previous, 11)
        .is_err());
    assert!(db
        .agent_load_turn_prior_messages("c", i64::MAX, 11)
        .is_err());
    assert!(db
        .agent_load_turn_prior_messages("c", input, 0)
        .unwrap()
        .is_empty());
    db.agent_delete_conversation("c").unwrap();
    assert!(db.agent_load_turn_prior_messages("c", input, 11).is_err());
}

#[test]
fn prior_window_matches_full_eligible_history_and_does_not_decode_excluded_rows() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-prior-window-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "context").unwrap();
    db.agent_create_conversation("other", "context").unwrap();
    for n in 0..80 {
        db.agent_append_message(
            "c",
            ["user", "assistant", "tool"][n % 3],
            &format!("全文-{n}"),
            Some(r#"[{"mime":"image/png","data_base64":"image"}]"#),
            Some("any_tool"),
            Some(r#"{"body":"complete result"}"#),
        )
        .unwrap();
    }
    let input = db
        .agent_append_message("c", "user", "current", None, None, None)
        .unwrap();
    let late = db
        .agent_append_message("c", "tool", "late", None, Some("tool"), None)
        .unwrap();
    let other = db
        .agent_append_message("other", "user", "other", None, None, None)
        .unwrap();
    {
        let conn = db.conn.lock().unwrap();
        // Include tied seconds, reverse insertion order and a clock rewind.
        conn.execute("UPDATE agent_messages SET created_at=(id % 9) - 4", [])
            .unwrap();
        // Excluded payloads must not be read or converted, including this input.
        conn.execute(
            "UPDATE agent_messages SET content=x'ff', images_json=x'ff' WHERE id IN (?1,?2,?3)",
            params![input, late, other],
        )
        .unwrap();
    }
    let prior = {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,conv_id,role,content,images_json,tool_name,tool_result_json,created_at FROM agent_messages WHERE conv_id='c' AND id<?1 ORDER BY created_at,id").unwrap();
        stmt.query_map([input], message_row)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    for limit in [0, 1, 10, 11, 30, 80, 100] {
        let recent = db
            .agent_load_turn_prior_messages("c", input, limit)
            .unwrap();
        let expected = &prior[prior.len().saturating_sub(limit)..];
        assert_eq!(
            serde_json::to_value(recent).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {PRIOR_HISTORY_SQL}"))
        .unwrap();
    let plan = stmt
        .query_map(params!["c", input, 11], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("idx_agent_messages_conv"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    drop(stmt);
    let newest = prior.last().unwrap().id;
    conn.execute(
        "UPDATE agent_messages SET content=x'ff' WHERE id=?1",
        [newest],
    )
    .unwrap();
    drop(conn);
    assert!(db
        .agent_load_turn_prior_messages("c", input, 11)
        .unwrap_err()
        .starts_with("DB input history read:"));
    assert!(db
        .agent_load_turn_prior_messages("c", input, usize::MAX)
        .is_err());
}
