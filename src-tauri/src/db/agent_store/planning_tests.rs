use super::*;

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-planning-history-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn image(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .is_some_and(|value| {
            value
                .pointer("/image/mime")
                .and_then(serde_json::Value::as_str)
                .is_some()
                && value
                    .pointer("/image/data_base64")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        })
}
fn seed(db: &Database) {
    for conv in ["c", "other"] {
        db.agent_create_conversation(conv, "history").unwrap();
    }
    for n in 0..60 {
        let json = match n {
            0 | 12 | 20 => Some(format!(
                r#"{{"image":{{"mime":"image/png","data_base64":"image-{n}"}}}}"#
            )),
            21 => Some("{broken".into()),
            22 => Some(r#"{"image":{"mime":1,"data_base64":"not an image"}}"#.into()),
            _ => None,
        };
        db.agent_append_message(
            "c",
            if json.is_some() { "tool" } else { "assistant" },
            &format!("text-{n}"),
            None,
            Some("any_tool"),
            json.as_deref(),
        )
        .unwrap();
    }
    db.agent_append_message(
        "other",
        "tool",
        "",
        None,
        Some("screenshot"),
        Some(r#"{"image":{"mime":"image/png","data_base64":"wrong conversation"}}"#),
    )
    .unwrap();
    // Tie timestamps and nonchronological insertion both exercise the exact
    // (created_at,id) cursor, not just a row-ID or OFFSET shortcut.
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE agent_messages SET created_at=id / 9", [])
        .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_messages SET created_at=-1 WHERE conv_id='c' AND id=60",
            [],
        )
        .unwrap();
}

#[test]
fn planning_reads_only_the_text_tail_and_the_latest_matching_older_tool() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    seed(&db);
    let full = db.agent_load_messages("c").unwrap();
    let recent = db
        .agent_load_planning_messages("c", 8, Some(&image))
        .unwrap();
    assert_eq!(recent.len(), 9);
    assert_eq!(
        serde_json::to_value(&recent[1..]).unwrap(),
        serde_json::to_value(&full[52..]).unwrap()
    );
    assert!(recent[0]
        .tool_result_json
        .as_deref()
        .unwrap()
        .contains("image-20"));
    let text_only = db.agent_load_planning_messages("c", 8, None).unwrap();
    assert_eq!(
        serde_json::to_value(text_only).unwrap(),
        serde_json::to_value(&full[52..]).unwrap()
    );
    assert_eq!(
        db.agent_load_planning_messages("c", 100, Some(&image))
            .unwrap()
            .len(),
        60
    );
    assert!(db
        .agent_load_planning_messages("c", 0, Some(&image))
        .unwrap()
        .is_empty());
    assert!(db
        .agent_load_planning_messages("missing", 8, Some(&image))
        .unwrap()
        .is_empty());
    assert_eq!(db.agent_load_messages("c").unwrap().len(), 60);
    // A recent image must not cause the older image to be appended as well.
    db.agent_append_message(
        "c",
        "tool",
        "",
        None,
        Some("screenshot"),
        Some(r#"{"image":{"mime":"image/png","data_base64":"new"}}"#),
    )
    .unwrap();
    assert_eq!(
        db.agent_load_planning_messages("c", 8, Some(&image))
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn private_reader_releases_the_shared_mutex_and_keeps_a_consistent_snapshot() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    seed(&db);
    let deleted = std::cell::Cell::new(false);
    let predicate = |json: &str| {
        assert!(
            db.conn.try_lock().is_ok(),
            "image lookup held the shared DB mutex"
        );
        if image(json) && !deleted.replace(true) {
            db.agent_delete_conversation("c").unwrap();
        }
        image(json)
    };
    let snapshot = db
        .agent_load_planning_messages("c", 8, Some(&predicate))
        .unwrap();
    assert!(deleted.get());
    assert_eq!(snapshot.len(), 9);
    assert!(snapshot[0]
        .tool_result_json
        .as_deref()
        .unwrap()
        .contains("image-20"));
    assert!(db.agent_load_messages("c").unwrap().is_empty());
}

#[test]
fn planning_reports_row_conversion_errors_and_uses_the_chronological_index() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    seed(&db);
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare("EXPLAIN QUERY PLAN SELECT id, tool_result_json FROM agent_messages WHERE conv_id=?1 AND (created_at,id)<(?2,?3) AND role='tool' AND tool_result_json IS NOT NULL ORDER BY created_at DESC,id DESC").unwrap();
    let plan = stmt
        .query_map(params!["c", 7, 60], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("idx_agent_messages_conv"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    drop(stmt);
    conn.execute(
        "UPDATE agent_messages SET tool_result_json=x'ff' WHERE conv_id='c' AND id=23",
        [],
    )
    .unwrap();
    drop(conn);
    assert!(db
        .agent_load_planning_messages("c", 8, Some(&image))
        .unwrap_err()
        .starts_with("DB read tool result:"));
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_messages SET content=x'ff' WHERE conv_id='c' AND id=59",
            [],
        )
        .unwrap();
    assert!(db
        .agent_load_planning_messages("c", 8, None)
        .unwrap_err()
        .starts_with("DB read message:"));
}

#[test]
fn scoped_reader_filters_payloads_before_decoding_and_keeps_indexed_order() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "test").unwrap();
    let input = db
        .agent_append_message("c", "user", "current", None, None, None)
        .unwrap();
    let screenshot = db
        .agent_append_message(
            "c",
            "tool",
            "",
            None,
            Some("screenshot"),
            Some(r#"{"image":{"mime":"image/png","data_base64":"owned"}}"#),
        )
        .unwrap();
    let mut messages = vec![screenshot];
    for _ in 0..10 {
        messages.push(
            db.agent_append_message("c", "assistant", "owned text", None, None, None)
                .unwrap(),
        );
    }
    let foreign = db
        .agent_append_message(
            "c",
            "tool",
            "foreign",
            None,
            Some("screenshot"),
            Some("bad"),
        )
        .unwrap();
    let history = crate::agent_turn_scope::History {
        input_message: input,
        messages,
    };
    let conn = db.conn.lock().unwrap();
    // Invalid excluded fields must never be materialized into model history.
    conn.execute(
        "UPDATE agent_messages SET content=x'ff',tool_result_json=x'ff' WHERE id IN (?1,?2)",
        params![input, foreign],
    )
    .unwrap();
    let own_ids = serde_json::to_string(&history.messages).unwrap();
    for (sql, args) in [
        (
            RECENT_PLANNING_SQL,
            vec![
                rusqlite::types::Value::Text("c".into()),
                8_i64.into(),
                input.into(),
                own_ids.clone().into(),
            ],
        ),
        (
            OLDER_PLANNING_TOOL_SQL,
            vec![
                rusqlite::types::Value::Text("c".into()),
                i64::MAX.into(),
                foreign.into(),
                input.into(),
                own_ids.into(),
            ],
        ),
    ] {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let plan = stmt
            .query_map(rusqlite::params_from_iter(args), |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        assert!(plan.contains("idx_agent_messages_conv"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
    drop(conn);
    let recent = db
        .agent_load_turn_planning_messages("c", 8, Some(&image), &history)
        .unwrap();
    assert_eq!(recent.len(), 9);
    assert_eq!(recent[0].id, screenshot);
    assert!(recent.iter().all(|row| history.messages.contains(&row.id)));
    // The same corruption on an eligible older candidate must be reported.
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_messages SET tool_result_json=x'ff' WHERE id=?1",
            [screenshot],
        )
        .unwrap();
    assert!(db
        .agent_load_turn_planning_messages("c", 8, Some(&image), &history)
        .unwrap_err()
        .starts_with("DB read tool result:"));
    let newest = *history.messages.last().unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_messages SET content=x'ff' WHERE id=?1",
            [newest],
        )
        .unwrap();
    assert!(db
        .agent_load_turn_planning_messages("c", 8, None, &history)
        .unwrap_err()
        .starts_with("DB read message:"));
}
