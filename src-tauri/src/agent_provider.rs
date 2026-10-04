//! Provider abstraction for the Selah agent.
//!
//! Two concrete variants:
//!   - **Local (macOS)**: runs Apple's on-device Foundation Model when Apple
//!     Intelligence is available. Windows stays cloud-only.
//!   - **Remote**: calls any OpenAI-compatible or Gemini API (SSE streaming).
//!
//! The agent pipeline (`agent.rs`) talks only to the `AgentProvider` enum,
//! so switching between local and remote is transparent.

use crate::agent_error::AgentError;
use crate::agent_text;
use crate::ai::{AiConfig, ChatMessage};
#[cfg(target_os = "macos")]
use crate::local_ai;

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

const CANCELLED_MSG: &str = "推論はキャンセルされました";
const NON_STREAMING_ATTEMPTS: usize = 2;

// ─────────────────────── Cancel registry (remote) ───────────────────────

static REMOTE_CANCEL: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static TURN_CANCEL: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

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
    #[cfg(target_os = "macos")]
    Local {
        model_id: String,
        file_name: String,
    },
    Remote {
        config: AiConfig,
    },
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
            #[cfg(target_os = "macos")]
            "local" => {
                crate::local_ai_support::ensure_supported().map_err(AgentError::config)?;
                Self::resolve_local(&cfg)
            }
            #[cfg(not(target_os = "macos"))]
            "local" => Err(AgentError::config(
                crate::local_ai_support::unsupported_message(),
            )),
            // OpenRouter is OpenAI-compatible, so it routes through the OpenAI path.
            "openai" | "openrouter" | "gemini" => Ok(Self::Remote { config: cfg }),
            other => Err(AgentError::config(format!("不明なプロバイダー: {}", other))),
        }
    }

    #[cfg(target_os = "macos")]
    fn resolve_local(cfg: &AiConfig) -> Result<Self, AgentError> {
        let _ = cfg;
        Ok(Self::Local {
            model_id: local_ai::APPLE_INTELLIGENCE_MODEL_ID.into(),
            file_name: String::new(),
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
        #[cfg(not(target_os = "macos"))]
        let _ = (prefill, think_budget_pct);

        if Self::is_cancelled(gen_id) {
            return Err(AgentError::Cancelled);
        }
        match self {
            #[cfg(target_os = "macos")]
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
                .map_err(agent_error_from_model)
            }
            Self::Remote { config } => {
                let plan_id = plan_gen_id(gen_id);
                clear_remote_cancel(&plan_id);
                let result = remote_chat_completion(
                    config,
                    messages,
                    max_tokens,
                    temperature,
                    &plan_id,
                    true,
                )
                .await;
                let cancelled = is_remote_cancelled(&plan_id);
                clear_remote_cancel(&plan_id);
                if cancelled {
                    return Err(AgentError::Cancelled);
                }
                result.map_err(agent_error_from_model)
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
        if Self::is_cancelled(gen_id) {
            return Err(AgentError::Cancelled);
        }
        match self {
            #[cfg(target_os = "macos")]
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
                .map_err(agent_error_from_model)
            }
            Self::Remote { config } => {
                let gen_id = gen_id.to_string();
                remote_stream_answer(config, messages, &gen_id, on_chunk, think_budget_pct)
                    .await
                    .map_err(|e| {
                        if e == CANCELLED_MSG {
                            AgentError::Cancelled
                        } else {
                            AgentError::model(e)
                        }
                    })
            }
        }
    }

    /// Cancel any ongoing inference for `gen_id`.
    /// Cancels both the answer-phase id and the synthesised plan-phase id.
    pub fn cancel(gen_id: &str) {
        if let Ok(mut set) = TURN_CANCEL.lock() {
            set.insert(gen_id.to_string());
        }
        #[cfg(target_os = "macos")]
        local_ai::cancel_inference(gen_id);
        cancel_remote(gen_id);
        let plan_id = plan_gen_id(gen_id);
        if !plan_id.is_empty() {
            #[cfg(target_os = "macos")]
            local_ai::cancel_inference(&plan_id);
            cancel_remote(&plan_id);
        }
    }

    pub fn clear_cancel(gen_id: &str) {
        if let Ok(mut set) = TURN_CANCEL.lock() {
            set.remove(gen_id);
        }
        #[cfg(target_os = "macos")]
        local_ai::clear_inference_cancel(gen_id);
        clear_remote_cancel(gen_id);
        let plan_id = plan_gen_id(gen_id);
        if !plan_id.is_empty() {
            #[cfg(target_os = "macos")]
            local_ai::clear_inference_cancel(&plan_id);
            clear_remote_cancel(&plan_id);
        }
    }

    pub fn is_cancelled(gen_id: &str) -> bool {
        TURN_CANCEL
            .lock()
            .map(|set| set.contains(gen_id))
            .unwrap_or(false)
    }

    /// Whether the provider honours assistant prefill (used for Phase 1 JSON).
    /// Apple Intelligence and the cloud APIs both need a full object, not a prefix.
    pub fn supports_prefill(&self) -> bool {
        let _ = self;
        false
    }

    /// Whether to attempt sending images to this provider. On-device Apple
    /// Intelligence is text-only, so never. For Remote we always TRY (we don't pre-judge any
    /// model — including DeepSeek): vision models receive the image, and if a
    /// text-only endpoint rejects the `image_url` part the request layer falls
    /// back to text-only for that one call instead of failing.
    pub fn supports_vision(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    pub fn is_local(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            Self::Local { .. } => true,
            Self::Remote { .. } => false,
        }
    }
}

/// Whether an API error body indicates the model/endpoint refused image input
/// (so the request can be retried text-only instead of hard-failing).
fn is_image_unsupported_error(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("image_url")
        || b.contains("inlinedata")
        || b.contains("multimodal")
        || (b.contains("image")
            && (b.contains("unknown variant")
                || b.contains("not support")
                || b.contains("unsupported")
                || b.contains("invalid")
                || b.contains("deserialize")))
}

fn agent_error_from_model(e: String) -> AgentError {
    if e == CANCELLED_MSG {
        AgentError::Cancelled
    } else {
        AgentError::model(format_remote_api_error(&e).unwrap_or(e))
    }
}

fn format_remote_api_error(raw: &str) -> Option<String> {
    let (status_label, payload) = parse_api_error_payload(raw)?;
    let parsed: serde_json::Value = serde_json::from_str(payload).ok()?;
    let error = parsed.get("error").unwrap_or(&parsed);
    let code = error.get("code").and_then(|v| v.as_i64());
    let api_status = error.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let message = error
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if code.is_none() && api_status.is_empty() && message.is_empty() {
        return None;
    }

    let mut details = Vec::new();
    if let Some(code) = code {
        details.push(format!("code={code}"));
    }
    if !api_status.is_empty() {
        details.push(format!("status={api_status}"));
    }
    if !message.is_empty() {
        details.push(format!("message={message}"));
    }
    Some(format!(
        "远端 AI provider 请求失败（{status_label}{}）。这表示当前配置的 API key/项目/模型权限不可用，不是浏览器本地工具或网页登录失败。请检查 Settings 里的 provider、API key、模型名，以及对应云项目的 API 启用、权限、配额/计费状态。",
        if details.is_empty() {
            String::new()
        } else {
            format!("; {}", details.join(", "))
        }
    ))
}

fn parse_api_error_payload(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("API error (")?;
    let (status_label, after_status) = rest.split_once("):")?;
    let payload = after_status.trim();
    if payload.starts_with('{') {
        Some((status_label.trim(), payload))
    } else {
        None
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
            // Planning calls can return large JSON payloads. A short whole-request
            // timeout can expire while a healthy provider is still sending them.
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client")
    });
    &CLIENT
}

async fn send_non_streaming_request(
    request: reqwest::RequestBuilder,
    provider: &str,
) -> Result<(reqwest::StatusCode, String), String> {
    for attempt in 1..=NON_STREAMING_ATTEMPTS {
        let request = request
            .try_clone()
            .ok_or_else(|| "AIリクエストを再試行用に複製できませんでした".to_string())?;
        let resp = match request.send().await {
            Ok(resp) => resp,
            Err(error) if attempt < NON_STREAMING_ATTEMPTS => {
                log::warn!(
                    "plan({}): request transport failed on attempt {}/{}; retrying: {}",
                    provider,
                    attempt,
                    NON_STREAMING_ATTEMPTS,
                    error
                );
                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                continue;
            }
            Err(error) => return Err(format!("リクエスト失敗: {}", error)),
        };
        let status = resp.status();
        match resp.text().await {
            Ok(text) => return Ok((status, text)),
            Err(error) => {
                return Err(format!(
                    "AI応答の受信が途中で中断されました。重複生成を避けるため自動再試行しません: {}",
                    error
                ));
            }
        }
    }
    Err("AI応答を受信できませんでした".to_string())
}

async fn remote_chat_completion(
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

/// OpenAI chat message JSON — multimodal `content` array when the message
/// carries images (e.g. an agent screenshot), plain string otherwise.
fn openai_message_json(m: &ChatMessage) -> serde_json::Value {
    if m.images.is_empty() {
        return serde_json::json!({ "role": m.role, "content": m.content });
    }
    let mut parts = vec![serde_json::json!({ "type": "text", "text": m.content })];
    for img in &m.images {
        parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", img.mime, img.data_base64) },
        }));
    }
    serde_json::json!({ "role": m.role, "content": parts })
}

/// Gemini content parts — appends inline image data parts when present.
fn gemini_parts_json(m: &ChatMessage) -> Vec<serde_json::Value> {
    let mut parts = vec![serde_json::json!({
        "text": agent_text::neutralize_pseudo_tool_calls(&m.content)
    })];
    for img in &m.images {
        parts.push(serde_json::json!({
            "inlineData": { "mimeType": img.mime, "data": img.data_base64 },
        }));
    }
    parts
}

/// OpenAI messages array; `with_images=false` drops image parts (text-only
/// fallback for endpoints that reject `image_url`).
fn openai_messages_json(messages: &[ChatMessage], with_images: bool) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            if with_images && !m.images.is_empty() {
                openai_message_json(m)
            } else {
                serde_json::json!({ "role": m.role, "content": m.content })
            }
        })
        .collect()
}

/// Gemini content parts for one message; `with_images=false` drops image parts.
fn gemini_message_parts(m: &ChatMessage, with_images: bool) -> Vec<serde_json::Value> {
    if with_images {
        gemini_parts_json(m)
    } else {
        vec![serde_json::json!({
            "text": agent_text::neutralize_pseudo_tool_calls(&m.content)
        })]
    }
}

async fn remote_openai_non_streaming(
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

fn collect_openai_message_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(|text| text.as_str())
                    .or_else(|| part.get("content").and_then(|text| text.as_str()))
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn response_format_unsupported(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("response_format")
        || lower.contains("response format")
        || lower.contains("json_object")
}

async fn remote_gemini_non_streaming(
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
struct ThinkFilter<F: FnMut(&str, bool) + Send + 'static> {
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

// ─────────────────────── Utility ───────────────────────

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

fn gemini_malformed_function_call(v: &serde_json::Value) -> Option<String> {
    v.get("candidates")?
        .as_array()?
        .iter()
        .find_map(|candidate| {
            let reason = candidate.get("finishReason")?.as_str()?;
            if reason != "MALFORMED_FUNCTION_CALL" {
                return None;
            }
            let message = candidate.get("finishMessage")?.as_str()?;
            extract_pseudo_call_from_finish_message(message)
        })
}

fn gemini_function_call(v: &serde_json::Value) -> Option<String> {
    v.get("candidates")?
        .as_array()?
        .iter()
        .find_map(|candidate| {
            candidate
                .get("content")?
                .get("parts")?
                .as_array()?
                .iter()
                .find_map(|part| {
                    let call = part.get("functionCall")?;
                    let name = call.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    let args = call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    let args = if args.is_object() {
                        args
                    } else {
                        serde_json::json!({})
                    };
                    let args = serde_json::to_string(&args).ok()?;
                    Some(format!("call:{name}{args}"))
                })
        })
}

fn extract_pseudo_call_from_finish_message(message: &str) -> Option<String> {
    let message = message
        .trim()
        .strip_prefix("Malformed function call:")
        .unwrap_or(message)
        .trim();
    agent_text::extract_pseudo_call_from_text(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ImagePart;

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
}
