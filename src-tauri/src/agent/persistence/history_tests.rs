use super::*;
use crate::agent_turn_scope::RunningTurn;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn followup_prompts_keep_owned_tools_and_images_without_other_requests_late_writes() {
    let id = format!("history-scope-{}", uuid::Uuid::new_v4());
    let temporary = Temporary(std::env::temp_dir().join(&id));
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation(&id, "test").unwrap();
    persist_tool_result(
        &db,
        &id,
        "any_tool",
        json!({"image":{"mime":"image/png", "data_base64":"previous-image"}}),
        None,
    )
    .unwrap();
    for n in 0..20 {
        db.agent_append_message(
            &id,
            "assistant",
            &format!("previous answer {n}"),
            None,
            None,
            None,
        )
        .unwrap();
    }
    let old = RunningTurn::begin(&id, None);
    old.turn
        .set_input_message(
            db.agent_append_message(&id, "user", "old request", None, None, None)
                .unwrap(),
        )
        .unwrap();
    let current = RunningTurn::begin(&id, None);
    let user_text = "current unique input";
    current
        .turn
        .set_input_message(
            db.agent_append_message(
                &id,
                "user",
                user_text,
                Some(r#"[{"mime":"image/png","data_base64":"current-attachment"}]"#),
                None,
                None,
            )
            .unwrap(),
        )
        .unwrap();
    persist_tool_result(
        &db,
        &id,
        "computer_screenshot",
        json!({"image":{"mime":"image/png", "data_base64":"own-image"}}),
        Some(&current.turn),
    )
    .unwrap();
    for n in 0..12 {
        persist_tool_result(
            &db,
            &id,
            "read_browser_page",
            json!({"body":format!("own result {n}")}),
            Some(&current.turn),
        )
        .unwrap();
        persist_answer(&db, &id, "foreign answer", Some(&old.turn)).unwrap();
    }
    persist_answer(&db, &id, "own saved answer", Some(&current.turn)).unwrap();
    db.agent_append_message(&id, "user", "foreign later input", None, None, None)
        .unwrap();
    persist_tool_result(
        &db,
        &id,
        "computer_screenshot",
        json!({"image":{"mime":"image/png", "data_base64":"foreign-image"}}),
        Some(&old.turn),
    )
    .unwrap();

    let history = current.turn.history().unwrap();
    assert_eq!(history.messages.len(), 14);
    assert!(!history
        .messages
        .iter()
        .any(|id| old.turn.history().unwrap().messages.contains(id)));
    // Independent reference: filter the full committed audit history, then
    // compare actual model messages and skip policy rather than just row count.
    let eligible = db
        .agent_load_messages(&id)
        .unwrap()
        .into_iter()
        .filter(|row| row.id < history.input_message || history.messages.contains(&row.id))
        .collect::<Vec<_>>();
    for vision in [false, true] {
        let recent = planning_history_for_turn(&db, &id, vision, &history).unwrap();
        assert_eq!(
            recent.len(),
            CFG.plan_history_turns.max(6) + usize::from(vision)
        );
        for row in &recent {
            assert!(row.id < history.input_message || history.messages.contains(&row.id));
        }
        if vision {
            assert_eq!(
                recent_screenshot_images(&recent, 1)[0].data_base64,
                "own-image"
            );
        }
        for local in [false, true] {
            let before = build_plan_messages_with_note(
                None,
                &eligible,
                user_text,
                false,
                None,
                &AgentTurnContext::default(),
                vision,
                local,
            );
            let after = build_plan_messages_with_note(
                None,
                &recent,
                user_text,
                false,
                None,
                &AgentTurnContext::default(),
                vision,
                local,
            );
            assert_eq!(
                serde_json::to_value(&after[1..]).unwrap(),
                serde_json::to_value(&before[1..]).unwrap()
            );
            let joined = after
                .iter()
                .skip(1)
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(joined.matches(user_text).count(), 1);
            assert!(!joined.contains("foreign"));
        }
        for text in ["ありがとう", "继续", "好，点开", "詳しく"] {
            assert_eq!(
                should_skip_tools(&recent, text),
                should_skip_tools(&eligible, text)
            );
        }
    }
    // Existing tools/answers remain in audit history; only model context is isolated.
    assert!(db
        .agent_load_messages(&id)
        .unwrap()
        .iter()
        .any(|row| row.content == "foreign answer"));
    let conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_results BEFORE INSERT ON agent_messages BEGIN SELECT RAISE(ABORT, 'failed save'); END;").unwrap();
    assert!(persist_answer(&db, &id, "failed answer", Some(&current.turn)).is_err());
    assert!(persist_tool_result(
        &db,
        &id,
        "read_browser_page",
        json!({"body":"failed tool"}),
        Some(&current.turn)
    )
    .is_err());
    assert_eq!(current.turn.history().unwrap().messages, history.messages);
    assert_eq!(
        serde_json::to_value(planning_history_for_turn(&db, &id, true, &history).unwrap()).unwrap(),
        serde_json::to_value(
            planning_history_for_turn(&db, &id, true, &current.turn.history().unwrap()).unwrap()
        )
        .unwrap()
    );
}

#[test]
fn scoped_planning_checks_input_and_keeps_older_image_in_the_same_snapshot() {
    let id = format!("snapshot-scope-{}", uuid::Uuid::new_v4());
    let temporary = Temporary(std::env::temp_dir().join(&id));
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation(&id, "test").unwrap();
    let running = RunningTurn::begin(&id, None);
    running
        .turn
        .set_input_message(
            db.agent_append_message(&id, "user", "input", None, None, None)
                .unwrap(),
        )
        .unwrap();
    persist_tool_result(
        &db,
        &id,
        "any_tool",
        json!({"image":{"mime":"image/png", "data_base64":"own-image"}}),
        Some(&running.turn),
    )
    .unwrap();
    for _ in 0..10 {
        persist_answer(&db, &id, "own answer", Some(&running.turn)).unwrap();
    }
    let history = running.turn.history().unwrap();
    let invalid = crate::agent_turn_scope::History {
        input_message: i64::MAX,
        messages: history.messages.clone(),
    };
    assert!(planning_history_for_turn(&db, &id, true, &invalid).is_err());
    assert!(planning_history_for_turn(&db, "other", true, &history).is_err());
    let deleted = std::cell::Cell::new(false);
    let predicate = |json: &str| {
        if tool_result::has_screenshot_image(json) && !deleted.replace(true) {
            db.agent_delete_conversation(&id).unwrap();
        }
        tool_result::has_screenshot_image(json)
    };
    let snapshot = db
        .agent_load_turn_planning_messages(&id, 8, Some(&predicate), &history)
        .unwrap();
    assert!(deleted.get());
    assert_eq!(snapshot.len(), 9);
    assert_eq!(
        recent_screenshot_images(&snapshot, 1)[0].data_base64,
        "own-image"
    );
    assert!(planning_history_for_turn(&db, &id, true, &history).is_err());
    assert!(db.agent_load_messages(&id).unwrap().is_empty());
}
