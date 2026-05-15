//! Provider abstraction for the Selah agent.
//!
//! Two concrete variants:
//!   - **Local**: runs Qwen 3.5 2B/4B via llama-cpp-2 (blocking, on-device).
//!   - **Remote**: calls any OpenAI-compatible or Gemini API (SSE streaming).
//!
//! The agent pipeline (`agent.rs`) talks only to the `AgentProvider` enum,
//! so switching between local and remote is transparent.

use crate::agent_error::AgentError;
use crate::ai::{AiConfig, ChatMessage};
use crate::local_ai;

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

// ─────────────────────── Cancel registry (remote) ───────────────────────

static REMOTE_CANCEL: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn cancel_remote(gen_id: &str) {
    if let Ok(mut set) = REMOTE_CANCEL.lock() {
        set.insert(gen_id.to_string());
    }
}

fn is_remote_cancelled(gen_id: &str) -> bool {
    REMOTE_CANCEL
        .lock()
        .map(|s| s.contains(gen_id))
        .unwrap_or(false)
}

fn clear_remote_cancel(gen_id: &str) {
    if let Ok(mut set) = REMOTE_CANCEL.lock() {
        set.remove(gen_id);
    }
}

/// Derive the gen id used during the planning phase. Keeping this prefix
/// distinct from the answer-phase id lets `cancel` clear both reliably even
/// when the same conv id is reused.
fn plan_gen_id(conv_id: &str) -> String {
    if conv_id.is_empty() {
        String::new()
    } else {
        format!("plan:{}", conv_id)
    }
}

fn collect_gemini_text_parts(parts: Option<&serde_json::Value>) -> String {
    parts
        .and_then(|p| p.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<String>()
        })
        .unwrap_or_default()
}

// ─────────────────────── Provider enum ───────────────────────

/// Resolved provider ready to run inference.
pub enum AgentProvider {
    Local { model_id: String, file_name: String },
    Remote { config: AiConfig },
}

impl AgentProvider {
    /// Resolve the correct provider from the user's current AiConfig.
    pub fn resolve() -> Result<Self, AgentError> {
        let cfg = crate::ai::load_ai_config();
        if !cfg.ai_enabled {
            return Err(AgentError::config(
                "AI機能が無効になっています。設定画面で有効にしてください。",
            ));
        }
        match cfg.provider.as_str() {
            "local" => Self::resolve_local(&cfg),
            "openai" | "gemini" => Ok(Self::Remote { config: cfg }),
            other => Err(AgentError::config(format!("不明なプロバイダー: {}", other))),
        }
    }

    fn resolve_local(cfg: &AiConfig) -> Result<Self, AgentError> {
        let catalog = local_ai::model_catalog();
        let info = catalog
            .iter()
            .find(|m| m.id == cfg.local_model)
            .ok_or_else(|| AgentError::model(format!("不明なモデル: {}", cfg.local_model)))?;
        if !local_ai::is_model_downloaded(&info.file_name) {
            return Err(AgentError::model(
                "ローカルモデルがダウンロードされていません。",
            ));
        }
        Ok(Self::Local {
            model_id: info.id.clone(),
            file_name: info.file_name.clone(),
        })
    }

    /// Non-streaming inference used for Phase 1 (planning).
    /// `gen_id` should be the conversation id so cancel requests reach planning too.
    pub async fn plan(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
        temperature: f32,
        prefill: &str,
        think_budget_pct: u32,
        gen_id: &str,
    ) -> Result<String, AgentError> {
        match self {
            Self::Local {
                model_id,
                file_name,
            } => {
                let model_id = model_id.clone();
                let file_name = file_name.clone();
                let prefill = prefill.to_string();
                let gen_id = plan_gen_id(gen_id);
                tokio::task::spawn_blocking(move || {
                    local_ai::run_inference(local_ai::InferenceRequest {
                        model_id,
                        file_name,
                        messages,
                        sampler: local_ai::SamplerConfig::deterministic(temperature),
                        max_tokens,
                        prefill,
                        gen_id,
                        think_budget_pct,
                    })
                })
                .await
                .map_err(AgentError::task)?
                .map_err(AgentError::model)
            }
            Self::Remote { config } => {
                let plan_id = plan_gen_id(gen_id);
                clear_remote_cancel(&plan_id);
                let result =
                    remote_chat_completion(config, messages, max_tokens, temperature, &plan_id)
                        .await
                        .map_err(AgentError::model);
                clear_remote_cancel(&plan_id);
                result
            }
        }
    }

    /// Streaming inference used for Phase 2 (answering).
    /// `on_chunk(text, is_think)` is called for each token/chunk.
    pub async fn answer<F>(
        &self,
        messages: Vec<ChatMessage>,
        gen_id: &str,
        think_budget_pct: u32,
        on_chunk: F,
    ) -> Result<String, AgentError>
    where
        F: FnMut(&str, bool) + Send + 'static,
    {
        match self {
            Self::Local {
                model_id,
                file_name,
            } => {
                let model_id = model_id.clone();
                let file_name = file_name.clone();
                let gen_id = gen_id.to_string();
                tokio::task::spawn_blocking(move || {
                    local_ai::run_inference_streaming(
                        local_ai::InferenceRequest {
                            model_id,
                            file_name,
                            messages,
                            sampler: local_ai::SamplerConfig::default(),
                            max_tokens: 0,
                            prefill: String::new(),
                            gen_id,
                            think_budget_pct,
                        },
                        on_chunk,
                    )
                })
                .await
                .map_err(AgentError::task)?
                .map_err(AgentError::model)
            }
            Self::Remote { config } => {
                let gen_id = gen_id.to_string();
                remote_stream_answer(config, messages, &gen_id, on_chunk, think_budget_pct)
                    .await
                    .map_err(AgentError::model)
            }
        }
    }

    /// Cancel any ongoing inference for `gen_id`.
    /// Cancels both the answer-phase id and the synthesised plan-phase id.
    pub fn cancel(gen_id: &str) {
        local_ai::cancel_inference(gen_id);
        cancel_remote(gen_id);
        let plan_id = plan_gen_id(gen_id);
        if !plan_id.is_empty() {
            local_ai::cancel_inference(&plan_id);
            cancel_remote(&plan_id);
        }
    }

    /// Whether the provider honours assistant prefill (used for Phase 1 JSON).
    /// Local llama-cpp can literally prepend bytes to the assistant turn;
    /// OpenAI/Gemini cannot, so the planner prompt must ask for a full object.
    pub fn supports_prefill(&self) -> bool {
        matches!(self, Self::Local { .. })
    }
}

// ─────────────────────── Remote: non-streaming (plan) ───────────────────────

/// HTTP client shared with `ai.rs`.
fn http_client() -> &'static reqwest::Client {
    // Reuse the same LazyLock-based client from ai.rs.
    // We access it by calling a non-streaming chat completion.
    // For decoupling, we build our own minimal client.
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client")
    });
    &CLIENT
}

async fn remote_chat_completion(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    cancel_id: &str,
) -> Result<String, String> {
    if !cancel_id.is_empty() && is_remote_cancelled(cancel_id) {
        return Err("推論はキャンセルされました".into());
    }
    match config.provider.as_str() {
        "gemini" => remote_gemini_non_streaming(config, messages, max_tokens, temperature).await,
        _ => remote_openai_non_streaming(config, messages, max_tokens, temperature).await,
    }
}

async fn remote_openai_non_streaming(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
) -> Result<String, String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": if max_tokens == 0 { 8192 } else { max_tokens },
        "temperature": temperature,
    });
    let resp = http_client()
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("リクエスト失敗: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;
    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON解析失敗: {}", e))?;
    v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "AIからの応答がありません".into())
}

async fn remote_gemini_non_streaming(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
) -> Result<String, String> {
    let model = urlencoding::encode(&config.model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        model
    );
    let system_text: Vec<String> = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.clone())
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
        .into_iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            serde_json::json!({
                "role": if m.role == "assistant" { "model" } else { "user" },
                "parts": [{ "text": m.content }]
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
    if !system_instruction.is_null() {
        body["systemInstruction"] = system_instruction;
    }
    let resp = http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("リクエスト失敗: {}", e))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;
    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let content = collect_gemini_text_parts(
        v.get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts")),
    );
    if content.is_empty() {
        return Err(format!(
            "AIからの応答がありません: {}",
            truncate(&text, 300)
        ));
    }
    Ok(content)
}

// ─────────────────────── Remote: SSE streaming (answer) ──────────────────

async fn remote_stream_answer<F>(
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
    clear_remote_cancel(gen_id);
    let answer = result?;
    if answer.trim().is_empty() && !is_remote_cancelled(gen_id) {
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
        )
        .await?;
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

/// Stateful `<think>...</think>` splitter: routes content inside the block to
/// `on_chunk(text, true)` and everything else to `on_chunk(text, false)`.
/// Tolerates tag boundaries that cross chunks.
struct ThinkFilter<F: FnMut(&str, bool) + Send + 'static> {
    inner: F,
    buf: String,
    in_think: bool,
}

impl<F: FnMut(&str, bool) + Send + 'static> ThinkFilter<F> {
    /// Returns `(feed, flush)`. `feed(chunk, is_think)` ingests a chunk;
    /// `flush()` drains any buffered tail (call it once the upstream stream
    /// has ended so a trailing partial `<think>` block is not silently lost).
    fn wrap_with_flush(inner: F) -> (Box<dyn FnMut(&str, bool) + Send>, Box<dyn FnMut() + Send>) {
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
                if let Some(idx) = self.buf.find("</think>") {
                    let inside = self.buf[..idx].to_string();
                    if !inside.is_empty() {
                        (self.inner)(&inside, true);
                    }
                    self.buf.drain(..idx + "</think>".len());
                    self.in_think = false;
                    continue;
                }
                // Hold back last 8 chars in case "</think>" straddles chunks.
                let hold = holdback(&self.buf, 8);
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
                if let Some(idx) = self.buf.find("<think>") {
                    let before = self.buf[..idx].to_string();
                    if !before.is_empty() {
                        (self.inner)(&before, false);
                    }
                    self.buf.drain(..idx + "<think>".len());
                    self.in_think = true;
                    continue;
                }
                let hold = holdback(&self.buf, 7);
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

/// Return the byte index up to which it's safe to emit, keeping `keep` bytes
/// in reserve at the tail (so a partial tag isn't cut in half).
fn holdback(s: &str, keep: usize) -> usize {
    if s.len() <= keep {
        return 0;
    }
    let cutoff = s.len() - keep;
    let mut idx = cutoff;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// OpenAI-compatible SSE streaming (`stream: true`).
async fn remote_openai_stream<F>(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    gen_id: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool) + Send + 'static,
{
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": messages,
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
        return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
    }

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

/// Gemini SSE streaming (`streamGenerateContent`).
async fn remote_gemini_stream<F>(
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
        .map(|m| m.content.clone())
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
        .into_iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            serde_json::json!({
                "role": if m.role == "assistant" { "model" } else { "user" },
                "parts": [{ "text": m.content }]
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
        body["systemInstruction"] = system_instruction;
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
        return Err(format!("API error ({}): {}", status, truncate(&text, 300)));
    }

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

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer = buffer[line_end + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
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
    }

    Ok(full_text)
}

// ─────────────────────── Utility ───────────────────────

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}
