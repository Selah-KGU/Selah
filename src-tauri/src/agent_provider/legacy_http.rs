//! Frozen pre-worker HTTP assembly and parsing for compatibility checks.
#![allow(unused_variables, clippy::needless_borrow)]
use super::*;
use crate::agent_text;
use crate::ai::{AiConfig, ChatMessage};
pub(super) fn parse_openai(text: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let message = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"));
    let mut content = collect_openai_message_text(message.and_then(|m| m.get("content")));
    // Reasoning models behind OpenRouter (e.g. minimax with json_object) return a
    // null `content` and put the whole JSON answer in `reasoning`, with HTTP 200 and
    // finish_reason=stop — so the error-text retry above never fires. Recover it.
    if content.trim().is_empty() {
        if let Some(reasoning) = message
            .and_then(|m| m.get("reasoning"))
            .and_then(|r| r.as_str())
        {
            content = reasoning.to_string();
        }
    }
    if content.trim().is_empty() {
        let finish_reason = v
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(|reason| reason.as_str())
            .unwrap_or("unknown");
        Err(format!(
            "AIからの応答がありません (finish_reason: {})",
            finish_reason
        ))
    } else {
        Ok(content)
    }
}
pub(super) fn openai_plan(
    config: &AiConfig,
    messages: &[ChatMessage],
    with_images: bool,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": openai_messages_json(&messages, with_images),
        "max_tokens": if max_tokens == 0 { 8192 } else { max_tokens },
        "temperature": temperature,
    });
    if json_mode {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    let request = http_client()
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        // Large compressed JSON responses occasionally arrive truncated
        // through compatible gateways, which reqwest reports as a body
        // decoding failure. Prefer a plain response for plan calls.
        .header("Accept-Encoding", "identity")
        .json(&body);
    request
}
pub(super) fn openai_stream(
    config: &AiConfig,
    messages: &[ChatMessage],
    with_images: bool,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": openai_messages_json(&messages, with_images),
        "max_tokens": if config.max_tokens == 0 { 32768u32 } else { config.max_tokens },
        "temperature": config.temperature,
        "stream": true,
    });

    http_client()
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
}
pub(super) fn parse_gemini(text: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let content = collect_gemini_text_parts(
        v.get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts")),
    );
    if content.is_empty() {
        if let Some(call) = gemini_function_call(&v) {
            log::warn!(
                "[agent answer] gemini returned functionCall part; forwarding pseudo call to executor: {}",
                truncate(&call, 200)
            );
            return Ok(call);
        }
        if let Some(call) = gemini_malformed_function_call(&v) {
            log::warn!(
                "[agent answer] gemini returned MALFORMED_FUNCTION_CALL; forwarding pseudo call to executor: {}",
                truncate(&call, 200)
            );
            return Ok(call);
        }
        return Err(format!(
            "AIからの応答がありません: {}",
            truncate(&text, 300)
        ));
    }
    Ok(content)
}
pub(super) fn gemini_plan(
    config: &AiConfig,
    messages: &[ChatMessage],
    with_images: bool,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> reqwest::RequestBuilder {
    let model = urlencoding::encode(&config.model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        model
    );
    let system_text: Vec<String> = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| agent_text::neutralize_pseudo_tool_calls(&m.content))
        .collect();
    let system_instruction = if system_text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "role": "user",
            "parts": [{ "text": system_text.join("\n") }]
        })
    };
    let contents: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            serde_json::json!({
                "role": if m.role == "assistant" { "model" } else { "user" },
                "parts": gemini_message_parts(m, with_images)
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": if max_tokens == 0 { 8192 } else { max_tokens },
            "temperature": temperature,
        },
    });
    if json_mode {
        body["generationConfig"]["responseMimeType"] =
            serde_json::Value::String("application/json".into());
    }
    if !system_instruction.is_null() {
        body["systemInstruction"] = system_instruction.clone();
    }
    let request = http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &config.api_key)
        .header("Accept-Encoding", "identity")
        .json(&body);
    request
}
pub(super) fn gemini_stream(
    config: &AiConfig,
    messages: &[ChatMessage],
    with_images: bool,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> reqwest::RequestBuilder {
    let model = urlencoding::encode(&config.model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
        model
    );
    let system_text: Vec<String> = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| agent_text::neutralize_pseudo_tool_calls(&m.content))
        .collect();
    let system_instruction = if system_text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "role": "user",
            "parts": [{ "text": system_text.join("\n") }]
        })
    };
    let contents: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            serde_json::json!({
                "role": if m.role == "assistant" { "model" } else { "user" },
                "parts": gemini_message_parts(m, with_images)
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": if config.max_tokens == 0 { 32768u32 } else { config.max_tokens },
            "temperature": config.temperature,
        },
    });
    if !system_instruction.is_null() {
        body["systemInstruction"] = system_instruction.clone();
    }

    http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &config.api_key)
        .json(&body)
}
