//! Browser observation and click-plan tests.

use super::*;

#[test]
fn smalltalk_skips_tools() {
    assert!(should_skip_tools(&[], "你好"));
    assert!(should_skip_tools(&[], "あなたは誰？"));
    assert!(should_skip_tools(&[], "hello"));
}

#[test]
fn detail_or_referential_follow_up_runs_tools_again() {
    // Even when recent tool context exists, follow-ups that ask for more
    // detail or refer ambiguously ("那个呢？") should re-plan rather than
    // silently reuse stale context — false positives there give wrong
    // answers. Only explicit acknowledgments skip tools; see
    // `follow_up_with_thanks_skips_tools` for that case.
    let history = vec![tool_row("list_today_classes")];
    assert!(!should_skip_tools(&history, "那个呢？"));
    assert!(!should_skip_tools(&history, "もう少し詳しく"));
}

#[test]
fn deterministic_weather_plan() {
    let plan = heuristic_plan(&[], "明日の天気は？").expect("plan");
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "get_weather");
}

#[test]
fn heuristic_opens_luna_with_real_browser_tool() {
    let plan = heuristic_plan(&[], "打开luna看看").expect("plan");
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "open_browser_url");
    assert_eq!(
        plan.tools[0].args.get("url").and_then(|v| v.as_str()),
        Some(crate::config::LUNA_BASE)
    );
}

#[test]
fn heuristic_does_not_reopen_kwic_for_browser_clicks() {
    let plan = heuristic_plan(&[], "浏览器点击kwic的最新通知");
    assert!(
        plan.is_none(),
        "browser click intent should go through the full planner with current browser context"
    );
}

#[test]
fn attached_browser_panel_home_request_starts_with_screenshot() {
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        browser_click_labels: Vec::new(),
        ..Default::default()
    };
    let plan = attached_browser_control_plan(&normalize_planner_text("回到首页"), &ctx)
        .expect("attached browser plan");
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "computer_screenshot");
}

#[test]
fn attached_browser_panel_click_label_starts_with_screenshot() {
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        browser_click_labels: vec!["home".into()],
        ..Default::default()
    };
    let plan = attached_browser_control_plan(&normalize_planner_text("点啊"), &ctx)
        .expect("attached browser click plan");
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "computer_screenshot");
}

#[test]
fn explicit_fill_continues_after_page_observation() {
    let plan = single_tool_plan("read_browser_page", json!({}));
    let results = vec![(
        "read_browser_page".into(),
        json!({"inputs":[{"label":"名前"}]}),
    )];
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        ..Default::default()
    };
    assert!(should_continue_after_browser_observation(
        &plan,
        &results,
        "名前にSelahと入力して",
        &ctx
    ));
    assert!(should_continue_after_browser_observation(
        &plan,
        &results,
        "fill the name field",
        &ctx
    ));
}

#[test]
fn browser_observation_does_not_continue_after_action_or_for_read_only_request() {
    let observation = single_tool_plan("read_browser_page", json!({}));
    let clicked = vec![
        ("read_browser_page".into(), json!({"buttons":["次へ"]})),
        ("browser_click".into(), json!({"ok":true})),
    ];
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        ..Default::default()
    };
    assert!(!should_continue_after_browser_observation(
        &observation,
        &clicked,
        "次へをクリック",
        &ctx
    ));
    assert!(!should_continue_after_browser_observation(
        &observation,
        &[("read_browser_page".into(), json!({"text":"本文"}))],
        "このページを要約して",
        &ctx
    ));
}

#[test]
fn failed_browser_action_can_reobserve_and_continue_safely() {
    let attempted = single_tool_plan("browser_click", json!({"text":"次へ"}));
    let results = vec![
        ("browser_click".into(), json!({"error":"not found"})),
        (
            "read_browser_page".into(),
            json!({"buttons":[{"text":"続ける"}]}),
        ),
    ];
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        ..Default::default()
    };
    assert!(should_continue_after_browser_observation(
        &attempted,
        &results,
        "次へをクリック",
        &ctx
    ));
    assert!(is_browser_mutation_tool("browser_click"));
    assert!(is_browser_mutation_tool("browser_fill"));
    assert!(!is_browser_mutation_tool("read_browser_page"));
    assert!(!is_browser_mutation_tool("browser_wait_for"));
}

#[test]
fn lookup_followup_candidates_are_scoped_by_fresh_result_type() {
    let calendar = vec![(
        "list_google_calendar_events".into(),
        json!({"events":[{"event_id":"event-1","title":"試験"}]}),
    )];
    assert_eq!(
        allowed_lookup_followup_actions(&calendar),
        vec![
            "delete_google_calendar_event",
            "update_google_calendar_event"
        ]
    );
    assert!(should_continue_after_actionable_lookup(&calendar));

    let files = vec![(
        "list_downloaded_files".into(),
        json!({"files":[{"path":"/tmp/a.pdf"}]}),
    )];
    assert_eq!(
        allowed_lookup_followup_actions(&files),
        vec![
            "open_downloaded_file",
            "delete_downloaded_file",
            "read_downloaded_file"
        ]
    );

    let luna = vec![(
        "list_luna_todos".into(),
        json!({"todos":[{"title":"第7回課題","luna_id":"LUNA-42"}]}),
    )];
    assert_eq!(
        allowed_lookup_followup_actions(&luna),
        vec![
            "get_luna_activity_detail",
            "open_copilot_page",
            "open_luna_attachment",
            "download_luna_attachment",
            "download_course_material"
        ]
    );

    let notifications = vec![(
        "search_notifications".into(),
        json!({"notifications":[{"title":"履修登録のお知らせ"}]}),
    )];
    assert_eq!(
        allowed_lookup_followup_actions(&notifications),
        vec!["get_notification_detail", "open_copilot_page"]
    );

    let notification_detail = vec![(
        "get_notification_detail".into(),
        json!({"source":"KWIC","title":"履修登録のお知らせ"}),
    )];
    assert_eq!(
        allowed_lookup_followup_actions(&notification_detail),
        vec!["open_copilot_page"]
    );

    let luna_detail = vec![
        (
            "list_luna_todos".into(),
            json!({"todos":[{"title":"第7回課題","luna_id":"LUNA-42"}]}),
        ),
        (
            "get_luna_activity_detail".into(),
            json!({"matched_title":"第7回課題","activity_type":"report"}),
        ),
    ];
    assert!(!allowed_lookup_followup_actions(&luna_detail).contains(&"get_luna_activity_detail"));
}

#[test]
fn planner_summaries_keep_dynamic_action_identifiers() {
    let calendar = summarize_plan_tool_result(
        "list_google_calendar_events",
        r#"{"events":[{"event_id":"event-123","title":"試験","date":"2026-06-15","start_time":"10:00","end_time":"11:00"}]}"#,
    );
    assert!(calendar.contains("event-123"));

    let files = summarize_plan_tool_result(
        "list_downloaded_files",
        r#"{"files":[{"path":"/tmp/lecture.pdf","filename":"lecture.pdf"}]}"#,
    );
    assert!(files.contains("/tmp/lecture.pdf"));

    let detail = summarize_plan_tool_result(
        "get_luna_activity_detail",
        r#"{"matched_title":"第7回課題","period":"2026-06-20","source":{"luna_id":"LUNA-42"},"attachments":[{"name":"instructions.pdf"},{"name":"answer.docx"}]}"#,
    );
    assert!(detail.contains("title=第7回課題"));
    assert!(detail.contains("luna_id=LUNA-42"));
    assert!(detail.contains("instructions.pdf / answer.docx"));

    let announcements = summarize_plan_tool_result(
        "list_luna_announcements",
        r#"{"announcements":[{"title":"第7回資料","course":"政治学","period":"2026-06-12","luna_id":"LUNA-42"}]}"#,
    );
    assert!(announcements.contains("title=第7回資料"));
    assert!(announcements.contains("luna_id=LUNA-42"));

    let notifications = summarize_plan_tool_result(
        "search_notifications",
        r#"{"notifications":[{"source":"KWIC","identifier":"notice-7","title":"履修登録"}]}"#,
    );
    assert!(notifications.contains("source=KWIC"));
    assert!(notifications.contains("identifier=notice-7"));

    let browser = summarize_plan_tool_result(
        "list_browser_windows",
        r#"{"windows":[{"target":"tab-1","type":"detail","title":"第7回課題","url":"index.html#surface=university-detail"}]}"#,
    );
    assert!(browser.contains("type=detail"));
    assert!(browser.contains("title=第7回課題"));
}

#[test]
fn visible_plan_steps_are_specific_without_exposing_field_values() {
    let click = ToolCall {
        name: "browser_click".into(),
        args: json!({"text":"次へ"}),
    };
    assert_eq!(plan_step_detail(&click).as_deref(), Some("次へ"));

    let file = ToolCall {
        name: "read_downloaded_file".into(),
        args: json!({"path":"/Users/haru/Downloads/lecture.pdf"}),
    };
    assert_eq!(plan_step_detail(&file).as_deref(), Some("lecture.pdf"));

    let fill = ToolCall {
        name: "browser_fill".into(),
        args: json!({"label":"パスワード","value":"secret-value"}),
    };
    assert_eq!(plan_step_detail(&fill).as_deref(), Some("パスワード"));
    assert_ne!(plan_step_detail(&fill).as_deref(), Some("secret-value"));
}

#[test]
fn browser_observation_can_infer_mouse_click_for_home() {
    let page = serde_json::json!({
        "links": [
            {
                "text": "HOME",
                "rect": { "centerX": 42, "centerY": 18 }
            }
        ],
        "interactive_elements": {
            "buttons": [],
            "inputs": []
        }
    });
    let args = infer_mouse_click_from_observation("回到首页", &page, &AgentTurnContext::default())
        .expect("mouse args");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(42));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(18));
}

#[test]
fn browser_home_request_can_fall_back_to_top_left_logo_area() {
    let page = serde_json::json!({
        "viewport": { "width": 1200, "height": 800 },
        "links": [
            {
                "text": "公益財団法人 ひょうご環境創造協会",
                "rect": { "centerX": 96, "centerY": 42 }
            },
            {
                "text": "お問い合わせ",
                "rect": { "centerX": 980, "centerY": 48 }
            }
        ],
        "interactive_elements": {
            "buttons": [],
            "inputs": []
        }
    });
    let args = infer_mouse_click_from_observation("回到主页", &page, &AgentTurnContext::default())
        .expect("mouse args");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(96));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(42));
}

#[test]
fn browser_screenshot_can_infer_top_left_home_click() {
    let screenshot = serde_json::json!({
        "coordinate_space": "screenshot",
        "screen_rect": { "x": 200, "y": 80, "width": 1200, "height": 800 },
        "image": { "mime": "image/png", "data_base64": "" }
    });
    let args =
        infer_mouse_click_from_screenshot("回到主页", &screenshot, &AgentTurnContext::default())
            .expect("mouse args");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(144));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(64));
    assert_eq!(
        args.get("coordinate_space").and_then(|v| v.as_str()),
        Some("screenshot")
    );
}

#[test]
fn short_click_confirmation_inherits_recent_home_intent() {
    let history = vec![
        crate::db::AgentMessageRow {
            id: 1,
            conv_id: "c".into(),
            role: "user".into(),
            content: "你不会点击logo回到主页吗".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 0,
        },
        crate::db::AgentMessageRow {
            id: 2,
            conv_id: "c".into(),
            role: "user".into(),
            content: "点啊".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 1,
        },
    ];
    let labels = browser_click_labels_for_turn(&history, "点啊");
    assert!(labels.iter().any(|label| label == "home"));
    assert!(labels.iter().any(|label| label == "logo"));
}

#[test]
fn short_click_confirmation_inherits_recent_suggested_tab_label() {
    let history = vec![
        crate::db::AgentMessageRow {
            id: 1,
            conv_id: "c".into(),
            role: "assistant".into(),
            content: "如果是想寻找志愿者相关的信息，我可以帮你点击上方的“ボランティア集まれ”按钮。"
                .into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 0,
        },
        crate::db::AgentMessageRow {
            id: 2,
            conv_id: "c".into(),
            role: "user".into(),
            content: "点击".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 1,
        },
    ];
    let labels = browser_click_labels_for_turn(&history, "点击");
    assert!(labels.iter().any(|label| label == "ボランティア集まれ"));
}

#[test]
fn observation_can_click_recent_suggested_tab_label() {
    let page = serde_json::json!({
        "links": [
            {
                "text": "ボランティア集まれ",
                "url": "https://jof-camp.com/new/volunteer/join-leader/",
                "rect": { "centerX": 640, "centerY": 96 }
            }
        ],
        "interactive_elements": {
            "buttons": [],
            "inputs": []
        }
    });
    let ctx = AgentTurnContext {
        browser_target: Some("ext-a-ct".into()),
        browser_click_labels: vec!["ボランティア集まれ".into()],
        ..Default::default()
    };
    let args = infer_mouse_click_from_observation("点击", &page, &ctx).expect("mouse args");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(640));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(96));
}

#[test]
fn generic_browse_tabs_request_clicks_safe_top_navigation_candidate() {
    let page = serde_json::json!({
        "url": "https://jof-camp.com/new/",
        "viewport": { "width": 1200, "height": 800 },
        "links": [
            {
                "text": "HOME",
                "url": "https://jof-camp.com/new/",
                "rect": { "centerX": 46, "centerY": 92 }
            },
            {
                "text": "募集中のキャンプ",
                "url": "https://jof-camp.com/new/camp/",
                "rect": { "centerX": 280, "centerY": 92 }
            },
            {
                "text": "お問い合わせ",
                "url": "https://jof-camp.com/new/contact/",
                "rect": { "centerX": 960, "centerY": 92 }
            }
        ],
        "interactive_elements": {
            "buttons": [],
            "inputs": []
        }
    });
    let args =
        infer_tab_browse_click_from_observation("点击标签看看全部", &page).expect("tab click");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(280));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(92));
}

#[test]
fn generic_tab_tail_is_not_treated_as_literal_click_label() {
    assert!(requested_click_labels(&normalize_planner_text("点击标签看看全部")).is_none());
}

#[test]
fn click_label_strips_generic_button_suffix() {
    let labels =
        requested_click_labels(&normalize_planner_text("点击募集中的按钮")).expect("labels");
    assert!(labels.iter().any(|label| label == "募集中"));
}

#[test]
fn numeric_selection_inherits_recent_numbered_browser_option() {
    let history = vec![
        crate::db::AgentMessageRow {
            id: 1,
            conv_id: "c".into(),
            role: "assistant".into(),
            content: "1. 页面顶部导航栏左侧的 **「募集中のキャンプ（募集中营地）」** 按钮\n2. 页面下方的 **「募集中」** 绿色图标链接".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 0,
        },
        crate::db::AgentMessageRow {
            id: 2,
            conv_id: "c".into(),
            role: "user".into(),
            content: "1".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 1,
        },
    ];
    let labels = browser_click_labels_for_turn(&history, "1");
    assert!(labels.iter().any(|label| label == "募集中のキャンプ"));
    assert!(labels.iter().all(|label| label != "募集中"));
}

#[test]
fn retry_inherits_recent_explicit_click_target() {
    let history = vec![
        crate::db::AgentMessageRow {
            id: 1,
            conv_id: "c".into(),
            role: "assistant".into(),
            content: "我这就为你点击左上角的「募集中のキャンプ（募集中营地）」按钮。".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 0,
        },
        crate::db::AgentMessageRow {
            id: 2,
            conv_id: "c".into(),
            role: "user".into(),
            content: "重试".into(),
            images_json: None,
            tool_name: None,
            tool_result_json: None,
            created_at: 1,
        },
    ];
    let labels = browser_click_labels_for_turn(&history, "重试");
    assert!(labels.iter().any(|label| label == "募集中のキャンプ"));
}

#[test]
fn ambiguous_short_boshuuchuu_click_prefers_top_navigation_over_pdf() {
    let page = serde_json::json!({
        "url": "https://jof-camp.com/new/",
        "links": [
            {
                "text": "募集中",
                "url": "https://jof-camp.com/new/files/spring.pdf",
                "rect": { "centerX": 520, "centerY": 560 }
            },
            {
                "text": "募集中のキャンプ",
                "url": "https://jof-camp.com/new/jof_camp/",
                "rect": { "centerX": 280, "centerY": 92 }
            }
        ],
        "interactive_elements": {
            "buttons": [],
            "inputs": []
        }
    });
    let args =
        infer_mouse_click_from_observation("点击募集中的按钮", &page, &AgentTurnContext::default())
            .expect("mouse args");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(280));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(92));
}

#[test]
fn observation_click_matches_cjk_equivalent_visible_label() {
    let page = serde_json::json!({
        "url": "https://kwic.kwansei.ac.jp/portal/",
        "links": [
            {
                "text": "語学資料",
                "url": "https://kwic.kwansei.ac.jp/portal/lang",
                "rect": { "centerX": 92, "centerY": 44 }
            },
            {
                "text": "履修登録",
                "url": "https://kwic.kwansei.ac.jp/portal/registration",
                "rect": { "centerX": 220, "centerY": 44 }
            }
        ],
        "interactive_elements": {
            "buttons": [],
            "inputs": []
        }
    });
    let args = infer_mouse_click_from_observation(
        "点击语学资料",
        &page,
        &AgentTurnContext {
            browser_target: Some("kwic-detail-0-ct".into()),
            browser_click_labels: Vec::new(),
            ..Default::default()
        },
    )
    .expect("mouse args");
    assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(92));
    assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(44));
}

#[test]
fn browser_click_success_can_finish_without_remote_answer() {
    let answer = local_browser_action_answer(
        "点击可见链接",
        &[(
            "computer_mouse_click".into(),
            serde_json::json!({
                "current_url": "https://example.test/next"
            }),
        )],
        &AgentTurnContext {
            browser_target: Some("kwic-detail-0-ct".into()),
            browser_click_labels: Vec::new(),
            ..Default::default()
        },
    )
    .expect("local browser action answer");
    assert!(answer.contains("已点击"));
    assert!(answer.contains("https://example.test/next"));
}
