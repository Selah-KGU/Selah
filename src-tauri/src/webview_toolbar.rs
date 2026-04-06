use tauri::{Emitter, Manager};

const TOOLBAR_HEIGHT: f64 = 38.0;

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
    let mut content_builder = tauri::webview::WebviewBuilder::new(&content_label, url);
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
