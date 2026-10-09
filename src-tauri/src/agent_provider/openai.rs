//! OpenAI-compatible non-streaming and SSE requests.

use super::requests::{Mode, SharedMessages};
use super::*;
use crate::ai::AiConfig;

pub async fn remote_openai_non_streaming(
    config: &AiConfig,
    messages: SharedMessages,
    max_tokens: u32,
    temperature: f32,
    json_mode: bool,
) -> Result<String, String> {
    let has_images = messages.iter().any(|m| !m.images.is_empty());
    let mut with_images = has_images;
    let mut with_json_mode = json_mode;
    let text = loop {
        let request = requests::openai(
            config,
            messages.clone(),
            with_images,
            Mode::Plan {
                max_tokens,
                temperature,
                json_mode: with_json_mode,
            },
        )
        .await?;
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
    processing::openai(text).await
}
/// OpenAI-compatible SSE streaming (`stream: true`).
pub async fn remote_openai_stream<F>(
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
    // Retry once text-only if the endpoint refuses images. on_chunk is untouched
    // until streaming begins, so re-POSTing here is safe.
    let resp = loop {
        let resp = requests::openai(config, messages.clone(), with_images, Mode::Stream)
            .await?
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

    receive_openai(resp, gen_id, on_chunk).await
}

/// Consume the accepted HTTP stream; shared framing preserves fragmented UTF-8.
pub(super) async fn receive_openai<F>(
    response: reqwest::Response,
    gen_id: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send,
{
    let mut full_text = String::new();
    sse::receive(response, gen_id, |data| {
        if data == "[DONE]" {
            return false;
        }
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
        true
    })
    .await?;
    Ok(full_text)
}
