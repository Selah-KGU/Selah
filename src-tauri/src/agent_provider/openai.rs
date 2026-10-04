//! OpenAI-compatible non-streaming and SSE requests.

use super::*;
use crate::ai::{AiConfig, ChatMessage};

pub async fn remote_openai_non_streaming(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> Result<String, String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    let mut with_json_mode = json_mode;
    let text = loop {
        let mut body = serde_json::json!({
            "model": config.model,
            "messages": openai_messages_json(&messages, with_images),
            "max_tokens": if max_tokens == 0 { 8192 } else { max_tokens },
            "temperature": temperature,
        });
        if with_json_mode {
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
        let (status, text) = send_non_streaming_request(request, "openai").await?;
        if !status.is_success() {
            // Endpoint refused the image part → retry once text-only.
            if with_images && is_image_unsupported_error(&text) {
                log::warn!("plan: model rejected image input, retrying text-only");
                with_images = false;
                continue;
            }
            if with_json_mode && response_format_unsupported(&text) {
                log::warn!("plan: model rejected JSON response format, retrying without it");
                with_json_mode = false;
                continue;
            }
            return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
        }
        break text;
    };
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
/// OpenAI-compatible SSE streaming (`stream: true`).
pub async fn remote_openai_stream<F>(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    gen_id: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send + 'static,
{
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    // Retry once text-only if the endpoint refuses images. on_chunk is untouched
    // until streaming begins, so re-POSTing here is safe.
    let resp = loop {
        let body = serde_json::json!({
            "model": config.model,
            "messages": openai_messages_json(&messages, with_images),
            "max_tokens": if config.max_tokens == 0 { 32768u32 } else { config.max_tokens },
            "temperature": config.temperature,
            "stream": true,
        });

        let resp = http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if with_images && is_image_unsupported_error(&text) {
                log::warn!("answer: model rejected image input, retrying text-only");
                with_images = false;
                continue;
            }
            return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
        }
        break resp;
    };

    // Read SSE byte stream.
    let mut full_text = String::new();
    let mut buffer = String::new();
    let mut byte_stream = resp.bytes_stream();
    use futures_util::StreamExt;

    while let Some(chunk_result) = byte_stream.next().await {
        if is_remote_cancelled(gen_id) {
            break;
        }
        let bytes = chunk_result.map_err(|e| format!("ストリーム読み取り失敗: {}", e))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        // Process complete SSE lines.
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line == "data: [DONE]" {
                break;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(delta) = v
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        full_text.push_str(delta);
                        // Remote models don't use <think> blocks typically,
                        // but we pass is_think=false to keep the interface consistent.
                        on_chunk(delta, false);
                    }
                    // Some providers return reasoning_content for think tokens.
                    if let Some(think) = v
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("reasoning_content"))
                        .and_then(|c| c.as_str())
                    {
                        on_chunk(think, true);
                    }
                }
            }
        }
    }

    Ok(full_text)
}
