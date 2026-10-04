use serde_json::{json, Value};

use super::super::text::{compact_string_list, compact_text};
use super::support::{browser_rect_value, resolve_browser_target_from_args};

pub async fn read_browser_page(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let status_guard =
        crate::webview_toolbar::BrowserAgentStatusGuard::start(app, &target, "read_page");
    let payload_result = crate::webview_toolbar::extract_page_text(app, &target).await;
    let payload = payload_result?;
    let headings = compact_string_list(&payload.headings, 10, 140);
    let links: Vec<Value> = payload
        .links
        .iter()
        .filter_map(|link| {
            let text = compact_text(&link.text, 120);
            let url = compact_text(&link.url, 240);
            if text.is_none() && url.is_none() {
                return None;
            }
            let mut item = serde_json::Map::new();
            if let Some(text) = text {
                item.insert("text".into(), Value::String(text));
            }
            if let Some(url) = url {
                item.insert("url".into(), Value::String(url));
            }
            if let Some(rect) = browser_rect_value(&link.rect) {
                item.insert("rect".into(), rect);
            }
            Some(Value::Object(item))
        })
        .take(24)
        .collect();
    let buttons: Vec<Value> = payload
        .buttons
        .iter()
        .filter_map(|button| {
            let text = compact_text(&button.text, 120)?;
            let mut item = serde_json::Map::new();
            item.insert("text".into(), Value::String(text));
            if let Some(kind) = compact_text(&button.kind, 32) {
                item.insert("type".into(), Value::String(kind));
            }
            if let Some(rect) = browser_rect_value(&button.rect) {
                item.insert("rect".into(), rect);
            }
            Some(Value::Object(item))
        })
        .take(10)
        .collect();
    let inputs: Vec<Value> = payload
        .inputs
        .iter()
        .filter_map(|input| {
            let label = compact_text(&input.label, 120);
            let name = compact_text(&input.name, 80);
            let placeholder = compact_text(&input.placeholder, 120);
            let value = compact_text(&input.value, 120);
            let kind = compact_text(&input.kind, 32);
            if label.is_none()
                && name.is_none()
                && placeholder.is_none()
                && value.is_none()
                && kind.is_none()
            {
                return None;
            }
            let mut item = serde_json::Map::new();
            if let Some(label) = label {
                item.insert("label".into(), Value::String(label));
            }
            if let Some(kind) = kind {
                item.insert("type".into(), Value::String(kind));
            }
            if let Some(name) = name {
                item.insert("name".into(), Value::String(name));
            }
            if let Some(placeholder) = placeholder {
                item.insert("placeholder".into(), Value::String(placeholder));
            }
            if let Some(value) = value {
                item.insert("value".into(), Value::String(value));
            }
            if let Some(rect) = browser_rect_value(&input.rect) {
                item.insert("rect".into(), rect);
            }
            if input.required {
                item.insert("required".into(), Value::Bool(true));
            }
            if input.disabled {
                item.insert("disabled".into(), Value::Bool(true));
            }
            Some(Value::Object(item))
        })
        .take(10)
        .collect();
    let result = json!({
        "target": target,
        "title": compact_text(&payload.title, 200).unwrap_or_default(),
        "url": payload.url,
        "viewport": payload.viewport,
        "content_source": compact_text(&payload.content_source, 40).unwrap_or_else(|| "document".into()),
        "content": compact_text(&payload.text, 8_000).unwrap_or_default(),
        "headings": headings,
        "links": links,
        "interactive_elements": {
            "buttons": buttons,
            "inputs": inputs,
        },
    });
    status_guard.finish();
    Ok(result)
}
