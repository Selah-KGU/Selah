//! Agent side panel for the document-tabs window.

use super::*;

pub(super) fn emit_agent_visibility(app: &tauri::AppHandle, open: bool) {
    let _ = app.emit_to(
        tauri::EventTarget::AnyLabel {
            label: TAB_STRIP_LABEL.to_string(),
        },
        "document-tabs-agent-visibility",
        serde_json::json!({
            "owner": OWNER_LABEL,
            "open": open,
        }),
    );
}

pub fn emit_agent_status(app: &tauri::AppHandle, target: &str, active: bool, action: &str) {
    if !AGENT_PANEL_OPEN.load(Ordering::Relaxed) {
        return;
    }
    let _ = app.emit_to(
        tauri::EventTarget::AnyLabel {
            label: AGENT_PANEL_LABEL.to_string(),
        },
        "browser-agent-status",
        serde_json::json!({
            "target": target,
            "active": active,
            "action": action,
        }),
    );
}

pub(super) fn agent_panel_width(width: f64) -> f64 {
    if AGENT_PANEL_OPEN.load(Ordering::Relaxed) {
        width * agent_panel_ratio()
    } else {
        0.0
    }
}

pub(super) fn agent_panel_ratio() -> f64 {
    (AGENT_PANEL_RATIO_BPS.load(Ordering::Relaxed) as f64 / 10_000.0)
        .clamp(MIN_AGENT_PANEL_RATIO, MAX_AGENT_PANEL_RATIO)
}

pub(super) fn set_agent_panel_ratio(ratio: f64) -> f64 {
    let clamped = ratio.clamp(MIN_AGENT_PANEL_RATIO, MAX_AGENT_PANEL_RATIO);
    AGENT_PANEL_RATIO_BPS.store((clamped * 10_000.0).round() as u32, Ordering::Relaxed);
    clamped
}

pub(super) fn agent_panel_url(tab: Option<&DocumentTab>) -> String {
    let (target, title, kind) = tab
        .map(|tab| (tab.target.as_str(), tab.title.as_str(), tab.kind.as_str()))
        .unwrap_or((OWNER_LABEL, "エージェント", "agent"));
    format!(
        "index.html#surface=agent-panel&owner={}&target={}&title={}&kind={}",
        urlencoding::encode(OWNER_LABEL),
        urlencoding::encode(target),
        urlencoding::encode(title),
        urlencoding::encode(kind),
    )
}

pub(super) fn open_agent_panel(app: &tauri::AppHandle) -> Result<(), String> {
    let window = ensure_window(app, OWNER_LABEL)?;
    let active = active_tab_for_owner(OWNER_LABEL);
    if app.get_webview(AGENT_PANEL_LABEL).is_none() {
        let panel = tauri::webview::WebviewBuilder::new(
            AGENT_PANEL_LABEL,
            tauri::WebviewUrl::App(agent_panel_url(active.as_ref()).into()),
        )
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .auto_resize();
        let size = window.inner_size().map_err(|e| e.to_string())?;
        let scale = window.scale_factor().unwrap_or(1.0);
        let width = size.width as f64 / scale;
        let height = size.height as f64 / scale;
        let panel_width = width * agent_panel_ratio();
        window
            .add_child(
                panel,
                tauri::Position::Logical(tauri::LogicalPosition::new(
                    (width - panel_width).max(260.0),
                    TAB_STRIP_HEIGHT,
                )),
                tauri::Size::Logical(tauri::LogicalSize::new(
                    panel_width,
                    (height - TAB_STRIP_HEIGHT).max(120.0),
                )),
            )
            .map_err(|e| format!("Agent パネル作成失敗: {}", e))?;
    }
    AGENT_PANEL_OPEN.store(true, Ordering::Relaxed);
    resize_current_for_owner(app, OWNER_LABEL)?;
    emit_agent_visibility(app, true);
    emit_tabs_changed(app, OWNER_LABEL);
    Ok(())
}

pub(super) fn close_agent_panel(app: &tauri::AppHandle) -> Result<(), String> {
    AGENT_PANEL_OPEN.store(false, Ordering::Relaxed);
    if let Some(panel) = app.get_webview(AGENT_PANEL_LABEL) {
        let _ = panel.hide();
        let _ = panel.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            OFFSCREEN_X,
            TAB_STRIP_HEIGHT,
        )));
        let _ = panel.set_size(tauri::Size::Logical(tauri::LogicalSize::new(0.0, 0.0)));
    }
    let has_tabs = !list_tabs_for_owner(OWNER_LABEL).is_empty();
    if has_tabs {
        resize_current_for_owner(app, OWNER_LABEL)?;
    } else if let Some(window) = app.get_window(OWNER_LABEL) {
        force_close_window(&window);
        DOCUMENT_WINDOWS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(OWNER_LABEL);
    }
    emit_agent_visibility(app, false);
    Ok(())
}

pub fn open_agent_workspace(app: &tauri::AppHandle) -> Result<(), String> {
    let window = ensure_window(app, OWNER_LABEL)?;
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    open_agent_panel(app)
}
