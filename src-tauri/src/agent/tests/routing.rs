//! Heuristic routing and plan parsing tests.

use super::*;

#[test]
fn heuristic_grades() {
    let plan = heuristic_plan(&[], "成績どうだった？").expect("plan");
    assert_eq!(plan.tools[0].name, "get_grades");
}

#[test]
fn heuristic_mail() {
    let plan = heuristic_plan(&[], "メール見せて").expect("plan");
    assert_eq!(plan.tools[0].name, "list_recent_mail");
}

#[test]
fn heuristic_tasks() {
    let plan = heuristic_plan(&[], "未提出の課題ある？").expect("plan");
    assert_eq!(plan.tools[0].name, "list_luna_todos");
}

#[test]
fn general_knowledge_falls_through() {
    // "帮我查一下地政学的相关知识" should NOT match any heuristic.
    assert!(heuristic_plan(&[], "帮我查一下地政学的相关知识").is_none());
}

#[test]
fn course_name_falls_through_to_model() {
    // Course-specific queries should NOT be caught by heuristics —
    // the model needs to translate and pick the right tool.
    assert!(heuristic_plan(&[], "我下周要上国际关系历史基础").is_none());
}

#[test]
fn kgc_code_extraction() {
    assert_eq!(extract_kgc_code("AB12345 の詳細"), Some("AB12345".into()));
    assert_eq!(extract_kgc_code("hello"), None);
}

#[test]
fn parse_plan_from_json() {
    let plan = parse_plan("{\"tools\":[{\"name\":\"get_weather\",\"args\":{}}]}").unwrap();
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "get_weather");
}

#[test]
fn parse_plan_rejects_garbage_for_retry() {
    assert!(parse_plan("not json at all").is_err());
    assert!(parse_plan(r#"{"tools":[{"name":"get_weather""#).is_err());
    assert!(parse_plan(r#"{"answer":"I will check"}"#).is_err());
}

#[test]
fn trim_to_respects_limit() {
    assert_eq!(trim_to("hello", 10), "hello");
    assert_eq!(trim_to("hello world", 5), "hello…");
}

#[test]
fn heuristic_tomorrow_classes() {
    let plan = heuristic_plan(&[], "明日の授業は？").expect("plan");
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "list_week_classes");
}

#[test]
fn heuristic_tomorrow_chinese() {
    let plan = heuristic_plan(&[], "明天有什么课").expect("plan");
    assert_eq!(plan.tools[0].name, "list_week_classes");
}

#[test]
fn heuristic_notifications() {
    let plan = heuristic_plan(&[], "お知らせある？").expect("plan");
    assert_eq!(plan.tools[0].name, "list_recent_notifications");
}

#[test]
fn heuristic_registration() {
    let plan = heuristic_plan(&[], "履修科目一覧見せて").expect("plan");
    assert_eq!(plan.tools[0].name, "get_registration");
}

#[test]
fn follow_up_with_thanks_skips_tools() {
    let history = vec![tool_row("get_grades")];
    assert!(should_skip_tools(&history, "ありがとう"));
    assert!(should_skip_tools(&history, "了解"));
}

#[test]
fn multi_tool_query_falls_to_model() {
    // Queries requiring multiple tools or ambiguous intent should NOT match a single heuristic.
    assert!(heuristic_plan(&[], "来週の予定を全部まとめて教えて、準備するものも").is_none());
    assert!(heuristic_plan(&[], "看看邮件和课题").is_none());
    assert!(heuristic_plan(&[], "LunaとKWICを開いて").is_none());
}

#[test]
fn production_preplan_leaves_business_intent_to_model() {
    let ctx = AgentTurnContext::default();
    assert!(deterministic_preplan(&[], "打开Luna看看", &ctx).is_none());
    assert!(deterministic_preplan(&[], "看看邮件", &ctx).is_none());
    assert!(deterministic_preplan(&[], "打开相关Copilot页面", &ctx).is_none());
    assert!(deterministic_preplan(&[], "明天有什么课", &ctx).is_none());
}

#[test]
fn planner_failure_reads_attached_page_instead_of_doing_nothing() {
    let ctx = AgentTurnContext {
        browser_target: Some("detail-a-ct".into()),
        ..Default::default()
    };
    let plan = planner_failure_fallback(&[], "这个页面有什么", &ctx);
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "read_browser_page");
    assert_eq!(
        plan.tools[0]
            .args
            .get("target")
            .and_then(|value| value.as_str()),
        Some("detail-a-ct")
    );
}

#[test]
fn empty_plan_retries_for_data_requests_but_not_recent_summaries() {
    assert!(should_retry_empty_plan(
        &[],
        "下周的课程和课题怎么样",
        &AgentTurnContext::default()
    ));
    assert!(!should_retry_empty_plan(
        &[tool_row("list_luna_todos")],
        "总结一下",
        &AgentTurnContext::default()
    ));
    assert!(!should_retry_empty_plan(
        &[],
        "帮我解释一下这个概念",
        &AgentTurnContext::default()
    ));
}

#[test]
fn finalized_plan_allows_six_step_chain() {
    let plan = Plan {
        tools: [
            "get_weather",
            "list_recent_mail",
            "list_luna_todos",
            "list_today_classes",
            "list_recent_notifications",
            "get_grades",
            "get_registration",
        ]
        .into_iter()
        .map(|name| ToolCall {
            name: name.into(),
            args: json!({}),
        })
        .collect(),
        image_only: false,
    };
    let finalized =
        finalize_plan_with_diagnostics(plan, &[], "全部まとめて", &AgentTurnContext::default());
    assert_eq!(finalized.plan.tools.len(), 6);
}

#[test]
fn parse_plan_with_prefill() {
    // Simulates prefilled output: {"tools":[ + model continuation
    let raw = r#"{"tools":[{"name":"get_grades","args":{}}]}"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "get_grades");
}

#[test]
fn parse_plan_prefill_empty_array() {
    // Model outputs ]} after prefill {"tools":[
    let raw = r#"{"tools":[]}"#;
    let plan = parse_plan(raw).unwrap();
    assert!(plan.tools.is_empty());
}

#[test]
fn parse_plan_prefill_multi_tool() {
    let raw = r#"{"tools":[{"name":"get_grades","args":{}},{"name":"list_luna_todos","args":{}}]}"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.tools.len(), 2);
    assert_eq!(plan.tools[0].name, "get_grades");
    assert_eq!(plan.tools[1].name, "list_luna_todos");
}

#[test]
fn parse_plan_with_trailing_text() {
    // Model might output extra text after JSON
    let raw = r#"{"tools":[{"name":"get_weather","args":{}}]} I chose weather because..."#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "get_weather");
}
