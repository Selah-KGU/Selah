use serde_json::Value;

use super::support::{
    compact_browser_observation, insert_browser_window_snapshot, resolve_browser_target_from_args,
    run_browser_action_tool,
};

pub async fn browser_click(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let mut action = serde_json::Map::new();
    action.insert("kind".into(), Value::String("click".into()));
    if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
        action.insert("selector".into(), Value::String(selector.to_string()));
    }
    if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
        action.insert("text".into(), Value::String(text.to_string()));
    }
    if let Some(href_contains) = args.get("href_contains").and_then(|v| v.as_str()) {
        action.insert(
            "hrefContains".into(),
            Value::String(href_contains.to_string()),
        );
    }
    if let Some(index) = args.get("index").and_then(|v| v.as_u64()) {
        action.insert("index".into(), Value::Number(index.into()));
    }
    run_browser_action_tool(
        app,
        &target,
        Value::Object(action),
        4_000,
        450,
        "ページ内のクリック対象が見つかりません",
    )
    .await
}

pub async fn browser_fill(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let mut action = serde_json::Map::new();
    action.insert("kind".into(), Value::String("fill".into()));
    if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
        action.insert("selector".into(), Value::String(selector.to_string()));
    }
    if let Some(label) = args.get("label").and_then(|v| v.as_str()) {
        action.insert("label".into(), Value::String(label.to_string()));
    }
    if let Some(value) = args.get("value").and_then(|v| v.as_str()) {
        action.insert("value".into(), Value::String(value.to_string()));
    }
    if let Some(index) = args.get("index").and_then(|v| v.as_u64()) {
        action.insert("index".into(), Value::Number(index.into()));
    }
    run_browser_action_tool(
        app,
        &target,
        Value::Object(action),
        4_000,
        120,
        "ページ内の入力欄が見つかりません",
    )
    .await
}

pub async fn browser_select_option(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let mut action = serde_json::Map::new();
    action.insert("kind".into(), Value::String("select_option".into()));
    if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
        action.insert("selector".into(), Value::String(selector.to_string()));
    }
    if let Some(label) = args.get("label").and_then(|v| v.as_str()) {
        action.insert("label".into(), Value::String(label.to_string()));
    }
    if let Some(value) = args.get("value").and_then(|v| v.as_str()) {
        action.insert("value".into(), Value::String(value.to_string()));
    }
    if let Some(index) = args.get("index").and_then(|v| v.as_u64()) {
        action.insert("index".into(), Value::Number(index.into()));
    }
    run_browser_action_tool(
        app,
        &target,
        Value::Object(action),
        4_000,
        120,
        "ページ内の選択欄が見つかりません",
    )
    .await
}

pub async fn browser_press(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let mut action = serde_json::Map::new();
    action.insert("kind".into(), Value::String("press".into()));
    if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
        action.insert("selector".into(), Value::String(selector.to_string()));
    }
    if let Some(key) = args.get("key").and_then(|v| v.as_str()) {
        action.insert("key".into(), Value::String(key.to_string()));
    }
    run_browser_action_tool(
        app,
        &target,
        Value::Object(action),
        4_000,
        300,
        "ページへキー入力を送れませんでした",
    )
    .await
}

pub async fn browser_scroll(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let mut action = serde_json::Map::new();
    action.insert("kind".into(), Value::String("scroll".into()));
    if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
        action.insert("selector".into(), Value::String(selector.to_string()));
    }
    if let Some(direction) = args.get("direction").and_then(|v| v.as_str()) {
        action.insert("direction".into(), Value::String(direction.to_string()));
    }
    if let Some(amount) = args.get("amount").and_then(|v| v.as_u64()) {
        action.insert("amount".into(), Value::Number(amount.into()));
    }
    run_browser_action_tool(
        app,
        &target,
        Value::Object(action),
        3_500,
        120,
        "ページをスクロールできませんでした",
    )
    .await
}

pub async fn browser_mouse_click(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let status_guard =
        crate::webview_toolbar::BrowserAgentStatusGuard::start(app, &target, "mouse_click");
    let result =
        crate::computer_control::mouse_click(app, Some(&target), x, y, Some("webview")).await?;
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    let current_url = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    let mut out = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other);
            map
        }
    };
    out.insert("target".into(), Value::String(target.clone()));
    out.insert("coordinate_space".into(), Value::String("webview".into()));
    if !current_url.is_empty() {
        out.insert("current_url".into(), Value::String(current_url));
    }
    if let Ok(payload) = crate::webview_toolbar::extract_page_text(app, &target).await {
        out.insert("observation".into(), compact_browser_observation(&payload));
    }
    insert_browser_window_snapshot(app, &mut out);
    status_guard.finish();
    Ok(Value::Object(out))
}

pub async fn browser_mouse_drag(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let from_x = args.get("from_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let from_y = args.get("from_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let to_x = args.get("to_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let to_y = args.get("to_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(8);
    let status_guard =
        crate::webview_toolbar::BrowserAgentStatusGuard::start(app, &target, "mouse_drag");
    let result = crate::computer_control::mouse_drag(
        app,
        Some(&target),
        from_x,
        from_y,
        to_x,
        to_y,
        steps,
        Some("webview"),
    )
    .await?;
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    let mut out = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other);
            map
        }
    };
    out.insert("target".into(), Value::String(target.clone()));
    out.insert("coordinate_space".into(), Value::String("webview".into()));
    if let Ok(payload) = crate::webview_toolbar::extract_page_text(app, &target).await {
        out.insert("observation".into(), compact_browser_observation(&payload));
    }
    insert_browser_window_snapshot(app, &mut out);
    status_guard.finish();
    Ok(Value::Object(out))
}

pub async fn browser_wait_for(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let mut action = serde_json::Map::new();
    action.insert("kind".into(), Value::String("wait_for".into()));
    if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
        action.insert("selector".into(), Value::String(selector.to_string()));
    }
    if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
        action.insert("text".into(), Value::String(text.to_string()));
    }
    if let Some(timeout_ms) = args.get("timeout_ms").and_then(|v| v.as_u64()) {
        action.insert("timeoutMs".into(), Value::Number(timeout_ms.into()));
    }
    run_browser_action_tool(
        app,
        &target,
        Value::Object(action),
        args.get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(3_000)
            + 700,
        80,
        "等待页面变化超时了",
    )
    .await
}
