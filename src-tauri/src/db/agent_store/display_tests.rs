use super::*;

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-display-history-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn display_history_matches_all_visible_rows_and_preserves_full_saved_history() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    for id in ["c", "other"] {
        db.agent_create_conversation(id, id).unwrap();
    }
    let image = r#"[{"mime":"image/png","data_base64":"AA=="}]"#;
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for n in 0..1_600 {
            let role = ["user", "tool", "assistant", "system"][n % 4];
            tx.execute("INSERT INTO agent_messages(conv_id,role,content,images_json,tool_name,tool_result_json,created_at) VALUES (?1,?2,?3,?4,'example',?5,?6)",
                params![if n % 9 == 0 { "other" } else { "c" }, role, format!("全文 あ🌕 {n}"), image, r#"{"body":"hidden result"}"#, n / 7]).unwrap();
        }
        // Match full history's ordering even when the clock moves backwards.
        tx.execute("UPDATE agent_messages SET created_at=-1 WHERE id=1599", [])
            .unwrap();
        tx.commit().unwrap();
    }
    let full = db.agent_load_messages("c").unwrap();
    let expected = full
        .iter()
        .filter(|row| matches!(row.role.as_str(), "user" | "assistant"))
        .cloned()
        .map(|mut row| {
            row.tool_name = None;
            row.tool_result_json = None;
            row
        })
        .collect::<Vec<_>>();
    assert!(
        expected.len() > 120,
        "visible history must not be truncated"
    );
    let actual = db.agent_load_display_messages("c").unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(actual[0].id, 1599);
    assert!(actual
        .iter()
        .all(|row| row.images_json.as_deref() == Some(image)));
    assert_eq!(
        serde_json::to_value(db.agent_load_messages("c").unwrap()).unwrap(),
        serde_json::to_value(&full).unwrap()
    );
    assert!(db
        .agent_load_display_messages("missing")
        .unwrap()
        .is_empty());
    // Existing installations have the message tables but no display index.
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP INDEX idx_agent_messages_display;")
        .unwrap();
    let reopened = Database::open(&temporary.0).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.agent_load_messages("c").unwrap()).unwrap(),
        serde_json::to_value(&full).unwrap()
    );
    assert_eq!(
        serde_json::to_value(reopened.agent_load_display_messages("c").unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    // Inspect the connection that performed startup migration. EXPLAIN on an
    // older, idle connection may still compile against its cached schema.
    let conn = reopened.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {DISPLAY_HISTORY_SQL}"))
        .unwrap();
    let plan = stmt
        .query_map(params!["c"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("idx_agent_messages_display"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
}

#[test]
fn display_query_never_reads_hidden_rows_or_columns_and_reports_visible_corruption() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "c").unwrap();
    db.conn.lock().unwrap().execute_batch("INSERT INTO agent_messages(conv_id,role,content,images_json,tool_name,tool_result_json,created_at)
        VALUES ('c','tool',x'ff',x'ff',x'ff',x'ff',0), ('c','system',x'ff',x'ff',x'ff',x'ff',0),
        ('c','user','visible input',NULL,x'ff',x'ff',0);").unwrap();
    assert!(db.agent_load_messages("c").is_err());
    let visible = db.agent_load_display_messages("c").unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].content, "visible input");
    assert!(visible[0].tool_name.is_none());
    assert!(visible[0].tool_result_json.is_none());
    for column in ["content", "images_json"] {
        db.conn
            .lock()
            .unwrap()
            .execute_batch(&format!(
                "UPDATE agent_messages SET {column}=x'ff' WHERE role='user';"
            ))
            .unwrap();
        assert!(db
            .agent_load_display_messages("c")
            .unwrap_err()
            .starts_with("DB display history message:"));
        db.conn
            .lock()
            .unwrap()
            .execute_batch("UPDATE agent_messages SET content='visible input',images_json=NULL WHERE role='user';")
            .unwrap();
    }
}
