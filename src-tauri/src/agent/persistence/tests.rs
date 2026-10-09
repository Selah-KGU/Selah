use super::*;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn tool_storage_retains_owned_screenshots_and_reports_write_failures() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-tool-storage-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "test").unwrap();
    let result =
        json!({"image":{"mime":"image/png", "data_base64":"A".repeat(100_000)}, "text":"日本語"});
    let image_ptr = result["image"]["data_base64"].as_str().unwrap().as_ptr();
    let result = persist_tool_result(&db, "c", "computer_screenshot", result, None).unwrap();
    assert_eq!(
        result["image"]["data_base64"].as_str().unwrap().as_ptr(),
        image_ptr
    );
    let rows = db.agent_load_messages("c").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool_name.as_deref(), Some("computer_screenshot"));
    assert_eq!(
        serde_json::from_str::<Value>(rows[0].tool_result_json.as_deref().unwrap()).unwrap(),
        result
    );

    // A failed tool history write must not look like a successfully saved result.
    let invalid_dir = Temporary(
        std::env::temp_dir().join(format!("selah-tool-failure-{}", uuid::Uuid::new_v4())),
    );
    let failed_db = Database::open(&invalid_dir.0).unwrap();
    failed_db.agent_create_conversation("c", "test").unwrap();
    // Install a failing trigger in a temporary DB through another connection.
    let conn = rusqlite::Connection::open(invalid_dir.0.join("courses.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_tools BEFORE INSERT ON agent_messages BEGIN SELECT RAISE(ABORT, 'tool write failed'); END;").unwrap();
    assert!(matches!(
        persist_tool_result(&failed_db, "c", "computer_screenshot", result, None),
        Err(AgentError::DbError(_))
    ));
    assert!(failed_db.agent_load_messages("c").unwrap().is_empty());
}

#[test]
fn bounded_initial_context_preserves_planning_and_click_selection() {
    let mut history = (0..50)
        .map(|n| crate::db::AgentMessageRow {
            id: n,
            conv_id: "c".into(),
            role: "assistant".into(),
            content: format!("message {n}"),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: n,
        })
        .collect::<Vec<_>>();
    history[38].content = "1. 「履修案内」\n2. 「資料一覧」".into();
    history[49].role = "user".into();
    history[49].content = "2".into();
    // Compare the old full-history behavior (current user at the tail) with
    // SQL's prior-only window. The current input is passed separately once.
    let prior = &history[..history.len() - 1];
    let recent = &prior[prior.len() - (BROWSER_CLICK_HISTORY_ROWS - 1)..];
    assert!(!browser_click_labels_for_turn(&history, "2").is_empty());
    assert_eq!(
        browser_click_labels_for_turn(recent, "2"),
        browser_click_labels_for_turn(&history, "2")
    );
    let old_slice = &prior[prior.len() - CFG.history_window..];
    let new_slice = slice_history(recent, CFG.history_window);
    assert_eq!(
        serde_json::to_value(old_slice).unwrap(),
        serde_json::to_value(new_slice).unwrap()
    );
    assert_eq!(new_slice.as_ptr(), recent.as_ptr().wrapping_add(1));
    assert!(slice_history(&[], 10).is_empty());
    assert_eq!(slice_history(&history[..1], 10).len(), 1);
    for local in [true, false] {
        let before = build_plan_messages_with_note(
            None,
            old_slice,
            "2",
            false,
            None,
            &AgentTurnContext::default(),
            true,
            local,
        );
        let after = build_plan_messages_with_note(
            None,
            new_slice,
            "2",
            false,
            None,
            &AgentTurnContext::default(),
            true,
            local,
        );
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn followup_read_keeps_an_older_screenshot() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-follow-history-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "test").unwrap();
    persist_tool_result(
        &db,
        "c",
        "computer_screenshot",
        json!({"image":{"mime":"image/png", "data_base64":"old screenshot"}}),
        None,
    )
    .unwrap();
    for _ in 0..20 {
        db.agent_append_message("c", "assistant", "newer text", None, None, None)
            .unwrap();
    }
    let rows = prepare::blocking(move || planning_history(&db, "c", true))
        .await
        .unwrap();
    assert_eq!(rows.len(), CFG.plan_history_turns.max(6) + 1);
    assert_eq!(
        recent_screenshot_images(&rows, 1)[0].data_base64,
        "old screenshot"
    );
}

#[test]
fn bounded_followup_context_matches_full_history_for_local_and_vision_planning() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-planning-context-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "test").unwrap();
    persist_tool_result(
        &db,
        "c",
        "any_tool",
        json!({"image":{"mime":"image/png","data_base64":"old screenshot"}}),
        None,
    )
    .unwrap();
    for _ in 0..30 {
        db.agent_append_message("c", "assistant", "old text", None, None, None)
            .unwrap();
    }
    persist_tool_result(
        &db,
        "c",
        "read_browser_page",
        json!({"title":"日本語 page", "headings":["資料"]}),
        None,
    )
    .unwrap();
    for text in ["資料一覧", "最新の説明"] {
        db.agent_append_message("c", "assistant", text, None, None, None)
            .unwrap();
    }
    let full = db.agent_load_messages("c").unwrap();
    for vision in [false, true] {
        let recent = planning_history(&db, "c", vision).unwrap();
        assert!(recent.len() <= CFG.plan_history_turns.max(6) + 1);
        for local in [false, true] {
            let before = build_plan_messages_with_note(
                None,
                &full,
                "続けて",
                false,
                None,
                &AgentTurnContext::default(),
                vision,
                local,
            );
            let after = build_plan_messages_with_note(
                None,
                &recent,
                "続けて",
                false,
                None,
                &AgentTurnContext::default(),
                vision,
                local,
            );
            // System date/time is independent of history and can tick during a test.
            assert_eq!(
                serde_json::to_value(&before[1..]).unwrap(),
                serde_json::to_value(&after[1..]).unwrap()
            );
        }
        for text in ["ありがとう", "继续", "好，点开", "詳しく"] {
            assert_eq!(
                should_skip_tools(&recent, text),
                should_skip_tools(&full, text)
            );
        }
    }
}

#[test]
#[ignore = "manual planning-context preparation comparison, not whole-app performance"]
fn benchmark_planning_history_reads() {
    use rusqlite::params;
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-planning-benchmark-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "history").unwrap();
    let payload = serde_json::json!({"body":"x".repeat(16 * 1024)}).to_string();
    let mut conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    {
        let tx = conn.transaction().unwrap();
        for n in 0..2000 {
            tx.execute("INSERT INTO agent_messages(conv_id,role,content,tool_name,tool_result_json,created_at) VALUES ('c','tool','', 'read_browser_page',?1,?2)", params![payload,n / 9]).unwrap();
        }
        tx.commit().unwrap();
    }
    let input_message = db
        .agent_append_message("c", "user", "inspect page", None, None, None)
        .unwrap();
    let messages = (0..2)
        .map(|_| {
            db.agent_append_message("c", "assistant", "owned answer", None, None, None)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let history = crate::agent_turn_scope::History {
        input_message,
        messages,
    };
    let foreign = json!({"body":"x".repeat(16 * 1024), "image":{"mime":"image/png","data_base64":"foreign screenshot"}}).to_string();
    for _ in 0..32 {
        db.agent_append_message(
            "c",
            "tool",
            "",
            None,
            Some("computer_screenshot"),
            Some(&foreign),
        )
        .unwrap();
    }
    let image = r#"{"image":{"mime":"image/png","data_base64":"older screenshot"}}"#;
    for (name, image_id, vision) in [
        ("just outside text window", Some(1990), true),
        ("first message", Some(1), true),
        ("no image", None, true),
        ("text model", None, false),
    ] {
        conn.execute(
            "UPDATE agent_messages SET tool_result_json=?1 WHERE id IN (1,1990)",
            params![payload],
        )
        .unwrap();
        if let Some(id) = image_id {
            conn.execute(
                "UPDATE agent_messages SET tool_result_json=?1 WHERE id=?2",
                params![image, id],
            )
            .unwrap();
        }
        let mut before = Vec::new();
        let mut after = Vec::new();
        for round in 0..8 {
            for optimized in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = std::time::Instant::now();
                let rows = if optimized {
                    planning_history_for_turn(&db, "c", vision, &history).unwrap()
                } else {
                    let mut rows = db.agent_load_messages("c").unwrap();
                    rows.retain(|row| {
                        row.id < history.input_message || history.messages.contains(&row.id)
                    });
                    rows
                };
                let messages = build_plan_messages_with_note(
                    None,
                    &rows,
                    "inspect page",
                    false,
                    None,
                    &AgentTurnContext::default(),
                    vision,
                    false,
                );
                std::hint::black_box((&rows, &messages));
                assert_eq!(
                    messages.last().unwrap().images.len(),
                    usize::from(image_id.is_some())
                );
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
        println!("Planning context including production image lookup + prompt, 2000 prior rows x 16 KiB + 2 own answers + 32 foreign screenshots, {name}, debug build, 7 alternating samples: full read + eligibility filter {:?}, scoped bounded snapshot {:?}", before[3], after[3]);
    }
}
