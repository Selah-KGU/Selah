//! Gemini function-call and malformed-call extraction.

use crate::agent_text;

pub fn gemini_malformed_function_call(v: &serde_json::Value) -> Option<String> {
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

pub fn gemini_function_call(v: &serde_json::Value) -> Option<String> {
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
