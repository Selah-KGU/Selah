//! Browser window listing and page actions.

use super::*;

pub fn list_browser_windows(app: &tauri::AppHandle) -> Vec<BrowserWindowInfo> {
    let labels: Vec<String> = BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    let mut items: Vec<BrowserWindowInfo> = labels
        .into_iter()
        .filter_map(|label| {
            let target = BROWSER_WINDOW_TARGETS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&label)
                .cloned()
                .unwrap_or_else(|| format!("{}-ct", &label));
            let owner = BROWSER_WINDOW_OWNERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&label)
                .cloned()
                .unwrap_or_else(|| label.clone());
            app.get_window(&owner)?;
            app.get_webview(&target)?;
            let url = readable_url(&target);
            let title = BROWSER_WINDOW_TITLES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&label)
                .cloned()
                .unwrap_or_default();
            let kind = BROWSER_WINDOW_KINDS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&label)
                .cloned()
                .unwrap_or_else(|| {
                    if label.contains("detail") || target.contains("detail") {
                        "detail".to_string()
                    } else {
                        "browser".to_string()
                    }
                });
            Some(BrowserWindowInfo {
                label,
                target,
                url,
                title,
                kind,
            })
        })
        .collect();
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

pub fn resolve_browser_target(
    app: &tauri::AppHandle,
    requested: Option<&str>,
) -> Result<String, String> {
    if let Some(target) = requested {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err("browser target is empty".into());
        }
        if app.get_webview(trimmed).is_some() {
            return Ok(trimmed.to_string());
        }
        let content = format!("{}-ct", trimmed);
        if app.get_webview(&content).is_some() {
            return Ok(content);
        }
        return Err(format!("Browser target not found: {}", trimmed));
    }
    let items = list_browser_windows(app);
    match items.as_slice() {
        [] => Err("No browser window is open".into()),
        [only] => Ok(only.target.clone()),
        _ => Err("Multiple browser windows are open; list_browser_windows first".into()),
    }
}

pub async fn extract_page_text(
    app: &tauri::AppHandle,
    target: &str,
) -> Result<PageTextPayload, String> {
    let wv = app.get_webview(target).ok_or("Webview not found")?;

    for attempt in 0..5 {
        let request_id = format!("browser-text-{}", uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        PAGE_TEXT_WAITERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request_id.clone(), tx);

        let js = format!(
            "(function(){{ if (window.__selahBrowserExtractText) window.__selahBrowserExtractText({}); }})();",
            serde_json::to_string(&request_id).unwrap_or_else(|_| "\"\"".into())
        );

        if let Err(e) = wv.eval(&js) {
            PAGE_TEXT_WAITERS
                .lock()
                .unwrap_or_else(|pe| pe.into_inner())
                .remove(&request_id);
            return Err(e.to_string());
        }

        match tokio::time::timeout(std::time::Duration::from_millis(1200), rx).await {
            Ok(Ok(payload))
                if !payload.url.is_empty()
                    && payload.url != "about:blank"
                    && (!payload.text.trim().is_empty() || attempt >= 2) =>
            {
                return Ok(payload);
            }
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
                PAGE_TEXT_WAITERS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&request_id);
                if attempt < 4 {
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    continue;
                }
            }
        }
    }
    Err("Timed out while extracting page text".into())
}

pub async fn run_browser_action(
    app: &tauri::AppHandle,
    target: &str,
    action: &Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    let wv = app.get_webview(target).ok_or("Webview not found")?;
    let request_id = format!("browser-action-{}", uuid::Uuid::new_v4());
    let (tx, rx) = tokio::sync::oneshot::channel();
    BROWSER_ACTION_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), tx);

    let js = format!(
        "(function(){{ if (window.__selahBrowserRunAction) window.__selahBrowserRunAction({}, {}); else window.__TAURI__?.core?.invoke?.('browser_report_action_result', {{ report: {{ requestId: {}, payload: {{ ok: false, error: 'Browser action bridge unavailable' }} }} }}); }})();",
        serde_json::to_string(&request_id).unwrap_or_else(|_| "\"\"".into()),
        serde_json::to_string(action).unwrap_or_else(|_| "{}".into()),
        serde_json::to_string(&request_id).unwrap_or_else(|_| "\"\"".into()),
    );

    if let Err(e) = wv.eval(&js) {
        BROWSER_ACTION_WAITERS
            .lock()
            .unwrap_or_else(|pe| pe.into_inner())
            .remove(&request_id);
        return Err(e.to_string());
    }

    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms.max(300)), rx).await {
        Ok(Ok(payload)) => Ok(payload),
        Ok(Err(_)) => Err("Browser action channel closed".into()),
        Err(_) => {
            BROWSER_ACTION_WAITERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&request_id);
            Err(format!(
                "Timed out while waiting for browser action after {} ms",
                timeout_ms.max(300)
            ))
        }
    }
}

#[cfg(debug_assertions)]
pub async fn debug_browser_mouse_click_selftest(app: tauri::AppHandle) -> Result<Value, String> {
    let owner_label = "ext-browser-mouse-selftest";
    if let Some(window) = app.get_window(owner_label) {
        let _ = window.close();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    let request_id = format!("browser-mouse-selftest-{}", uuid::Uuid::new_v4());
    let (tx, rx) = tokio::sync::oneshot::channel();
    BROWSER_MOUSE_SELFTEST_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), tx);
    let page_url = format!(
        "index.html?surface=browser-mouse-selftest&request={}",
        urlencoding::encode(&request_id)
    );

    let info = create_browser_window(
        &app,
        owner_label,
        tauri::WebviewUrl::App(page_url.into()),
        "Browser Mouse Selftest",
        760.0,
        520.0,
        &[],
    )?;

    for (label, window) in app.windows() {
        if label != owner_label {
            let _ = window.hide();
        }
    }
    if let Some(window) = app.get_window(owner_label) {
        let _ = window.set_always_on_top(true);
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            80, 80,
        )));
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    if let Some(webview) = app.get_webview(&info.target) {
        let _ = webview.set_focus();
    }

    tokio::time::sleep(std::time::Duration::from_millis(1600)).await;
    for (label, window) in app.windows() {
        if label != owner_label {
            let _ = window.hide();
        }
    }
    if let Some(window) = app.get_window(owner_label) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(webview) = app.get_webview(&info.target) {
        let _ = webview.set_focus();
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let click_x = 230.0;
    let click_y = 136.0;

    let click_result = crate::computer_control::mouse_click(
        &app,
        Some(&info.target),
        click_x,
        click_y,
        Some("webview"),
    )
    .await?;
    let click_result_for_error = click_result.clone();

    let report = match tokio::time::timeout(std::time::Duration::from_millis(4_000), rx).await {
        Ok(Ok(report)) => report,
        Ok(Err(_)) => return Err("browser mouse selftest report channel closed".into()),
        Err(_) => {
            BROWSER_MOUSE_SELFTEST_WAITERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&request_id);
            let screenshot = write_selftest_screenshot(&app, &info.target).await;
            return Err(format!(
                "mouse click did not reach WebView button; clicked at viewport ({:.0}, {:.0}); click result: {}; screenshot: {}",
                click_x,
                click_y,
                click_result_for_error,
                screenshot.unwrap_or_else(|| "unavailable".into())
            ));
        }
    };
    if report.count == 0 {
        return Err("browser mouse selftest report returned count=0".into());
    }

    if let Some(window) = app.get_window(owner_label) {
        let _ = window.close();
    }

    Ok(serde_json::json!({
        "status": "passed",
        "label": info.label,
        "target": info.target,
        "button": {
            "text": "Click target",
            "rect": {
                "x": 120,
                "y": 112,
                "width": 220,
                "height": 48,
                "centerX": click_x,
                "centerY": click_y,
            },
        },
        "click": click_result,
        "report": report,
    }))
}

#[cfg(debug_assertions)]
pub(super) async fn write_selftest_screenshot(
    app: &tauri::AppHandle,
    target: &str,
) -> Option<String> {
    use base64::Engine;

    let value = crate::computer_control::screenshot(app, Some(target))
        .await
        .ok()?;
    let data = value
        .get("image")
        .and_then(|image| image.get("data_base64"))
        .and_then(|data| data.as_str())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    let rect = value.get("screen_rect").cloned().unwrap_or(Value::Null);
    let path = std::env::temp_dir().join(format!(
        "selah-browser-mouse-selftest-{}.png",
        std::process::id()
    ));
    std::fs::write(&path, bytes).ok()?;
    Some(format!("{} screen_rect={}", path.display(), rect))
}
