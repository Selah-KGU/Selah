//! Provider dispatch for non-streaming and streaming answers.

use super::*;
use crate::agent_text;
use crate::ai::{AiConfig, ChatMessage};
use std::sync::{Arc, Mutex};

mod think_filter;
pub(super) use think_filter::ThinkFilter;

pub async fn remote_chat_completion(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    cancel_id: &str,
    json_mode: bool,
) -> Result<String, String> {
    remote_chat_completion_shared(
        config,
        requests::share_messages(messages),
        max_tokens,
        temperature,
        cancel_id,
        json_mode,
    )
    .await
}

async fn remote_chat_completion_shared(
    config: &AiConfig,
    messages: requests::SharedMessages,
    max_tokens: u32,
    temperature: f32,
    cancel_id: &str,
    json_mode: bool,
) -> Result<String, String> {
    if !cancel_id.is_empty() && is_remote_cancelled(cancel_id) {
        return Err("推論はキャンセルされました".into());
    }
    match config.provider.as_str() {
        "gemini" => {
            remote_gemini_non_streaming(config, messages, max_tokens, temperature, json_mode).await
        }
        _ => {
            remote_openai_non_streaming(config, messages, max_tokens, temperature, json_mode).await
        }
    }
}
// ─────────────────────── Remote: SSE streaming (answer) ──────────────────

pub async fn remote_stream_answer<F>(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    gen_id: &str,
    on_chunk: F,
    think_budget_pct: u32,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send + 'static,
{
    clear_remote_cancel(gen_id);
    let callback = Arc::new(Mutex::new(on_chunk));
    let callback_for_stream = callback.clone();
    let (mut filtered, mut flush) =
        ThinkFilter::wrap_with_flush(move |chunk: &str, is_think: bool| {
            if let Ok(mut cb) = callback_for_stream.lock() {
                (*cb)(chunk, is_think);
            }
        });
    let messages = requests::share_messages(messages);
    let stream_messages = Arc::clone(&messages);
    let stream_callback = move |chunk: &str, is_think: bool| filtered(chunk, is_think);
    let result = match config.provider.as_str() {
        "gemini" => remote_gemini_stream(config, stream_messages, gen_id, stream_callback).await,
        _ => remote_openai_stream(config, stream_messages, gen_id, stream_callback).await,
    };
    flush();
    let cancelled = is_remote_cancelled(gen_id);
    clear_remote_cancel(gen_id);
    if cancelled {
        return Err(CANCELLED_MSG.into());
    }
    let answer = result?;
    if answer.trim().is_empty() {
        log::warn!(
            "[agent answer] streaming produced empty visible text; falling back to non-streaming provider={}",
            config.provider
        );
        let fallback = remote_chat_completion_shared(
            config,
            messages,
            if config.max_tokens == 0 {
                32768
            } else {
                config.max_tokens
            },
            config.temperature,
            gen_id,
            false,
        )
        .await?;
        if agent_text::contains_leading_pseudo_tool_call(&fallback) {
            log::warn!(
                "[agent answer] non-streaming fallback returned pseudo tool call; returning without emitting"
            );
            return Ok(fallback);
        }
        if !fallback.is_empty() {
            let callback_for_fallback = callback.clone();
            let (mut feed, mut flush_fb) =
                ThinkFilter::wrap_with_flush(move |chunk: &str, is_think: bool| {
                    if let Ok(mut cb) = callback_for_fallback.lock() {
                        (*cb)(chunk, is_think);
                    }
                });
            feed(&fallback, false);
            flush_fb();
        }
        let _ = think_budget_pct;
        return Ok(fallback);
    }
    Ok(answer)
}
