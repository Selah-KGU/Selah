//! HTTP request assembly/serialization and large input destruction run on workers.
use super::*;

pub(super) async fn openai(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<reqwest::RequestBuilder, String> {
    prepare(config.clone(), messages, build_openai).await
}
pub(super) async fn gemini(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<reqwest::RequestBuilder, String> {
    prepare(config.clone(), messages, build_gemini).await
}
pub(super) async fn prepare<R: Send + 'static>(
    config: AiConfig,
    messages: Vec<ChatMessage>,
    work: impl FnOnce(AiConfig, Vec<ChatMessage>) -> R + Send + 'static,
) -> Result<R, String> {
    crate::background_ipc::run("AIリクエスト準備失敗", move || {
        Ok(work(config, messages))
    })
    .await
}

pub(super) fn build_openai(
    config: AiConfig,
    messages: Vec<ChatMessage>,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    let body = OpenAiRequest {
        model: config.model.clone(),
        messages,
        max_tokens: config.max_tokens,
        temperature: config.temperature,
    };

    let request = HTTP_CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        // Ask the gateway not to compress: OpenRouter has been observed sending
        // truncated gzip bodies that fail to parse. Identity encoding sidesteps it.
        .header("Accept-Encoding", "identity")
        .json(&body);
    request
}

pub(super) fn build_gemini(
    config: AiConfig,
    messages: Vec<ChatMessage>,
) -> reqwest::RequestBuilder {
    let model = urlencoding::encode(&config.model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        model
    );

    // Extract system instruction from messages
    let system_instruction = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| sanitize_for_gemini(&m.content))
        .collect::<Vec<_>>();

    let system_instruction = if system_instruction.is_empty() {
        None
    } else {
        Some(GeminiContent {
            role: "user".into(), // Gemini systemInstruction uses "user" role
            parts: vec![GeminiPart {
                text: system_instruction.join("\n"),
            }],
        })
    };

    let contents: Vec<GeminiContent> = messages
        .into_iter()
        .filter(|m| m.role != "system")
        .map(|m| GeminiContent {
            role: if m.role == "assistant" {
                "model".into()
            } else {
                "user".into()
            },
            parts: vec![GeminiPart {
                text: sanitize_for_gemini(&m.content),
            }],
        })
        .collect();

    let body = GeminiRequest {
        contents,
        generation_config: GeminiGenerationConfig {
            max_output_tokens: config.max_tokens,
            temperature: config.temperature,
        },
        system_instruction,
    };

    let request = HTTP_CLIENT
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &config.api_key) // Header auth, not URL query
        .header("Accept-Encoding", "identity")
        .json(&body);
    request
}

fn sanitize_for_gemini(text: &str) -> String {
    let mut cleaned = text.replace("<call:", "[call:");
    cleaned = cleaned.replace("</call:", "[/call:");
    cleaned = cleaned.replace("<call ", "[call ");
    cleaned = cleaned.replace("task_call:", "task-call:");
    cleaned = cleaned.replace("tool_call:", "tool-call:");
    cleaned = cleaned.replace("function_call:", "function-call:");
    cleaned = cleaned.replace("call:", "c-all:");
    cleaned = cleaned.replace('(', "（");
    cleaned = cleaned.replace(')', "）");
    cleaned = cleaned.replace('<', "＜");
    cleaned = cleaned.replace('>', "＞");
    cleaned = cleaned.replace('‹', "〈");
    cleaned = cleaned.replace('›', "〉");
    cleaned
}
