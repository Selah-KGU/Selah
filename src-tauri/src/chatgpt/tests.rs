use super::inference::{request_body, Output};
use crate::ai::{ChatMessage, ImagePart};

#[test]
fn plan_requests_keep_history_images_and_omit_unsupported_api_parameters() {
    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: "Return JSON".into(),
            images: vec![],
        },
        ChatMessage {
            role: "user".into(),
            content: "Read this".into(),
            images: vec![ImagePart {
                mime: "image/png".into(),
                data_base64: "test".into(),
            }],
        },
        ChatMessage {
            role: "assistant".into(),
            content: "Earlier answer".into(),
            images: vec![],
        },
    ];
    let body = request_body("account-selected-model", &messages, true);
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["input"][0]["role"], "developer");
    assert_eq!(
        body["input"][1]["content"][1]["image_url"],
        "data:image/png;base64,test"
    );
    assert_eq!(body["input"][2]["content"], "Earlier answer");
    assert_eq!(body["text"]["format"]["type"], "json_object");
    for unsupported in [
        "temperature",
        "max_tokens",
        "max_output_tokens",
        "previous_response_id",
        "messages",
        "tools",
        "background",
        "conversation",
    ] {
        assert!(body.get(unsupported).is_none(), "unexpected {unsupported}");
    }
}

#[test]
fn partial_text_is_not_success_until_completed_and_usage_errors_are_preserved() {
    let delta = r#"{"type":"response.output_text.delta","delta":"こんにちは"}"#;
    let mut chunks = String::new();
    let mut output = Output::default();
    assert!(output.event(delta, &mut |s, _| chunks.push_str(s)));
    assert_eq!(chunks, "こんにちは");
    assert!(output.finish().is_err());
    for terminal in [
        r#"{"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}"#,
        r#"{"type":"response.incomplete","response":{"status":"incomplete"}}"#,
        r#"{"type":"error","error":{"code":"subscription_sharing_usage_unavailable"}}"#,
    ] {
        let mut output = Output::default();
        output.event(delta, &mut |_, _| {});
        assert!(!output.event(terminal, &mut |_, _| {}));
        assert!(output.finish().is_err());
    }
    let mut output = Output::default();
    output.event(delta, &mut |_, _| {});
    assert!(!output.event(
        r#"{"type":"response.completed","response":{"status":"completed"}}"#,
        &mut |_, _| {}
    ));
    assert_eq!(output.finish().unwrap(), "こんにちは");
}

#[test]
fn top_level_stream_error_keeps_usage_limit_guidance() {
    let mut output = Output::default();
    output.event(r#"{"type":"error","code":"subscription_sharing_usage_limit_exceeded","message":"Limit reached","param":null}"#, &mut |_, _| {});
    assert!(output.finish().unwrap_err().contains("利用上限"));
}
