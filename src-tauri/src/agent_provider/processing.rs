//! Full non-streaming response parsing runs outside the async executor.
use super::*;
pub(super) async fn openai(text: String) -> Result<String, String> {
    crate::background_ipc::run("Agent応答処理失敗", move || parse_openai(&text)).await
}
fn parse_openai(text: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON解析失敗: {}", e))?;
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
pub(super) async fn gemini(text: String) -> Result<String, String> {
    crate::background_ipc::run("Agent応答処理失敗", move || parse_gemini(&text)).await
}
fn parse_gemini(text: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON解析失敗: {}", e))?;
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
        return Err(format!("AIからの応答がありません: {}", truncate(text, 300)));
    }
    Ok(content)
}
