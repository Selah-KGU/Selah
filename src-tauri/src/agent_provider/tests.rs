use super::*;
use crate::ai::{ChatMessage, ImagePart};

fn msg_with_image() -> ChatMessage {
    ChatMessage {
        role: "user".into(),
        content: "what is on screen?".into(),
        images: vec![ImagePart {
            mime: "image/png".into(),
            data_base64: "QUJD".into(),
        }],
    }
}

#[test]
fn openai_message_json_is_multimodal_with_image() {
    let v = openai_message_json(&msg_with_image());
    let content = v.get("content").and_then(|c| c.as_array()).unwrap();
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
}

#[test]
fn openai_message_json_plain_string_without_image() {
    let m = ChatMessage {
        role: "user".into(),
        content: "hi".into(),
        images: vec![],
    };
    assert_eq!(openai_message_json(&m)["content"], "hi");
}

#[test]
fn image_unsupported_error_detected_for_deepseek_style_400() {
    assert!(is_image_unsupported_error(
        "Failed to deserialize the JSON body: messages[6]: unknown variant `image_url`, expected `text`"
    ));
    assert!(!is_image_unsupported_error("rate limit exceeded"));
}

#[test]
fn gemini_parts_json_appends_inline_image() {
    let parts = gemini_parts_json(&msg_with_image());
    assert!(parts[0].get("text").is_some());
    assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
    assert_eq!(parts[1]["inlineData"]["data"], "QUJD");
}

#[test]
fn extracts_gemini_malformed_call_finish_message() {
    let payload = serde_json::json!({
        "candidates": [{
            "content": {},
            "finishReason": "MALFORMED_FUNCTION_CALL",
            "finishMessage": "Malformed function call: call:read_downloaded_file {\"path\":\"/tmp/a.md\"}"
        }]
    });

    assert_eq!(
        gemini_malformed_function_call(&payload).as_deref(),
        Some("call:read_downloaded_file {\"path\":\"/tmp/a.md\"}")
    );
}

#[test]
fn ignores_non_malformed_gemini_finish_message() {
    let payload = serde_json::json!({
        "candidates": [{
            "finishReason": "STOP",
            "finishMessage": "call:read_downloaded_file {\"path\":\"/tmp/a.md\"}"
        }]
    });

    assert!(gemini_malformed_function_call(&payload).is_none());
}

#[test]
fn scans_all_gemini_candidates_for_malformed_call() {
    let payload = serde_json::json!({
        "candidates": [
            {
                "finishReason": "STOP",
                "finishMessage": "not a call"
            },
            {
                "finishReason": "MALFORMED_FUNCTION_CALL",
                "finishMessage": "Malformed function call: task_call:list_downloaded_files(limit=5)"
            }
        ]
    });

    assert_eq!(
        gemini_malformed_function_call(&payload).as_deref(),
        Some("task_call:list_downloaded_files(limit=5)")
    );
}

#[test]
fn extracts_call_space_from_malformed_function_call() {
    let payload = serde_json::json!({
        "candidates": [{
            "content": {},
            "finishReason": "MALFORMED_FUNCTION_CALL",
            "finishMessage": "Malformed function call: call get_course_context {luna_id: \"2026341390020201\"}"
        }]
    });

    assert_eq!(
        gemini_malformed_function_call(&payload).as_deref(),
        Some("call get_course_context {luna_id: \"2026341390020201\"}")
    );
}

#[test]
fn extracts_gemini_function_call_part() {
    let payload = serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "read_browser_page",
                        "args": {}
                    },
                    "thoughtSignature": "ignored"
                }]
            },
            "finishReason": "STOP"
        }]
    });

    assert_eq!(
        gemini_function_call(&payload).as_deref(),
        Some("call:read_browser_page{}")
    );
}

#[test]
fn extracts_gemini_function_call_part_with_args() {
    let payload = serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "browser_click",
                        "args": {
                            "target": "最新のお知らせ"
                        }
                    }
                }]
            }
        }]
    });

    assert_eq!(
        gemini_function_call(&payload).as_deref(),
        Some("call:browser_click{\"target\":\"最新のお知らせ\"}")
    );
}

#[test]
fn extracts_fullwidth_colon_from_malformed_function_call() {
    let payload = serde_json::json!({
        "candidates": [{
            "content": {},
            "finishReason": "MALFORMED_FUNCTION_CALL",
            "finishMessage": "Malformed function call: call：read_file〔path: \"/tmp/a.pdf\"〕"
        }]
    });

    assert_eq!(
        gemini_malformed_function_call(&payload).as_deref(),
        Some("call：read_file〔path: \"/tmp/a.pdf\"〕")
    );
}

#[test]
fn formats_standard_remote_api_error_without_provider_specific_match() {
    let raw = r#"API error (403 Forbidden): {
  "error": {
"code": 403,
"message": "provider supplied permission message",
"status": "PERMISSION_DENIED"
  }
}"#;
    let formatted = format_remote_api_error(raw).expect("formatted error");
    assert!(formatted.contains("403 Forbidden"));
    assert!(formatted.contains("code=403"));
    assert!(formatted.contains("status=PERMISSION_DENIED"));
    assert!(formatted.contains("provider supplied permission message"));
    assert!(!formatted.contains("Lightning"));
}

#[test]
fn think_filter_routes_thought_tags_to_thinking_stream() {
    let chunks = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, bool)>::new()));
    let captured = chunks.clone();
    let (mut feed, mut flush) = ThinkFilter::wrap_with_flush(move |chunk, is_think| {
        captured.lock().unwrap().push((chunk.to_string(), is_think));
    });

    feed("visible <thought>hidden", false);
    feed("</thought> done", false);
    flush();

    let actual = chunks.lock().unwrap().clone();
    assert_eq!(
        actual,
        vec![
            ("visible ".to_string(), false),
            ("hidden".to_string(), true),
            (" done".to_string(), false),
        ]
    );
}

#[test]
fn think_filter_routes_malformed_thought_prefix_to_thinking_stream() {
    let chunks = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, bool)>::new()));
    let captured = chunks.clone();
    let (mut feed, mut flush) = ThinkFilter::wrap_with_flush(move |chunk, is_think| {
        captured.lock().unwrap().push((chunk.to_string(), is_think));
    });

    feed("<thoughtThe user wants tools", false);
    flush();

    let actual = chunks.lock().unwrap().clone();
    assert!(actual.iter().all(|(_, is_think)| *is_think));
    assert_eq!(
        actual
            .iter()
            .map(|(chunk, _)| chunk.as_str())
            .collect::<String>(),
        "The user wants tools"
    );
}

#[test]
fn turn_cancel_survives_until_explicitly_cleared() {
    let gen_id = "test-turn-cancel-survives-until-cleared";
    AgentProvider::clear_cancel(gen_id);
    assert!(!AgentProvider::is_cancelled(gen_id));
    AgentProvider::cancel(gen_id);
    assert!(AgentProvider::is_cancelled(gen_id));
    AgentProvider::clear_cancel(gen_id);
    assert!(!AgentProvider::is_cancelled(gen_id));
}

#[test]
fn collects_openai_text_from_string_and_part_arrays() {
    assert_eq!(
        collect_openai_message_text(Some(&serde_json::json!("plain"))),
        "plain"
    );
    assert_eq!(
        collect_openai_message_text(Some(&serde_json::json!([
            {"type": "text", "text": "{\"summary\":"},
            {"type": "text", "text": "\"ok\"}"}
        ]))),
        "{\"summary\":\"ok\"}"
    );
}

#[test]
fn detects_response_format_rejection() {
    assert!(response_format_unsupported(
        r#"{"error":{"message":"response_format json_object is unsupported"}}"#
    ));
    assert!(!response_format_unsupported(
        r#"{"error":{"message":"model not found"}}"#
    ));
}
