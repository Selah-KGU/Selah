//! Frozen HTTP request/response behavior before moving CPU work to workers.
use super::*;

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

pub(super) fn parse_openai(status: reqwest::StatusCode, text: &str) -> Result<String, String> {
    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate_error(&text)));
    }

    let parsed: OpenAiResponse =
        serde_json::from_str(text).map_err(|e| format!("レスポンス解析失敗: {}", e))?;

    let choice = parsed.choices.as_ref().and_then(|c| c.first());
    let mut content = collect_openai_message_text(choice.and_then(|c| c.message.content.as_ref()));
    // Reasoning models behind OpenRouter (e.g. minimax with json_object) sometimes
    // return the full answer in `reasoning` and leave `content` empty. Fall back to
    // it rather than failing — sanitize_model_output strips any think wrapper later.
    if content.trim().is_empty() {
        if let Some(reasoning) = choice.and_then(|c| c.message.reasoning.as_deref()) {
            content = reasoning.to_string();
        }
    }
    if content.trim().is_empty() {
        let finish_reason = choice
            .and_then(|c| c.finish_reason.as_deref())
            .unwrap_or("unknown");
        return Err(format!(
            "AIからの応答がありません (finish_reason: {})",
            finish_reason
        ));
    }
    Ok(content)
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

pub(super) fn parse_gemini(status: reqwest::StatusCode, text: &str) -> Result<String, String> {
    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate_error(&text)));
    }

    let parsed: GeminiResponse =
        serde_json::from_str(text).map_err(|e| format!("レスポンス解析失敗: {}", e))?;

    parsed
        .candidates
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.content.as_ref())
        .and_then(|c| c.parts.first())
        .map(|p| p.text.clone())
        .ok_or_else(|| {
            "AIからの応答がありません（安全フィルターによりブロックされた可能性があります）".into()
        })
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
