use super::*;

fn old_preview(value: &Value, limit: usize) -> String {
    let json = serde_json::to_string(&sanitize_answer_tool_result(value)).unwrap();
    let mut end = limit.min(json.len());
    while end > 0 && !json.is_char_boundary(end) {
        end -= 1;
    }
    if json.len() > limit {
        format!("{}…", &json[..end])
    } else {
        json
    }
}

#[test]
fn bounded_serialization_preserves_sanitization_escaping_and_utf8_at_every_boundary() {
    let cases = [
        Value::Null,
        json!(true),
        json!(-123.45),
        json!([]),
        json!({}),
        json!(
            "あ🌕\\\"\n\t task_call: tool_call: function_call: call: ‹ › MALFORMED_FUNCTION_CALL"
        ),
        json!({"a":"日本語🌕", "b":[1,{"action":"secret", "data_base64":"A".repeat(10_000), "body":"<call:x> </call:x>"}], "download_params":{"a":"hidden"}, "object_name":"secret", "_cid":"secret", "form_params":[], "download_action":"secret"}),
    ];
    for value in cases {
        let expected = serde_json::to_string(&sanitize_answer_tool_result(&value)).unwrap();
        for limit in 0..=expected.len() + 4 {
            assert_eq!(
                preview(&value, limit),
                old_preview(&value, limit),
                "byte limit {limit}: {value}"
            );
            assert_eq!(
                json_prefix(&value, limit),
                trim_to(&expected, limit),
                "character limit {limit}: {value}"
            );
        }
    }
}

#[test]
fn json_prefix_does_not_build_the_entire_large_result() {
    let value = json!({"a":"あ🌕".repeat(100_000), "z":["large later value".repeat(100_000)]});
    let (prefix, truncated) = json_bytes(&value, 180);
    assert!(truncated);
    assert!(prefix.len() <= 180);
    assert_eq!(preview(&value, 180), old_preview(&value, 180));
    let old = serde_json::to_string(&sanitize_answer_tool_result(&value)).unwrap();
    assert_eq!(json_prefix(&value, 7000), trim_to(&old, 7000));
    // Exact equality with the limit must not add an ellipsis.
    assert_eq!(preview(&Value::Null, 4), "null");
    assert_eq!(json_prefix(&Value::Null, 4), "null");
}

#[test]
fn plain_tool_text_is_borrowed_and_marker_replacement_matches_the_original_order() {
    let text = "plain 日本語 🌕";
    assert!(matches!(
        crate::agent_text::neutralized_tool_text(text),
        Cow::Borrowed(_)
    ));
    let text = "<call:x> </call:x> task_call:x tool_call:x function_call:x call:x ‹ › MALFORMED_FUNCTION_CALL";
    let expected = text
        .replace("<call:", "<call：")
        .replace("</call:", "</call：")
        .replace("task_call:", "task_call：")
        .replace("tool_call:", "tool_call：")
        .replace("function_call:", "function_call：")
        .replace("call:", "call：")
        .replace('‹', "〈")
        .replace('›', "〉")
        .replace("MALFORMED_FUNCTION_CALL", "MALFORMED FUNCTION CALL");
    assert_eq!(crate::agent_text::neutralized_tool_text(text), expected);
}

#[test]
fn large_marked_text_matches_full_sanitization_across_prefix_boundaries() {
    for marker in [
        "<call:",
        "</call:",
        "task_call:",
        "tool_call:",
        "function_call:",
        "call:",
        "‹",
        "›",
        "MALFORMED_FUNCTION_CALL",
    ] {
        let text = format!("{}{marker}{}", "あ🌕x".repeat(8), "末尾\\\"\n".repeat(500));
        let value = Value::String(text);
        let encoded = serde_json::to_string(&sanitize_answer_tool_result(&value)).unwrap();
        for limit in 0..160 {
            let mut end = limit.min(encoded.len());
            while !encoded.is_char_boundary(end) {
                end -= 1;
            }
            let expected = if encoded.len() > limit {
                format!("{}…", &encoded[..end])
            } else {
                encoded.clone()
            };
            assert_eq!(
                preview(&value, limit),
                expected,
                "{marker}, byte limit {limit}"
            );
            assert_eq!(
                json_prefix(&value, limit),
                trim_to(&encoded, limit),
                "{marker}, character limit {limit}"
            );
        }
    }
    let huge = "plain text ".repeat(100_000);
    let prefix = crate::agent_text::neutralized_tool_prefix(&huge, 180);
    assert!(matches!(prefix, Cow::Borrowed(_)));
    assert!(prefix.len() < 220);
}

fn old_image(json: &str) -> Option<ImagePart> {
    let value: Value = serde_json::from_str(json).ok()?;
    let image = value.get("image")?;
    Some(ImagePart {
        mime: image.get("mime")?.as_str()?.to_owned(),
        data_base64: image.get("data_base64")?.as_str()?.to_owned(),
    })
}

#[test]
fn selective_screenshot_decoding_keeps_old_behavior_and_borrows_unescaped_images() {
    let cases = [
        r#"{"image":{"mime":"image/png","data_base64":"AA=="},"unneeded":{"body":"ignored"}}"#,
        r#"{"image":{"mime":"image\/png","data_base64":"A\nA\u003d"}}"#,
        r#"{"image":{"mime":3,"data_base64":"AA=="}}"#,
        r#"{"image":{"mime":"image/png"}}"#,
        r#"{"image":null}"#,
        r#"{"image":false}"#,
        r#"{"image":{"mime":"first","mime":"image/png","data_base64":"AA=="}}"#,
        r#"{"image":{"mime":"old","data_base64":"old"},"image":{"mime":"new","data_base64":"new"}}"#,
        r#"{"image":{"mime":"image/png","data_base64":"AA=="}} trailing"#,
        r#"{"\u0069mage":{"mime":"image/png","data_base64":"AA=="}}"#,
        r#"{"im\u0061ge":{"mime":"image/png","data_base64":"AA=="}}"#,
        r#"{"image":{"mime":"old","data_base64":"old"},"\u0069mage":{"mime":"new","data_base64":"new"}}"#,
        r#"{"body":"日本語の長文だけ。","nested":{"image":{"mime":"image/png","data_base64":"nested"}}}"#,
        r#"{"body":"an escaped \"image\" key in text"}"#,
        r#"{"body":"no image"}"#,
        r#"{"body":"no image"} trailing"#,
        r#"["image",{"mime":"image/png","data_base64":"AA=="}]"#,
        r#"[{"image":{"mime":"image/png","data_base64":"AA=="}}]"#,
        "[]",
        "null",
        "{",
    ];
    for json in cases {
        assert_eq!(
            has_screenshot_image(json),
            old_image(json).is_some(),
            "{json}"
        );
        assert_eq!(
            serde_json::to_value(screenshot_image(json)).unwrap(),
            serde_json::to_value(old_image(json)).unwrap(),
            "{json}"
        );
    }
    let screenshot: Screenshot<'_> = serde_json::from_str(cases[0]).unwrap();
    let image = screenshot.image.unwrap();
    assert!(matches!(image.mime, Cow::Borrowed(_)));
    assert!(matches!(image.data_base64, Cow::Borrowed(_)));
    let screenshot: Screenshot<'_> = serde_json::from_str(cases[1]).unwrap();
    assert!(matches!(
        screenshot.image.unwrap().data_base64,
        Cow::Owned(_)
    ));
}

#[test]
fn large_non_image_results_and_escaped_candidate_keys_keep_full_parser_behavior() {
    for body in ["x".repeat(200_000), "日本語\n\\escaped\"".repeat(10_000)] {
        for tail in [
            "",
            ",\"image\":null",
            ",\"image\":{\"mime\":\"image/png\",\"data_base64\":\"AA==\"}",
            r#", "\u0069mage":{"mime":"image/png","data_base64":"BB=="}"#,
        ] {
            let json = format!(
                "{{\"body\":{}{tail}}}",
                serde_json::to_string(&body).unwrap()
            );
            for input in [&json, &format!("{json} trailing")] {
                assert_eq!(has_screenshot_image(input), old_image(input).is_some());
                assert_eq!(
                    serde_json::to_value(screenshot_image(input)).unwrap(),
                    serde_json::to_value(old_image(input)).unwrap()
                );
            }
        }
    }
}

#[test]
fn screenshot_metadata_ignores_large_image_payload_and_preserves_legacy_summaries() {
    let cases = [
        json!({"target":"browser", "screen_rect":{"width":123,"height":456}, "image":{"mime":"image/png","data_base64":"A".repeat(1_000_000)}}).to_string(),
        r#"{"screen_rect":false,"target":3}"#.into(),
        r#"{"screen_rect":{"width":true,"height":"123"},"target":null}"#.into(),
        r#"{"target":"old","target":"new"}"#.into(),
        "[]".into(), "null".into(), "123".into(), "{invalid".into(),
    ];
    for json in cases {
        let old: String = match serde_json::from_str::<Value>(&json) {
            Ok(value) => format!(
                "screenshot[target={}, size={}x{}]",
                value.get("target").and_then(Value::as_str).unwrap_or(""),
                value
                    .pointer("/screen_rect/width")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                value
                    .pointer("/screen_rect/height")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            ),
            Err(_) => trim_to(&json, 260),
        };
        assert_eq!(
            summarize_plan_tool_result("computer_screenshot", &json),
            old
        );
    }
}

#[test]
fn screenshot_history_skips_invalid_and_non_tool_rows_and_respects_the_limit() {
    let mut rows = ["old", "new", "malformed", "user"]
        .into_iter()
        .enumerate()
        .map(|(id, name)| crate::db::AgentMessageRow {
            id: id as i64,
            conv_id: "c".into(),
            role: if name == "user" { "user" } else { "tool" }.into(),
            content: String::new(),
            images_json: None,
            tool_name: Some("computer_screenshot".into()),
            tool_result_json: Some(
                json!({"image":{"mime":"image/png","data_base64":name}}).to_string(),
            ),
            created_at: 0,
        })
        .collect::<Vec<_>>();
    rows[2].tool_result_json = Some("{broken".into());
    assert!(recent_screenshot_images(&rows, 0).is_empty());
    assert_eq!(
        recent_screenshot_images(&rows, 1)
            .iter()
            .map(|i| i.data_base64.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );
    assert_eq!(
        recent_screenshot_images(&rows, 8)
            .iter()
            .map(|i| i.data_base64.as_str())
            .collect::<Vec<_>>(),
        ["new", "old"]
    );
}

#[test]
fn answer_context_preserves_the_literal_fallback_for_invalid_historical_json() {
    let raw = "{broken call:example(日本語)";
    let row = crate::db::AgentMessageRow {
        id: 0,
        conv_id: "c".into(),
        role: "tool".into(),
        content: String::new(),
        images_json: None,
        tool_name: Some("read_browser_page".into()),
        tool_result_json: Some(raw.into()),
        created_at: 0,
    };
    let messages = build_answer_messages(
        None,
        &[row],
        "question",
        &[],
        &[],
        None,
        &AgentTurnContext::default(),
        false,
        false,
    );
    let old =
        serde_json::to_string(&Value::String(trim_to(raw, CFG.recent_tool_result_chars))).unwrap();
    assert!(messages[0].content.contains(&format!(
        "[read_browser_page] {}",
        trim_to(&old, CFG.recent_tool_result_chars)
    )));
}

#[test]
#[ignore = "manual comparison of JSON preparation only, not whole-app performance"]
fn benchmark_tool_result_preparation() {
    let value = json!({"body":"日本語 body ".repeat(100_000), "image":{"mime":"image/png","data_base64":"A".repeat(2_000_000)}, "target":"browser", "screen_rect":{"width":1920,"height":1080}});
    let json = value.to_string();
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut stages = [
        [Vec::new(), Vec::new(), Vec::new()],
        [Vec::new(), Vec::new(), Vec::new()],
    ];
    for round in 0..8 {
        for optimized in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = std::time::Instant::now();
            let preview = if optimized {
                preview(&value, 180)
            } else {
                old_preview(&value, 180)
            };
            let preview_time = started.elapsed();
            let stage_start = std::time::Instant::now();
            let metadata = if optimized {
                screenshot_metadata(&json).unwrap()
            } else {
                serde_json::from_str::<Value>(&json).unwrap()
            };
            let metadata_time = stage_start.elapsed();
            let stage_start = std::time::Instant::now();
            let image = if optimized {
                screenshot_image(&json).unwrap()
            } else {
                old_image(&json).unwrap()
            };
            let image_time = stage_start.elapsed();
            std::hint::black_box((&preview, &metadata, &image));
            if round > 0 {
                stages[optimized as usize][0].push(preview_time);
                stages[optimized as usize][1].push(metadata_time);
                stages[optimized as usize][2].push(image_time);
                if optimized {
                    after.push(started.elapsed());
                } else {
                    before.push(started.elapsed());
                }
            }
        }
    }
    before.sort();
    after.sort();
    println!("Tool JSON prep, debug build, 7 alternating samples, {} text bytes + {} image bytes: old median {:?}, borrowed/bounded median {:?}; JSON preview capped at 180 bytes", value["body"].as_str().unwrap().len(), value["image"]["data_base64"].as_str().unwrap().len(), before[3], after[3]);
    for (index, name) in ["preview", "screenshot metadata", "screenshot image"]
        .iter()
        .enumerate()
    {
        stages[0][index].sort();
        stages[1][index].sort();
        println!(
            "{name}: full median {:?}, bounded/borrowed median {:?}",
            stages[0][index][3], stages[1][index][3]
        );
    }
}
