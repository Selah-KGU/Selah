//! Decode complete non-streaming responses after network IO releases its executor.
use super::*;

pub(super) async fn error(text: String) -> Result<String, String> {
    crate::background_ipc::run("AIエラー解析失敗", move || Ok(truncate_error(&text))).await
}

pub(super) async fn openai(status: reqwest::StatusCode, text: String) -> Result<String, String> {
    crate::background_ipc::run("AI応答処理失敗", move || parse_openai(status, &text)).await
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

pub(super) async fn gemini(status: reqwest::StatusCode, text: String) -> Result<String, String> {
    crate::background_ipc::run("AI応答処理失敗", move || parse_gemini(status, &text)).await
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
