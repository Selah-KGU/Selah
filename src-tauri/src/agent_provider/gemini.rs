//! Gemini non-streaming and SSE requests.

use super::requests::{Mode, SharedMessages};
use super::*;
use crate::ai::AiConfig;

pub async fn remote_gemini_non_streaming(
    config: &AiConfig,
    messages: SharedMessages,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> Result<String, String> {
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    let text = loop {
        let request = requests::gemini(
            config,
            messages.clone(),
            with_images,
            Mode::Plan {
                max_tokens,
                temperature,
                json_mode,
            },
        )
        .await?;
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
    processing::gemini(text).await
}
/// Gemini SSE streaming (`streamGenerateContent`).
pub async fn remote_gemini_stream<F>(
    config: &AiConfig,
    messages: SharedMessages,
    gen_id: &str,
    on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send + 'static,
{
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    let resp = loop {
        let resp = requests::gemini(config, messages.clone(), with_images, Mode::Stream)
            .await?
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

    receive_gemini(resp, gen_id, on_chunk).await
}

/// Consume the accepted HTTP stream; shared framing preserves fragmented UTF-8.
pub(super) async fn receive_gemini<F>(
    response: reqwest::Response,
    gen_id: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send,
{
    let mut full_text = String::new();
    let mut tool_call: Option<String> = None;
    sse::receive(response, gen_id, |data| {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
        if let Some(call) = gemini_function_call(&v) {
            log::warn!(
                "[agent answer] gemini stream returned functionCall part; forwarding pseudo call to executor: {}",
                truncate(&call, 200)
            );
            tool_call = Some(call);
            return false;
        }
        if let Some(call) = gemini_malformed_function_call(&v) {
            log::warn!(
                "[agent answer] gemini stream returned MALFORMED_FUNCTION_CALL; forwarding pseudo call to executor: {}",
                truncate(&call, 200)
            );
            tool_call = Some(call);
            return false;
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
        true
    }).await?;
    if let Some(call) = tool_call {
        return Ok(call);
    }
    Ok(full_text)
}
