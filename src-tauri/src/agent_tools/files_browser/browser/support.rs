use serde_json::{json, Value};

use super::super::text::{compact_string_list, compact_text};

pub(in crate::agent_tools) fn resolve_browser_target_from_args(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<String, String> {
    crate::webview_toolbar::resolve_browser_target(app, args.get("target").and_then(|v| v.as_str()))
}

fn browser_action_failed_message(result: &Value, fallback: &str) -> Option<String> {
    match result.get("ok").and_then(|v| v.as_bool()) {
        Some(true) => None,
        _ => Some(
            result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or(fallback)
                .to_string(),
        ),
    }
}

fn browser_action_label(action: &Value) -> String {
    action
        .get("kind")
        .or_else(|| action.get("action"))
        .and_then(|v| v.as_str())
        .unwrap_or("browser_action")
        .to_string()
}

pub(in crate::agent_tools) fn browser_rect_value<T: serde::Serialize>(
    rect: &Option<T>,
) -> Option<Value> {
    rect.as_ref()
        .and_then(|rect| serde_json::to_value(rect).ok())
}

pub(in crate::agent_tools) fn compact_browser_observation(
    payload: &crate::webview_toolbar::PageTextPayload,
) -> Value {
    let buttons: Vec<Value> = payload
        .buttons
        .iter()
        .filter_map(|button| {
            let text = compact_text(&button.text, 80)?;
            let mut item = serde_json::Map::new();
            item.insert("text".into(), Value::String(text));
            item.insert(
                "type".into(),
                Value::String(compact_text(&button.kind, 32).unwrap_or_default()),
            );
            if let Some(rect) = browser_rect_value(&button.rect) {
                item.insert("rect".into(), rect);
            }
            Some(Value::Object(item))
        })
        .take(6)
        .collect();
    let inputs: Vec<Value> = payload
        .inputs
        .iter()
        .filter_map(|input| {
            let label = compact_text(&input.label, 80)
                .or_else(|| compact_text(&input.name, 60))
                .or_else(|| compact_text(&input.placeholder, 80))?;
            let mut item = serde_json::Map::new();
            item.insert("label".into(), Value::String(label));
            item.insert(
                "type".into(),
                Value::String(compact_text(&input.kind, 32).unwrap_or_default()),
            );
            if let Some(rect) = browser_rect_value(&input.rect) {
                item.insert("rect".into(), rect);
            }
            Some(Value::Object(item))
        })
        .take(6)
        .collect();
    json!({
        "title": compact_text(&payload.title, 160).unwrap_or_default(),
        "url": payload.url,
        "viewport": payload.viewport,
        "headings": compact_string_list(&payload.headings, 5, 120),
        "interactive_elements": {
            "buttons": buttons,
            "inputs": inputs,
        },
    })
}

pub(in crate::agent_tools) async fn run_browser_action_tool(
    app: &tauri::AppHandle,
    target: &str,
    action: Value,
    timeout_ms: u64,
    settle_ms: u64,
    fallback_error: &str,
) -> Result<Value, String> {
    let action_label = browser_action_label(&action);
    let status_guard =
        crate::webview_toolbar::BrowserAgentStatusGuard::start(app, target, &action_label);
    let url_before = crate::webview_toolbar::browser_get_url(app.clone(), target.to_string())
        .await
        .unwrap_or_default();
    let action_result =
        crate::webview_toolbar::run_browser_action(app, target, &action, timeout_ms).await;
    let result = action_result?;
    if let Some(message) = browser_action_failed_message(&result, fallback_error) {
        return Err(message);
    }
    if settle_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(settle_ms)).await;
    }
    let current_url = crate::webview_toolbar::browser_get_url(app.clone(), target.to_string())
        .await
        .unwrap_or_default();
    // If the action navigated (e.g. clicking a product opened its detail page),
    // give the destination extra time to load before reading, so the returned
    // observation reflects the new page instead of a half-loaded or stale one.
    if !current_url.is_empty() && !url_before.is_empty() && current_url != url_before {
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    }
    let mut out = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other);
            map
        }
    };
    out.insert("target".into(), Value::String(target.to_string()));
    if !current_url.is_empty() {
        out.insert("current_url".into(), Value::String(current_url));
    }
    if let Ok(payload) = crate::webview_toolbar::extract_page_text(app, target).await {
        out.insert("observation".into(), compact_browser_observation(&payload));
    }
    insert_browser_window_snapshot(app, &mut out);
    status_guard.finish();
    Ok(Value::Object(out))
}

pub(in crate::agent_tools) fn insert_browser_window_snapshot(
    app: &tauri::AppHandle,
    out: &mut serde_json::Map<String, Value>,
) {
    let windows = crate::webview_toolbar::list_browser_windows(app)
        .into_iter()
        .map(|w| {
            json!({
                "label": w.label,
                "target": w.target,
                "url": w.url,
                "title": w.title,
                "type": w.kind,
            })
        })
        .collect::<Vec<_>>();
    if !windows.is_empty() {
        out.insert("browser_windows".into(), Value::Array(windows));
    }
}
