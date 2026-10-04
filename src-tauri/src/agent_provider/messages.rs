//! OpenAI and Gemini message payload builders.

use crate::agent_text;
use crate::ai::ChatMessage;

pub fn collect_gemini_text_parts(parts: Option<&serde_json::Value>) -> String {
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
/// OpenAI chat message JSON — multimodal `content` array when the message
/// carries images (e.g. an agent screenshot), plain string otherwise.
pub fn openai_message_json(m: &ChatMessage) -> serde_json::Value {
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
pub fn gemini_parts_json(m: &ChatMessage) -> Vec<serde_json::Value> {
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
pub fn openai_messages_json(messages: &[ChatMessage], with_images: bool) -> Vec<serde_json::Value> {
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
pub fn gemini_message_parts(m: &ChatMessage, with_images: bool) -> Vec<serde_json::Value> {
    if with_images {
        gemini_parts_json(m)
    } else {
        vec![serde_json::json!({
            "text": agent_text::neutralize_pseudo_tool_calls(&m.content)
        })]
    }
}
pub fn collect_openai_message_text(content: Option<&serde_json::Value>) -> String {
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

pub fn response_format_unsupported(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("response_format")
        || lower.contains("response format")
        || lower.contains("json_object")
}
