use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tauri::{Emitter, Manager};

const TOOLBAR_HEIGHT: f64 = 38.0;
const BROWSER_BRIDGE_SCRIPT: &str = r#"
(function () {
  if (window.__selahBrowserBridgeInstalled) return;
  window.__selahBrowserBridgeInstalled = true;
  window.__selahBrowserExtractText = async function (requestId) {
    try {
      var invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
      if (!invoke) return;
      var doc = document;
      var title = (doc.title || '').trim();
      var bodyText = '';
      if (doc.body) {
        bodyText = (doc.body.innerText || doc.body.textContent || '').trim();
      }
      if (!bodyText && doc.documentElement) {
        bodyText = (doc.documentElement.innerText || doc.documentElement.textContent || '').trim();
      }
      bodyText = bodyText.replace(/\s+\n/g, '\n').replace(/\n{3,}/g, '\n\n');
      await invoke('browser_report_page_text', {
        report: {
          requestId: requestId,
          payload: {
            title: title,
            url: String(window.location.href || ''),
            text: bodyText
          }
        }
      });
    } catch (_) {}
  };
})();
"#;

static PAGE_TEXT_WAITERS: LazyLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<PageTextPayload>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_WINDOW_LABELS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct BrowserWindowInfo {
    pub label: String,
    pub target: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTextPayload {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPageTextReport {
    request_id: String,
    payload: PageTextPayload,
}

/// Create a browser-style window with a native toolbar webview + content webview.
/// The toolbar is a local HTML page with back/forward/reload/URL/open-in-browser.
/// The content webview loads the external URL.
pub fn create_browser_window(
    app: &tauri::AppHandle,
    label: &str,
    url: tauri::WebviewUrl,
    title: &str,
    width: f64,
    height: f64,
    init_scripts: &[&str],
) -> Result<(), String> {
    let toolbar_label = format!("{}-tb", label);
    let content_label = format!("{}-ct", label);

    let builder = tauri::window::WindowBuilder::new(app, label)
        .title(title)
        .inner_size(width, height)
        .resizable(true);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    let window = builder
        .build()
        .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;
    BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string());

    // --- Toolbar webview (local HTML) ---
    let toolbar_url = format!(
        "browser-toolbar.html?target={}",
        urlencoding::encode(&content_label)
    );
    let toolbar_builder = tauri::webview::WebviewBuilder::new(
        &toolbar_label,
        tauri::WebviewUrl::App(toolbar_url.into()),
    )
    .auto_resize();

    window
        .add_child(
            toolbar_builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, 0.0)),
            tauri::Size::Logical(tauri::LogicalSize::new(width, TOOLBAR_HEIGHT)),
        )
        .map_err(|e| format!("ツールバー作成失敗: {}", e))?;

    // --- Content webview ---
    let mut content_builder = tauri::webview::WebviewBuilder::new(&content_label, url)
        .initialization_script(BROWSER_BRIDGE_SCRIPT);
    for script in init_scripts {
        content_builder = content_builder.initialization_script(*script);
    }

    // Emit URL changes to the toolbar
    let app_for_event = app.clone();
    let tb_label_event = toolbar_label.clone();
    content_builder = content_builder.on_page_load(move |_webview, payload| {
        use tauri::webview::PageLoadEvent;
        if matches!(payload.event(), PageLoadEvent::Finished) {
            let url_str = payload.url().to_string();
            let _ = app_for_event.emit_to(
                tauri::EventTarget::AnyLabel {
                    label: tb_label_event.clone(),
                },
                "browser-url-changed",
                &url_str,
            );
        }
    });

    window
        .add_child(
            content_builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, TOOLBAR_HEIGHT)),
            tauri::Size::Logical(tauri::LogicalSize::new(width, height - TOOLBAR_HEIGHT)),
        )
        .map_err(|e| format!("コンテンツ作成失敗: {}", e))?;

    // --- Handle window resize ---
    let app_resize = app.clone();
    let tb_label_resize = toolbar_label;
    let ct_label_resize = content_label;
    let win_for_scale = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Resized(phys_size) = event {
            let scale = win_for_scale.scale_factor().unwrap_or(1.0);
            let w = phys_size.width as f64 / scale;
            let h = phys_size.height as f64 / scale;

            if let Some(tb) = app_resize.get_webview(&tb_label_resize) {
                let _ = tb.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                    w,
                    TOOLBAR_HEIGHT,
                )));
            }
            if let Some(ct) = app_resize.get_webview(&ct_label_resize) {
                let _ = ct.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                    w,
                    (h - TOOLBAR_HEIGHT).max(0.0),
                )));
            }
        }
    });

    Ok(())
}

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
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.url().map(|u| u.to_string()).map_err(|e| e.to_string())
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
            let target = format!("{}-ct", &label);
            let toolbar = format!("{}-tb", &label);
            if app.get_window(&label).is_none() {
                return None;
            }
            if app.get_webview(&target).is_none() || app.get_webview(&toolbar).is_none() {
                return None;
            }
            let url = app
                .get_webview(&target)
                .and_then(|wv| wv.url().ok())
                .map(|u| u.to_string())
                .unwrap_or_default();
            Some(BrowserWindowInfo { label, target, url })
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
