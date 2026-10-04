//! Visible tool-call parsing and argument sanitizing tests.

use super::*;

#[test]
fn sanitize_tool_results_neutralizes_pseudo_calls() {
    let value = serde_json::json!({
        "body": "call:kg_canvas:download_luna_file({}) <call:tool /> MALFORMED_FUNCTION_CALL",
        "download_action": "/hidden",
    });
    let sanitized = sanitize_answer_tool_result(&value);
    let body = sanitized.get("body").and_then(|v| v.as_str()).unwrap();
    assert!(body.contains("call：kg_canvas"));
    assert!(!body.contains("call:kg_canvas"));
    assert!(sanitized.get("download_action").is_none());
}

#[test]
fn canonical_tool_aliases_are_accepted() {
    assert_eq!(
        agent_tools::canonical_tool_name("browser_reload"),
        Some("browser_reload_page")
    );
    assert_eq!(
        agent_tools::canonical_tool_name("read_file"),
        Some("read_downloaded_file")
    );
    assert_eq!(
        agent_tools::canonical_tool_name("view_file"),
        Some("read_downloaded_file")
    );
    assert_eq!(
        agent_tools::canonical_tool_name("kg_canvas:download_luna_file"),
        Some("download_course_material")
    );
    assert_eq!(
        agent_tools::canonical_tool_name("download_luna_file"),
        Some("download_course_material")
    );
    assert_eq!(
        agent_tools::canonical_tool_name("download_material_file"),
        Some("download_course_material")
    );
    assert_eq!(
        agent_tools::canonical_tool_name("fetch_lms_course_resources"),
        Some("list_luna_announcements")
    );
    assert_eq!(
        agent_tools::exact_tool_name("open_browser_url"),
        Some("open_browser_url")
    );
    assert!(agent_tools::exact_tool_name("launch_browser").is_none());
    assert!(agent_tools::canonical_tool_name("launch_browser").is_none());
    assert!(agent_tools::is_known_tool("read_file"));
}

#[test]
fn lms_resource_alias_preserves_course_keyword() {
    let args = serde_json::json!({ "course_name": "政治学基礎 ２" });
    let sanitized =
        agent_tools::sanitize_tool_args("fetch_lms_course_resources", &args).expect("sanitized");
    assert_eq!(
        sanitized.get("keyword").and_then(|v| v.as_str()),
        Some("政治学基礎 ２")
    );
}

#[test]
fn parses_visible_task_call_for_real_execution() {
    let answer = "‹task_call:download_course_material(luna_id=\"2026341390020201\",filename=\"2026年度春中間試験の実施要項.pdf\")›";
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "download_course_material");
    assert_eq!(
        call.args.get("luna_id").and_then(|v| v.as_str()),
        Some("2026341390020201")
    );
    assert_eq!(
        call.args.get("filename").and_then(|v| v.as_str()),
        Some("2026年度春中間試験の実施要項.pdf")
    );
}

#[test]
fn parses_visible_json_style_tool_call() {
    let answer = r#"task_call:download_course_material{"luna_id":"2026341390020201","filename":"midterm.pdf"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "download_course_material");
    assert_eq!(
        call.args.get("filename").and_then(|v| v.as_str()),
        Some("midterm.pdf")
    );
}

#[test]
fn parses_visible_download_luna_file_alias_for_real_execution() {
    let answer =
        r#"call:download_luna_file{"course_name":"政治学基礎 ２","filename":"midterm.pdf"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "download_course_material");
    assert_eq!(
        call.args.get("filename").and_then(|v| v.as_str()),
        Some("midterm.pdf")
    );
}

#[test]
fn parses_visible_download_luna_file_js_style_args() {
    let answer = r#"call:download_luna_file {luna_id: "2026341390020201", file_name: "2026年度春中間試験の実施要項.pdf"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "download_course_material");
    assert_eq!(
        call.args.get("luna_id").and_then(|v| v.as_str()),
        Some("2026341390020201")
    );
    assert_eq!(
        call.args.get("filename").and_then(|v| v.as_str()),
        Some("2026年度春中間試験の実施要項.pdf")
    );
}

#[test]
fn parses_glued_download_material_file_call() {
    let answer = "call:download_material_fileluna_id=2026341390020201file_name=2026年度春中間試験の実施要項.pdf";
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "download_course_material");
    assert_eq!(
        call.args.get("luna_id").and_then(|v| v.as_str()),
        Some("2026341390020201")
    );
    assert_eq!(
        call.args.get("filename").and_then(|v| v.as_str()),
        Some("2026年度春中間試験の実施要項.pdf")
    );
}

#[test]
fn parses_gemini_finish_message_call_for_real_execution() {
    let answer = r#"call:read_downloaded_file {"path":"/Users/haru/Documents/Selah/政治学基礎 ２/20260525_政治学基礎　２_live.md"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "read_downloaded_file");
    assert_eq!(
        call.args.get("path").and_then(|v| v.as_str()),
        Some("/Users/haru/Documents/Selah/政治学基礎 ２/20260525_政治学基礎　２_live.md")
    );
}

#[test]
fn parses_view_file_alias_for_real_execution() {
    let answer = r#"call:view_file {"path":"/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "read_downloaded_file");
    assert_eq!(
        call.args.get("path").and_then(|v| v.as_str()),
        Some("/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md")
    );
}

#[test]
fn detects_unknown_leading_pseudo_call_without_leaking_it() {
    let answer = r#"call:imaginary_file_tool {"path":"/tmp/a.md"}"#;
    assert!(parse_visible_tool_call(answer).is_none());
    assert!(has_any_pseudo_tool_call(answer));
    let raw = parse_any_raw_tool_call(answer).expect("raw pseudo call");
    assert_eq!(raw.name, "imaginary_file_tool");
    assert_eq!(
        raw.args.get("path").and_then(|v| v.as_str()),
        Some("/tmp/a.md")
    );
}

#[test]
fn parses_neutralized_fullwidth_call_for_real_execution() {
    let answer = r#"call：read_file〔path: "/Users/haru/Documents/Selah/政治学基礎 ２/2026年度春中間試験の実施要項.pdf"〕"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "read_downloaded_file");
    assert_eq!(
        call.args.get("path").and_then(|v| v.as_str()),
        Some("/Users/haru/Documents/Selah/政治学基礎 ２/2026年度春中間試験の実施要項.pdf")
    );
}

#[test]
fn parses_fullwidth_arg_delimiter_in_neutralized_call() {
    let answer = r#"call：read_file〔path： "/tmp/midterm.pdf"〕"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "read_downloaded_file");
    assert_eq!(
        call.args.get("path").and_then(|v| v.as_str()),
        Some("/tmp/midterm.pdf")
    );
}

#[test]
fn parses_read_downloaded_file_filename_only_call() {
    let answer = r#"call:read_downloaded_file {"filename":"20260525_政治学基礎　２_live.md","course_name":"政治学基礎 ２"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "read_downloaded_file");
    assert_eq!(
        call.args.get("filename").and_then(|v| v.as_str()),
        Some("20260525_政治学基礎　２_live.md")
    );
    assert_eq!(
        call.args.get("course_name").and_then(|v| v.as_str()),
        Some("政治学基礎 ２")
    );
}

#[test]
fn parses_fullwidth_bracket_course_context_call() {
    let answer = r#"call:get_course_context〔kgc_code: "34139002"〕"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "get_course_context");
    assert_eq!(
        call.args.get("query").and_then(|v| v.as_str()),
        Some("34139002")
    );
}

#[test]
fn parses_call_space_course_context_call() {
    let answer = r#"call get_course_context {luna_id: "2026341390020201"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "get_course_context");
    assert_eq!(
        call.args.get("query").and_then(|v| v.as_str()),
        Some("2026341390020201")
    );
}

#[test]
fn parses_activity_title_alias_for_luna_detail_call() {
    let answer = r#"call:get_luna_activity_detail{activity_title:"第7回復習課題（5/29 23:59締め切り）",luna_id:"2026341390020201"}"#;
    let call = parse_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "get_luna_activity_detail");
    assert_eq!(
        call.args.get("title").and_then(|v| v.as_str()),
        Some("第7回復習課題（5/29 23:59締め切り）")
    );
    assert_eq!(
        call.args.get("luna_id").and_then(|v| v.as_str()),
        Some("2026341390020201")
    );
}

#[test]
fn sanitizer_accepts_common_tool_arg_aliases() {
    let course_args = serde_json::json!({ "course_code": "34139002" });
    let course = agent_tools::sanitize_tool_args("get_course_context", &course_args)
        .expect("course context args");
    assert_eq!(
        course.get("query").and_then(|v| v.as_str()),
        Some("34139002")
    );

    let detail_args = serde_json::json!({ "activityTitle": "中間試験", "type": "material" });
    let detail = agent_tools::sanitize_tool_args("get_luna_activity_detail", &detail_args)
        .expect("luna detail args");
    assert_eq!(
        detail.get("title").and_then(|v| v.as_str()),
        Some("中間試験")
    );
    assert_eq!(
        detail.get("activity_type").and_then(|v| v.as_str()),
        Some("material")
    );

    let material_args =
        serde_json::json!({ "attachment_name": "2026年度春中間試験の実施要項.pdf" });
    let material = agent_tools::sanitize_tool_args("download_course_material", &material_args)
        .expect("download material args");
    assert_eq!(
        material.get("filename").and_then(|v| v.as_str()),
        Some("2026年度春中間試験の実施要項.pdf")
    );
}

#[test]
fn parse_plan_recovers_visible_tool_call() {
    let plan = parse_plan(r#"call:read_downloaded_file {"path":"/tmp/a.md"}"#).expect("plan");
    assert!(!plan.image_only);
    assert_eq!(plan.tools.len(), 1);
    assert_eq!(plan.tools[0].name, "read_downloaded_file");
    assert_eq!(
        plan.tools[0].args.get("path").and_then(|v| v.as_str()),
        Some("/tmp/a.md")
    );
}

#[test]
fn visible_tool_call_parser_requires_leading_call() {
    let answer = "これは説明です。task_call:download_course_material(filename=\"midterm.pdf\")";
    assert!(parse_visible_tool_call(answer).is_none());
}

#[test]
fn any_visible_tool_call_parser_handles_nonleading_call() {
    let answer = r#"確認します。 call:view_file {"path":"/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md"}"#;
    assert!(parse_visible_tool_call(answer).is_none());
    let call = parse_any_visible_tool_call(answer).expect("tool call");
    assert_eq!(call.name, "read_downloaded_file");
    assert_eq!(
        call.args.get("path").and_then(|v| v.as_str()),
        Some("/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md")
    );
    assert!(has_any_pseudo_tool_call(answer));
}

#[test]
fn pseudo_call_scan_ignores_normal_words() {
    assert!(find_pseudo_tool_call_start("callback: done").is_none());
    assert!(find_pseudo_tool_call_start("recall the file later").is_none());
    assert!(
        find_pseudo_tool_call_start("確認: call:get_course_context〔kgc_code: \"34139002\"〕")
            .is_some()
    );
}

#[test]
fn safe_visible_emit_len_holds_tail_for_split_detection() {
    assert_eq!(safe_visible_emit_len("短い call", 16), 0);
    let long = "これは普通の説明です。あとで call";
    let emit_len = safe_visible_emit_len(long, 8);
    assert!(emit_len > 0);
    assert!(long[emit_len..].contains("call"));
}

#[test]
fn visible_stream_start_detects_split_pseudo_call() {
    assert!(matches!(
        classify_visible_stream_start("‹task_"),
        VisibleStart::MaybePseudoCall
    ));
    assert!(matches!(
        classify_visible_stream_start("‹task_call:download_course_material("),
        VisibleStart::PseudoCall
    ));
    assert!(matches!(
        classify_visible_stream_start("call "),
        VisibleStart::PseudoCall
    ));
    assert!(matches!(
        classify_visible_stream_start("call：read_file〔"),
        VisibleStart::PseudoCall
    ));
    assert!(matches!(
        classify_visible_stream_start("call get_course_context {"),
        VisibleStart::PseudoCall
    ));
    assert!(matches!(
        classify_visible_stream_start("我看了一下資料"),
        VisibleStart::Normal
    ));
}

#[test]
fn estimate_tokens_sanity() {
    // Short ASCII text
    assert!(estimate_tokens("hello") > 0);
    // CJK text (3 bytes per char)
    let cjk = "こんにちは"; // 15 bytes
    assert!(estimate_tokens(cjk) >= 3);
    // Empty
    assert_eq!(estimate_tokens(""), 1);
}

#[test]
fn heuristic_student_profile() {
    let plan = heuristic_plan(&[], "学籍番号教えて").expect("plan");
    assert_eq!(plan.tools[0].name, "get_student_profile");
}

#[test]
fn heuristic_today_brief() {
    let plan = heuristic_plan(&[], "今天有什么安排").expect("plan");
    assert_eq!(plan.tools[0].name, "get_today_brief");
}

#[test]
fn kgc_code_whitelist_rejects_random_token() {
    // PDF12345 fits the structural pattern but isn't a real KGC prefix.
    assert_eq!(extract_kgc_code("PDF12345 syllabus"), None);
    // AB12345 should still be picked up.
    assert_eq!(extract_kgc_code("AB12345 syllabus"), Some("AB12345".into()));
}

#[test]
fn opinion_short_skips_smalltalk_but_long_does_not() {
    assert!(should_skip_tools(&[], "どう思う？"));
    assert!(!should_skip_tools(
        &[],
        "経済学が好きだから経済学の授業教えて"
    ));
}
