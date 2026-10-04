//! Remote API error classification and formatting.

use super::*;
use crate::agent_error::AgentError;

/// Whether an API error body indicates the model/endpoint refused image input
/// (so the request can be retried text-only instead of hard-failing).
pub fn is_image_unsupported_error(body: &str) -> bool {
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

pub fn agent_error_from_model(e: String) -> AgentError {
    if e == CANCELLED_MSG {
        AgentError::Cancelled
    } else {
        AgentError::model(format_remote_api_error(&e).unwrap_or(e))
    }
}

pub fn format_remote_api_error(raw: &str) -> Option<String> {
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
pub fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}
