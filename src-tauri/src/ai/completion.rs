use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;

use super::config::{AiConfig, ChatMessage};
#[path = "completion/processing.rs"]
mod processing;
#[path = "completion/requests.rs"]
mod requests;

/// Shared HTTP client — reuses connection pool across all AI calls.
/// Reasoning models behind OpenRouter can keep streaming a single non-streaming
/// response well past a minute; a short whole-request timeout expires mid-flight
/// and silently kills Live chunk summaries, so we match the planner's 300s.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
});

/// How many times a non-streaming AI request is attempted before giving up.
/// OpenRouter occasionally returns truncated/compressed bodies that fail to
/// parse; a single retry recovers most of them. Transient server-side failures
/// (429 rate limit, 5xx) are the dominant cause of probabilistic Live-summary
/// gaps, so we retry those too with exponential backoff.
const NON_STREAMING_ATTEMPTS: usize = 4;
const NON_STREAMING_BACKOFF_BASE: Duration = Duration::from_millis(750);

/// Compute the wait before the next attempt: honor an explicit `Retry-After`
/// (seconds) when present, otherwise exponential backoff (0.75s, 2.25s, 6.75s).
fn non_streaming_backoff(attempt: usize, retry_after: Option<Duration>) -> Duration {
    if let Some(after) = retry_after {
        // Clamp to avoid an adversarial gateway stalling the flush for minutes.
        return after.min(Duration::from_secs(30));
    }
    NON_STREAMING_BACKOFF_BASE * 3u32.pow((attempt - 1) as u32)
}

/// Send a non-streaming request, retrying transient failures with backoff.
/// Covers transport errors, body-read errors, and retryable status codes
/// (429, 5xx). Mirrors `agent_provider::send_non_streaming_request` so the Live
/// summary path gets the same robustness as the planner path.
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
                    "ai({}): request transport failed on attempt {}/{}; retrying: {}",
                    provider,
                    attempt,
                    NON_STREAMING_ATTEMPTS,
                    error
                );
                tokio::time::sleep(non_streaming_backoff(attempt, None)).await;
                continue;
            }
            Err(error) => return Err(format!("リクエスト失敗: {}", error)),
        };
        let status = resp.status();
        // Retry rate limiting and server errors, which the caller would
        // otherwise surface as a hard `API error (...)` with no recovery.
        if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
            && attempt < NON_STREAMING_ATTEMPTS
        {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let wait = non_streaming_backoff(attempt, retry_after);
            let body = resp.text().await.unwrap_or_default();
            let detail = processing::error(body).await?;
            log::warn!(
                "ai({}): server returned {} on attempt {}/{}; retrying in {:?}: {}",
                provider,
                status,
                attempt,
                NON_STREAMING_ATTEMPTS,
                wait,
                detail
            );
            tokio::time::sleep(wait).await;
            continue;
        }
        match resp.text().await {
            Ok(text) => return Ok((status, text)),
            Err(error) if attempt < NON_STREAMING_ATTEMPTS => {
                log::warn!(
                    "ai({}): response body read failed on attempt {}/{}; retrying: {}",
                    provider,
                    attempt,
                    NON_STREAMING_ATTEMPTS,
                    error
                );
                tokio::time::sleep(non_streaming_backoff(attempt, None)).await;
            }
            Err(error) => {
                return Err(format!("レスポンス読み取り失敗: {}", error));
            }
        }
    }
    Err("AI応答を受信できませんでした".to_string())
}
// ============ OpenAI API types ============

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Option<Vec<OpenAiChoice>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessageResponse,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessageResponse {
    // Some OpenRouter-routed models (e.g. minimax) return `content` as null or as
    // an array of `{type,text}` parts instead of a plain string, and reasoning
    // models can leave `content` null while putting the answer in `reasoning`.
    // Keep these as raw Values so a quirky shape never fails deserialization.
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    reasoning: Option<String>,
}

/// Flatten an OpenAI-compatible `message.content` (string, array of parts, or
/// null) into plain text. Mirrors `agent_provider::collect_openai_message_text`.
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

// ============ Gemini API types ============

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContentResponse>,
}

#[derive(Debug, Deserialize)]
struct GeminiContentResponse {
    parts: Vec<GeminiPartResponse>,
}

#[derive(Debug, Deserialize)]
struct GeminiPartResponse {
    text: String,
}
// ============ API call logic ============

/// Public accessor for other modules (e.g. timetable AI schedule).
pub async fn chat_completion_public(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    chat_completion(config, messages).await
}

pub(in crate::ai) async fn chat_completion(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    if !config.ai_enabled {
        return Err("AI機能が無効になっています。設定画面で有効にしてください。".into());
    }

    match config.provider.as_str() {
        #[cfg(target_os = "macos")]
        "local" => {
            crate::local_ai_support::ensure_supported()?;
            let msgs = messages;
            let temperature = config.temperature;
            tokio::task::spawn_blocking(move || {
                crate::local_ai::run_inference(crate::local_ai::InferenceRequest {
                    model_id: crate::local_ai::APPLE_INTELLIGENCE_MODEL_ID.into(),
                    file_name: String::new(),
                    messages: msgs,
                    sampler: crate::local_ai::SamplerConfig {
                        temperature,
                        ..crate::local_ai::SamplerConfig::default()
                    },
                    max_tokens: 0,
                    prefill: String::new(),
                    gen_id: String::new(),
                    think_budget_pct: 0,
                })
            })
            .await
            .map_err(|e| format!("タスク実行エラー: {}", e))?
        }
        #[cfg(not(target_os = "macos"))]
        "local" => Err(crate::local_ai_support::unsupported_message()),
        "gemini" => call_gemini(config, messages).await,
        _ => call_openai(config, messages).await,
    }
}

async fn call_openai(config: &AiConfig, messages: Vec<ChatMessage>) -> Result<String, String> {
    let request = requests::openai(config, messages).await?;
    let (status, text) = send_non_streaming_request(request, "openai").await?;
    processing::openai(status, text).await
}

async fn call_gemini(config: &AiConfig, messages: Vec<ChatMessage>) -> Result<String, String> {
    let request = requests::gemini(config, messages).await?;
    let (status, text) = send_non_streaming_request(request, "gemini").await?;
    processing::gemini(status, text).await
}

/// Truncate error body to avoid leaking excessive API detail to the frontend.
fn truncate_error(body: &str) -> String {
    // Try to extract a human-friendly message from JSON error responses
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        // OpenAI / OpenRouter format: { "error": { "message": "..." } }
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            let msg = msg.trim();
            if !msg.is_empty() {
                return if msg.len() > 200 {
                    format!(
                        "{}...",
                        &msg[..msg
                            .char_indices()
                            .nth(200)
                            .map(|(i, _)| i)
                            .unwrap_or(msg.len())]
                    )
                } else {
                    msg.to_string()
                };
            }
        }
        // Gemini format: { "error": { "status": "...", "message": "..." } }
        if let Some(status) = v
            .get("error")
            .and_then(|e| e.get("status"))
            .and_then(|s| s.as_str())
        {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            return format!(
                "{}: {}",
                status,
                &msg[..msg
                    .char_indices()
                    .nth(150)
                    .map(|(i, _)| i)
                    .unwrap_or(msg.len())]
            );
        }
    }
    // Fallback: truncate raw body
    match body.char_indices().nth(200) {
        Some((i, _)) => format!("{}...", &body[..i]),
        None => body.to_string(),
    }
}

#[cfg(test)]
#[path = "completion/tests.rs"]
mod tests;
