//! Dispatch, plan finalization, and prompt budget tests.

use super::*;

#[test]
fn dispatch_known_includes_new_tools() {
    for name in [
        "search_mail",
        "list_luna_announcements",
        "delete_downloaded_file",
        "download_url",
        "browser_close",
        "get_today_brief",
        "get_notification_detail",
    ] {
        assert!(
            agent_tools::is_known_tool(name),
            "tool {} missing from registry",
            name
        );
    }
}

#[test]
fn sanitize_get_notification_detail_args() {
    let args = serde_json::json!({"title": "  休講のお知らせ  "});
    let cleaned = agent_tools::sanitize_tool_args("get_notification_detail", &args).unwrap();
    assert_eq!(
        cleaned.get("title").and_then(|v| v.as_str()),
        Some("休講のお知らせ")
    );

    let empty = serde_json::json!({});
    assert!(agent_tools::sanitize_tool_args("get_notification_detail", &empty).is_none());
}

#[test]
fn all_registered_tools_have_dispatch_arms() {
    let registered =
        agent_tools::registered_tool_names().collect::<std::collections::BTreeSet<_>>();
    let dispatched = agent_tools::dispatched_tool_names()
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        registered, dispatched,
        "TOOL_SPECS and dispatch arms drifted apart"
    );
}

#[test]
fn finalize_plan_reports_rejected_tools_for_repair() {
    let plan = Plan {
        tools: vec![
            ToolCall {
                name: "launch_browser".into(),
                args: serde_json::json!({ "url": "https://example.com" }),
            },
            ToolCall {
                name: "browser_click".into(),
                args: serde_json::json!({}),
            },
            ToolCall {
                name: "open_browser_url".into(),
                args: serde_json::json!({ "url": "https://example.com" }),
            },
            ToolCall {
                name: "read_downloaded_file".into(),
                args: serde_json::json!({ "path": "<PATH_FROM_LIST>" }),
            },
        ],
        image_only: false,
    };
    let finalized =
        finalize_plan_with_diagnostics(plan, &[], "打开 example.com", &AgentTurnContext::default());
    assert_eq!(finalized.unknown_tools, vec!["launch_browser"]);
    assert_eq!(
        finalized.invalid_args,
        vec!["browser_click", "read_downloaded_file"]
    );
    assert_eq!(finalized.plan.tools.len(), 1);
    assert_eq!(finalized.plan.tools[0].name, "open_browser_url");
    assert!(plan_repair_note(&finalized).contains("launch_browser"));
}

#[test]
fn placeholder_detection_does_not_reject_literal_markup() {
    assert!(contains_unresolved_plan_placeholder(
        &json!({"path":"<PATH_FROM_LIST>"})
    ));
    assert!(contains_unresolved_plan_placeholder(
        &json!({"event_id":"<event_id_from_list>"})
    ));
    assert!(!contains_unresolved_plan_placeholder(
        &json!({"value":"<div>"})
    ));
    assert!(!contains_unresolved_plan_placeholder(
        &json!({"value":"<日本語>"})
    ));
}

#[test]
fn finalize_plan_locks_browser_target_for_attached_panel() {
    let plan = Plan {
        tools: vec![
            ToolCall {
                name: "read_browser_page".into(),
                args: serde_json::json!({}),
            },
            ToolCall {
                name: "browser_click".into(),
                args: serde_json::json!({
                    "target": "ext-b-ct",
                    "text": "詳細",
                }),
            },
            ToolCall {
                name: "list_browser_windows".into(),
                args: serde_json::json!({}),
            },
        ],
        image_only: false,
    };
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        browser_click_labels: Vec::new(),
        ..Default::default()
    };
    let finalized = finalize_plan_with_diagnostics(plan, &[], "这个页面看看", &ctx);
    assert_eq!(finalized.plan.tools.len(), 3);
    assert_eq!(
        finalized.plan.tools[0]
            .args
            .get("target")
            .and_then(|v| v.as_str()),
        Some("ext-a-ct")
    );
    assert_eq!(
        finalized.plan.tools[1]
            .args
            .get("target")
            .and_then(|v| v.as_str()),
        Some("ext-a-ct")
    );
    assert!(finalized.plan.tools[2].args.get("target").is_none());
}

#[test]
fn build_plan_messages_structure() {
    let history = vec![
        crate::db::AgentMessageRow {
            id: 1,
            conv_id: "c".into(),
            role: "user".into(),
            content: "天気は？".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 0,
        },
        tool_row("get_weather"),
    ];
    let msgs = build_plan_messages(None, &history, "明日は？", true);
    // system + 1 user history + 1 tool history + current user = 4
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[0].role, "system");
    assert_eq!(msgs.last().unwrap().role, "user");
    assert_eq!(msgs.last().unwrap().content, "明日は？");
}

#[test]
fn build_answer_messages_includes_tool_results() {
    let tool_results = vec![("get_weather".to_string(), serde_json::json!({"temp": 22}))];
    let msgs = build_answer_messages(
        None,
        &[],
        "天気は？",
        &[],
        &tool_results,
        None,
        &AgentTurnContext::default(),
        false,
        false,
    );
    assert_eq!(msgs.len(), 2); // system + user
    assert!(msgs[0].content.contains("tool_results"));
    assert!(msgs[0].content.contains("get_weather"));
    assert!(msgs[0].content.contains("TOOL EXECUTION BOUNDARY"));
    assert!(msgs[0].content.contains("AVAILABLE TOOLS REFERENCE"));
    assert!(msgs[0].content.contains("open_browser_url(url: string)"));
}
#[test]
fn local_answer_omits_tool_catalog_but_keeps_results() {
    let tool_results = vec![("get_weather".to_string(), serde_json::json!({"temp": 22}))];
    let msgs = build_answer_messages(
        None,
        &[],
        "天気は？",
        &[],
        &tool_results,
        None,
        &AgentTurnContext::default(),
        false,
        true,
    );
    assert!(msgs[0].content.contains("get_weather"));
    assert!(!msgs[0].content.contains("open_browser_url(url: string)"));
    assert_eq!(msgs.last().unwrap().content, "天気は？");
}

#[test]
fn local_answer_compacts_tool_json_instead_of_slicing_it() {
    let todos: Vec<_> = (0..40)
        .map(|index| {
            serde_json::json!({
                "title": format!("課題{index}{}", "あ".repeat(40)),
                "course": "科目",
                "deadline": "2026-04-20"
            })
        })
        .collect();
    let tool_results = vec![(
        "list_luna_todos".to_string(),
        serde_json::json!({"todos": todos}),
    )];
    let msgs = build_answer_messages(
        None,
        &[],
        "課題は？",
        &[],
        &tool_results,
        None,
        &AgentTurnContext::default(),
        false,
        true,
    );
    let system = &msgs[0].content;
    let marker = "[list_luna_todos]";
    let start = system.find(marker).expect("tool label");
    let rest = system[start + marker.len()..].trim_start();
    let line = rest.lines().next().expect("json line").trim();
    let parsed: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
        panic!("tool JSON was sliced: {error}: {line}");
    });
    assert!(parsed
        .get("todos")
        .and_then(|value| value.as_array())
        .is_some());
}

#[test]
fn local_plan_fits_apple_window() {
    let msgs = build_plan_messages_with_note(
        None,
        &[],
        "明日の予定は？",
        false,
        None,
        &AgentTurnContext::default(),
        false,
        true,
    );
    let system = &msgs[0].content;
    assert!(system.contains("list_today_classes()"));
    assert!(!system.contains("FAST SELECTION MAP"));
    assert!(
        crate::local_ai::estimate_apple_tokens(system) < crate::local_ai::APPLE_PROMPT_TOKEN_BUDGET
    );
}

#[test]
fn build_answer_messages_budget_limits_history() {
    // Each message is trimmed to 1200 chars (~400 tokens).
    // 200 messages × ~410 tokens = ~82000 > budget of 50000.
    let long_msg = "あ".repeat(20000);
    let history: Vec<crate::db::AgentMessageRow> = (0..200)
        .map(|i| crate::db::AgentMessageRow {
            id: i,
            conv_id: "c".into(),
            role: if i % 2 == 0 {
                "user".into()
            } else {
                "assistant".into()
            },
            content: long_msg.clone(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 0,
        })
        .collect();
    let msgs = build_answer_messages(
        None,
        &history,
        "test",
        &[],
        &[],
        None,
        &AgentTurnContext::default(),
        false,
        false,
    );
    // Budget should prevent ALL 200 history messages from being included.
    assert!(
        msgs.len() < 200,
        "expected truncation, got {} messages",
        msgs.len()
    );
    assert_eq!(msgs.last().unwrap().content, "test");
}
