//! Tauri commands for browser navigation and reports.

use super::*;

// ============ Browser Control Commands ============

#[tauri::command]
pub async fn browser_go_back(app: tauri::AppHandle, target: String) -> Result<(), String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_go_forward(app: tauri::AppHandle, target: String) -> Result<(), String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_reload(app: tauri::AppHandle, target: String) -> Result<(), String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.eval("location.reload()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_get_url(app: tauri::AppHandle, target: String) -> Result<String, String> {
    if app.get_webview(&target).is_none() {
        return Err("Webview not found".into());
    }
    Ok(readable_url(&target))
}

#[tauri::command]
pub async fn browser_navigate(
    app: tauri::AppHandle,
    target: String,
    url: String,
) -> Result<(), String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("URL parse error: {}", e))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.navigate(parsed.clone()).map_err(|e| e.to_string())?;
    set_readable_url(&target, parsed.as_str());
    Ok(())
}

/// Close the browser window that owns `target` (which may be either the window
/// label, the `-ct` content webview label, or the `-tb` toolbar webview label).
/// Removes the label from the registry so subsequent `list_browser_windows`
/// calls reflect reality even before Tauri finishes destroying the window.
pub async fn browser_close(app: tauri::AppHandle, target: String) -> Result<String, String> {
    let label = browser_window_label_from_target(&target);
    let window = app
        .get_window(&label)
        .ok_or_else(|| format!("ウィンドウが見つかりません: {}", label))?;
    unregister_readable_label(&label);
    window
        .close()
        .map_err(|e| format!("ウィンドウを閉じられませんでした: {}", e))?;
    Ok(label)
}

#[tauri::command]
pub async fn browser_report_page_text(report: BrowserPageTextReport) -> Result<(), String> {
    let tx = PAGE_TEXT_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&report.request_id)
        .ok_or_else(|| "No pending browser text request".to_string())?;
    let _ = tx.send(report.payload);
    Ok(())
}

#[tauri::command]
pub async fn browser_report_action_result(report: BrowserActionReport) -> Result<(), String> {
    let tx = BROWSER_ACTION_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&report.request_id)
        .ok_or_else(|| "No pending browser action request".to_string())?;
    let _ = tx.send(report.payload);
    Ok(())
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_browser_mouse_selftest_report(
    report: BrowserMouseSelftestReport,
) -> Result<(), String> {
    if report.count == 0 {
        eprintln!("SELAH_BROWSER_MOUSE_SELFTEST_READY {}", report.href);
        return Ok(());
    }
    let tx = BROWSER_MOUSE_SELFTEST_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&report.request_id)
        .ok_or_else(|| "No pending browser mouse selftest request".to_string())?;
    let _ = tx.send(report);
    Ok(())
}

#[cfg(not(debug_assertions))]
#[tauri::command]
pub async fn debug_browser_mouse_selftest_report() -> Result<(), String> {
    Err("debug commands are not available in release builds".into())
}
