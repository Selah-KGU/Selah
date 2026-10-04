use serde_json::Value;

use super::text::{sanitize_text_arg, sanitize_text_blob_arg};

pub(in crate::agent_tools) fn sanitize_browser_target_arg(args: &Value) -> Option<String> {
    sanitize_text_arg(args, "target", 120)
}

pub(in crate::agent_tools) fn sanitize_selector_arg(
    args: &Value,
    key: &str,
    max_len: usize,
) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() || value.len() > max_len || value.contains('\0') {
        return None;
    }
    let value = value.replace(['\n', '\r'], " ");
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(in crate::agent_tools) fn sanitize_small_index(
    args: &Value,
    key: &str,
    max: u64,
) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v.min(max))
}

pub(in crate::agent_tools) fn sanitize_browser_coord(args: &Value, key: &str) -> Option<u64> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v.min(20_000))
}

pub(in crate::agent_tools) fn sanitize_browser_click_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let text = sanitize_text_arg(args, "text", 120);
    let href_contains = sanitize_text_arg(args, "href_contains", 240);
    let index = sanitize_small_index(args, "index", 20).unwrap_or(0);
    if selector.is_none() && text.is_none() && href_contains.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    if let Some(text) = text {
        out.insert("text".into(), Value::String(text));
    }
    if let Some(href_contains) = href_contains {
        out.insert("href_contains".into(), Value::String(href_contains));
    }
    if index > 0 {
        out.insert("index".into(), Value::Number(index.into()));
    }
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_browser_mouse_click_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let x = sanitize_browser_coord(args, "x")?;
    let y = sanitize_browser_coord(args, "y")?;
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("x".into(), Value::Number(x.into()));
    out.insert("y".into(), Value::Number(y.into()));
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_browser_mouse_drag_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let from_x =
        sanitize_browser_coord(args, "from_x").or_else(|| sanitize_browser_coord(args, "fromX"))?;
    let from_y =
        sanitize_browser_coord(args, "from_y").or_else(|| sanitize_browser_coord(args, "fromY"))?;
    let to_x =
        sanitize_browser_coord(args, "to_x").or_else(|| sanitize_browser_coord(args, "toX"))?;
    let to_y =
        sanitize_browser_coord(args, "to_y").or_else(|| sanitize_browser_coord(args, "toY"))?;
    let steps = args
        .get("steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(2, 24);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("from_x".into(), Value::Number(from_x.into()));
    out.insert("from_y".into(), Value::Number(from_y.into()));
    out.insert("to_x".into(), Value::Number(to_x.into()));
    out.insert("to_y".into(), Value::Number(to_y.into()));
    out.insert("steps".into(), Value::Number(steps.into()));
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_browser_fill_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let label = sanitize_text_arg(args, "label", 120);
    let value = sanitize_text_blob_arg(args, "value", 2000)?;
    let index = sanitize_small_index(args, "index", 20).unwrap_or(0);
    if selector.is_none() && label.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    if let Some(label) = label {
        out.insert("label".into(), Value::String(label));
    }
    out.insert("value".into(), Value::String(value));
    if index > 0 {
        out.insert("index".into(), Value::Number(index.into()));
    }
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_browser_select_args(args: &Value) -> Option<Value> {
    sanitize_browser_fill_args(args)
}

pub(in crate::agent_tools) fn normalize_browser_key(raw: &str) -> Option<String> {
    let key = raw.trim();
    if key.is_empty() || key.len() > 32 {
        return None;
    }
    let normalized = match key.to_ascii_lowercase().as_str() {
        "enter" => "Enter",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "arrowup" | "up" => "ArrowUp",
        "arrowdown" | "down" => "ArrowDown",
        "arrowleft" | "left" => "ArrowLeft",
        "arrowright" | "right" => "ArrowRight",
        "space" | "spacebar" => " ",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "home" => "Home",
        "end" => "End",
        _ => key,
    };
    Some(normalized.to_string())
}

pub(in crate::agent_tools) fn sanitize_browser_press_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .and_then(normalize_browser_key)?;
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    out.insert("key".into(), Value::String(key));
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_browser_scroll_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| matches!(v.as_str(), "up" | "down" | "top" | "bottom"))
        .unwrap_or_else(|| "down".into());
    let amount = args
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(900)
        .clamp(80, 4000);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    out.insert("direction".into(), Value::String(direction));
    out.insert("amount".into(), Value::Number(amount.into()));
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_browser_wait_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let text = sanitize_text_arg(args, "text", 160);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(3000)
        .clamp(400, 12_000);
    if selector.is_none() && text.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    if let Some(text) = text {
        out.insert("text".into(), Value::String(text));
    }
    out.insert("timeout_ms".into(), Value::Number(timeout_ms.into()));
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_coordinate_space_arg(args: &Value) -> Option<String> {
    args.get("coordinate_space")
        .or_else(|| args.get("coordinateSpace"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| {
            matches!(
                v.as_str(),
                "screenshot" | "target" | "screen" | "webview" | "viewport"
            )
        })
}
