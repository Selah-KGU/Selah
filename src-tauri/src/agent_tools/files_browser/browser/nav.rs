use serde_json::{json, Value};
use tauri::Manager;

use super::support::resolve_browser_target_from_args;

pub async fn browser_close_tool(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let label = crate::webview_toolbar::browser_close(app.clone(), target.clone()).await?;
    Ok(json!({
        "status": "closed",
        "label": label,
        "target": target,
    }))
}

pub async fn list_browser_windows(app: &tauri::AppHandle) -> Result<Value, String> {
    let items = crate::webview_toolbar::list_browser_windows(app);
    Ok(json!({
        "windows": items.into_iter().map(|w| json!({
            "label": w.label,
            "target": w.target,
            "url": w.url,
            "title": w.title,
            "type": w.kind,
        })).collect::<Vec<_>>()
    }))
}

pub async fn open_browser_url(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if url.is_empty() {
        return Err("urlを指定してください".into());
    }
    // If this call is what opens the Copilot window (e.g. the main-window agent
    // opening a URL), reveal the sidebar agent afterwards for a continuous chat.
    let copilot_window_existed = app.get_window("document-tabs").is_some();
    let info = crate::commands::open_external_url(app.clone(), url.clone(), None).await?;
    if !copilot_window_existed {
        let _ = crate::document_tabs::open_agent_workspace(app);
    }
    Ok(json!({
        "status": "opened",
        "label": info.label,
        "target": info.target,
        "url": if info.url.is_empty() { url } else { info.url },
    }))
}

async fn wait_for_readable_url_change(
    app: &tauri::AppHandle,
    target: &str,
    previous: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    if app.get_webview(target).is_none() {
        return Err("Webview not found".into());
    }
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let current = crate::webview_toolbar::browser_get_url(app.clone(), target.to_string())
            .await
            .unwrap_or_default();
        if !current.is_empty() && current != previous {
            return Ok(current);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(current);
        }
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
}

pub async fn browser_back(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let previous = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    crate::webview_toolbar::browser_go_back(app.clone(), target.clone()).await?;
    let url = wait_for_readable_url_change(
        app,
        &target,
        &previous,
        std::time::Duration::from_millis(800),
    )
    .await
    .unwrap_or(previous);
    Ok(json!({ "target": target, "status": "ok", "url": url }))
}

pub async fn browser_forward(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let previous = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    crate::webview_toolbar::browser_go_forward(app.clone(), target.clone()).await?;
    let url = wait_for_readable_url_change(
        app,
        &target,
        &previous,
        std::time::Duration::from_millis(800),
    )
    .await
    .unwrap_or(previous);
    Ok(json!({ "target": target, "status": "ok", "url": url }))
}

pub async fn browser_reload_page(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    crate::webview_toolbar::browser_reload(app.clone(), target.clone()).await?;
    let url = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    Ok(json!({ "target": target, "status": "ok", "url": url }))
}
