//! Provider dispatch for non-streaming and streaming answers.

use super::*;
use crate::agent_text;
use crate::ai::{AiConfig, ChatMessage};
use std::sync::{Arc, Mutex};

pub async fn remote_chat_completion(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
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
    let stream_messages = messages.clone();
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
        let fallback = remote_chat_completion(
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
/// Stateful thinking-tag splitter: routes content inside the block to
/// `on_chunk(text, true)` and everything else to `on_chunk(text, false)`.
/// Tolerates tag boundaries that cross chunks.
pub struct ThinkFilter<F: FnMut(&str, bool) + Send + 'static> {
    inner: F,
    buf: String,
    in_think: bool,
}

impl<F: FnMut(&str, bool) + Send + 'static> ThinkFilter<F> {
    /// Returns `(feed, flush)`. `feed(chunk, is_think)` ingests a chunk;
    /// `flush()` drains any buffered tail (call it once the upstream stream
    /// has ended so a trailing partial thinking block is not silently lost).
    // The return-tuple type expresses exactly what callers consume; abstracting
    // into a `type` alias would just rename it without simplifying the API.
    #[allow(clippy::type_complexity)]
    pub fn wrap_with_flush(
        inner: F,
    ) -> (Box<dyn FnMut(&str, bool) + Send>, Box<dyn FnMut() + Send>) {
        let state = std::sync::Arc::new(std::sync::Mutex::new(ThinkFilter {
            inner,
            buf: String::new(),
            in_think: false,
        }));
        let feed_state = state.clone();
        let feed = Box::new(move |chunk: &str, is_think: bool| {
            let mut guard = match feed_state.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if is_think {
                (guard.inner)(chunk, true);
                return;
            }
            guard.buf.push_str(chunk);
            guard.drain(false);
        });
        let flush_state = state;
        let flush = Box::new(move || {
            if let Ok(mut guard) = flush_state.lock() {
                guard.drain(true);
            }
        });
        (feed, flush)
    }

    fn drain(&mut self, flush: bool) {
        loop {
            if self.in_think {
                if let Some((idx, tag_len)) = agent_text::find_thinking_end_tag(&self.buf) {
                    let inside = self.buf[..idx].to_string();
                    if !inside.is_empty() {
                        (self.inner)(&inside, true);
                    }
                    self.buf.drain(..idx + tag_len);
                    self.in_think = false;
                    continue;
                }
                let hold = agent_text::holdback(&self.buf, agent_text::THINKING_TAG_HOLDBACK);
                if hold > 0 {
                    let emit = self.buf[..hold].to_string();
                    (self.inner)(&emit, true);
                    self.buf.drain(..hold);
                }
                if flush && !self.buf.is_empty() {
                    let emit = std::mem::take(&mut self.buf);
                    (self.inner)(&emit, true);
                }
                return;
            } else {
                if let Some((idx, tag_len)) = agent_text::find_thinking_start_tag(&self.buf) {
                    let before = self.buf[..idx].to_string();
                    if !before.is_empty() {
                        (self.inner)(&before, false);
                    }
                    self.buf.drain(..idx + tag_len);
                    self.in_think = true;
                    continue;
                }
                let hold = agent_text::holdback(&self.buf, agent_text::THINKING_TAG_HOLDBACK);
                if hold > 0 {
                    let emit = self.buf[..hold].to_string();
                    (self.inner)(&emit, false);
                    self.buf.drain(..hold);
                }
                if flush && !self.buf.is_empty() {
                    let emit = std::mem::take(&mut self.buf);
                    (self.inner)(&emit, false);
                }
                return;
            }
        }
    }
}
