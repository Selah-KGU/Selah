//! Share immutable input across streaming, compatibility retries and fallback.
//! JSON construction/encoding and temporary buffer destruction run on workers.
use super::*;
use crate::agent_text;
use crate::ai::{AiConfig, ChatMessage};
use std::sync::Arc;

pub(super) type SharedMessages = Arc<Vec<ChatMessage>>;

pub(super) fn share_messages(messages: Vec<ChatMessage>) -> SharedMessages {
    Arc::new(messages)
}

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Plan {
        max_tokens: u32,
        temperature: f32,
        json_mode: bool,
    },
    Stream,
}

impl Mode {
    fn parameters(self, config: &AiConfig) -> (u32, f32, bool) {
        match self {
            Self::Plan {
                max_tokens,
                temperature,
                json_mode,
            } => (
                if max_tokens == 0 { 8192 } else { max_tokens },
                temperature,
                json_mode,
            ),
            Self::Stream => (
                if config.max_tokens == 0 {
                    32768
                } else {
                    config.max_tokens
                },
                config.temperature,
                false,
            ),
        }
    }
}

pub(super) async fn openai(
    config: &AiConfig,
    messages: SharedMessages,
    with_images: bool,
    mode: Mode,
) -> Result<reqwest::RequestBuilder, String> {
    prepare(config.clone(), messages, move |config, messages| {
        build_openai(&config, &messages, with_images, mode)
    })
    .await
}

pub(super) async fn gemini(
    config: &AiConfig,
    messages: SharedMessages,
    with_images: bool,
    mode: Mode,
) -> Result<reqwest::RequestBuilder, String> {
    prepare(config.clone(), messages, move |config, messages| {
        build_gemini(&config, &messages, with_images, mode)
    })
    .await
}

pub(super) async fn prepare<R: Send + 'static>(
    config: AiConfig,
    messages: SharedMessages,
    work: impl FnOnce(AiConfig, SharedMessages) -> R + Send + 'static,
) -> Result<R, String> {
    crate::background_ipc::run("Agentリクエスト準備失敗", move || {
        Ok(work(config, messages))
    })
    .await
}

fn build_openai(
    config: &AiConfig,
    messages: &[ChatMessage],
    with_images: bool,
    mode: Mode,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let (max_tokens, temperature, json_mode) = mode.parameters(config);
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": openai_messages_json(messages, with_images),
        "max_tokens": max_tokens,
        "temperature": temperature,
    });
    if matches!(mode, Mode::Stream) {
        body["stream"] = serde_json::Value::Bool(true);
    }
    if json_mode {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    let mut request = http_client()
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");
    if matches!(mode, Mode::Plan { .. }) {
        // Preserve the plan path's gateway truncation workaround.
        request = request.header("Accept-Encoding", "identity");
    }
    request.json(&body)
}

fn build_gemini(
    config: &AiConfig,
    messages: &[ChatMessage],
    with_images: bool,
    mode: Mode,
) -> reqwest::RequestBuilder {
    let model = urlencoding::encode(&config.model);
    let action = if matches!(mode, Mode::Stream) {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:{action}");
    let system_text: Vec<String> = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| agent_text::neutralize_pseudo_tool_calls(&m.content))
        .collect();
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
    let (max_tokens, temperature, json_mode) = mode.parameters(config);
    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": { "maxOutputTokens": max_tokens, "temperature": temperature },
    });
    if json_mode {
        body["generationConfig"]["responseMimeType"] =
            serde_json::Value::String("application/json".into());
    }
    if !system_text.is_empty() {
        body["systemInstruction"] = serde_json::json!({
            "role": "user", "parts": [{ "text": system_text.join("\n") }]
        });
    }
    let mut request = http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &config.api_key);
    if matches!(mode, Mode::Plan { .. }) {
        request = request.header("Accept-Encoding", "identity");
    }
    request.json(&body)
}
