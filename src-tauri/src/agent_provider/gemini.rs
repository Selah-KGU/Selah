//! Gemini non-streaming and SSE requests.

use super::*;
use crate::agent_text;
use crate::ai::{AiConfig, ChatMessage};

pub async fn remote_gemini_non_streaming(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> Result<String, String> {
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
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    let text = loop {
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
        let (status, text) = send_non_streaming_request(request, "gemini").await?;
        if !status.is_success() {
            if with_images && is_image_unsupported_error(&text) {
                log::warn!("plan(gemini): model rejected image input, retrying text-only");
                with_images = false;
                continue;
            }
            return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
        }
        break text;
    };
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
/// Gemini SSE streaming (`streamGenerateContent`).
pub async fn remote_gemini_stream<F>(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    gen_id: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send + 'static,
{
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
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    let resp = loop {
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

        let resp = http_client()
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if with_images && is_image_unsupported_error(&text) {
                log::warn!("answer(gemini): model rejected image input, retrying text-only");
                with_images = false;
                continue;
            }
            return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
        }
        break resp;
    };

    let mut full_text = String::new();
    let mut buffer = String::new();
    let mut tool_call: Option<String> = None;
    let mut byte_stream = resp.bytes_stream();
    use futures_util::StreamExt;

    while let Some(chunk_result) = byte_stream.next().await {
        if is_remote_cancelled(gen_id) {
            break;
        }
        let bytes = chunk_result.map_err(|e| format!("ストリーム読み取り失敗: {}", e))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer = buffer[line_end + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(call) = gemini_function_call(&v) {
                        log::warn!(
                            "[agent answer] gemini stream returned functionCall part; forwarding pseudo call to executor: {}",
                            truncate(&call, 200)
                        );
                        tool_call = Some(call);
                        buffer.clear();
                        break;
                    }
                    if let Some(call) = gemini_malformed_function_call(&v) {
                        log::warn!(
                            "[agent answer] gemini stream returned MALFORMED_FUNCTION_CALL; forwarding pseudo call to executor: {}",
                            truncate(&call, 200)
                        );
                        tool_call = Some(call);
                        buffer.clear();
                        break;
                    }
                    let text = collect_gemini_text_parts(
                        v.get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts")),
                    );
                    if !text.is_empty() {
                        full_text.push_str(&text);
                        on_chunk(&text, false);
                    }
                }
            }
        }
        if tool_call.is_some() {
            break;
        }
    }

    if let Some(call) = tool_call {
        return Ok(call);
    }
    Ok(full_text)
}
