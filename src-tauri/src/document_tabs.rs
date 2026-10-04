use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::{Emitter, Manager};

#[path = "document_tabs/agent_panel.rs"]
mod agent_panel;
#[path = "document_tabs/commands.rs"]
mod commands;
#[path = "document_tabs/layout.rs"]
mod layout;
#[path = "document_tabs/open.rs"]
mod open;

pub(in crate::document_tabs) use agent_panel::*;
pub use agent_panel::{emit_agent_status, open_agent_workspace};
pub use commands::*;
pub use layout::resize_current_for_owner;
pub(in crate::document_tabs) use layout::*;
pub use open::*;

const OWNER_LABEL: &str = "document-tabs";

// Injected into browser tabs so links that target a new window (or window.open)
// reliably open as a managed tab instead of being silently dropped by WKWebView.
const BROWSER_LINK_HANDLER_SCRIPT: &str = r#"(function () {
  function invoke() {
    var t = window.__TAURI__;
    return (t && t.core && t.core.invoke) || (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) || null;
  }
  function openTab(url) {
    var inv = invoke();
    if (!inv || !/^https?:\/\//i.test(String(url || ''))) return false;
    inv('open_external_url', { url: String(url) }).catch(function () {});
    return true;
  }
  document.addEventListener('click', function (e) {
    try {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
      if (!a) return;
      var target = (a.getAttribute('target') || '').toLowerCase();
      if (target !== '_blank') return;
      if (openTab(a.href)) {
        e.preventDefault();
        e.stopPropagation();
      }
    } catch (_) {}
  }, true);
  try {
    var nativeOpen = window.open;
    window.open = function (url) {
      if (openTab(url)) return null;
      return nativeOpen ? nativeOpen.apply(window, arguments) : null;
    };
  } catch (_) {}
})();"#;
const TAB_STRIP_LABEL: &str = "document-tabs-strip";
const AGENT_PANEL_LABEL: &str = "document-tabs-agent";
const TAB_STRIP_HEIGHT: f64 = 88.0;
/// Detective is a full-screen game with no per-tab controls, so its strip
/// collapses to just the tab row (no second toolbar row).
const COMPACT_STRIP_HEIGHT: f64 = 46.0;
const DEFAULT_AGENT_PANEL_RATIO_BPS: u32 = 3_333;
const MIN_AGENT_PANEL_RATIO: f64 = 0.2;
const MAX_AGENT_PANEL_RATIO: f64 = 0.4;
const SPLIT_DIVIDER_WIDTH: f64 = 12.0;
const MAX_SPLIT_DIVIDERS: usize = 2;
const MIN_SPLIT_RATIO: f64 = 0.2;
const DEFAULT_WIDTH: f64 = 1080.0;
const DEFAULT_HEIGHT: f64 = 760.0;
const OFFSCREEN_X: f64 = 50_000.0;
const MAX_TABS: usize = 32;

static TAB_COUNTER: AtomicU32 = AtomicU32::new(0);
static AGENT_PANEL_OPEN: AtomicBool = AtomicBool::new(false);
static AGENT_PANEL_RATIO_BPS: AtomicU32 = AtomicU32::new(DEFAULT_AGENT_PANEL_RATIO_BPS);
// Set right before a programmatic window.close() (e.g. last tab closed) so the
// CloseRequested guard lets it through; otherwise the red close button only hides
// the window, preserving open tabs for the next time it is shown.
static FORCE_CLOSE: AtomicBool = AtomicBool::new(false);
static DOCUMENT_WINDOWS: LazyLock<Mutex<HashMap<String, DocumentWindowState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
// Event-loop thread id, recorded at setup. Webview creation must be deferred to
// a background thread when running on it (see note on ensure_window).
static MAIN_THREAD_ID: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

/// Record the event-loop (main) thread. Called once from setup.
pub fn record_main_thread() {
    let _ = MAIN_THREAD_ID.set(std::thread::current().id());
}

fn on_main_thread() -> bool {
    MAIN_THREAD_ID.get() == Some(&std::thread::current().id())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTabInfo {
    pub id: String,
    pub label: String,
    pub target: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub active: bool,
    pub loading: bool,
    pub controls: Vec<DocumentTabControl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split_ratios: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reopen: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTabControl {
    pub id: String,
    pub label: String,
    pub action: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    #[serde(default)]
    pub indicator: bool,
    #[serde(default)]
    pub indicator_on: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTabProbeReport {
    pub owner: String,
    pub id: String,
    pub label: String,
    pub target: String,
    pub title: String,
    pub url: String,
    pub kind: String,
    pub href: String,
    pub ready_state: String,
    pub visibility_state: String,
    pub has_content: bool,
    pub content_children: i64,
    pub content_text_length: i64,
    pub content_html_length: i64,
    pub has_loading: bool,
    pub has_error: bool,
    pub has_detail_wrap: bool,
    pub has_page_title: bool,
    pub viewport_width: i64,
    pub viewport_height: i64,
    pub body_width: i64,
    pub body_height: i64,
    pub preview: String,
}

#[derive(Debug, Clone)]
struct ChildPane {
    target: String,
    label: String,
    /// Whether this pane shows a discussion board (掲示板). Only a board pane may
    /// spawn a third (grandchild) pane when you drill into a thread from it.
    is_board: bool,
    child: Option<Box<ChildPane>>,
}

#[derive(Debug, Clone)]
struct DocumentTab {
    id: String,
    label: String,
    target: String,
    key: Option<String>,
    title: String,
    url: String,
    kind: String,
    controls: Vec<DocumentTabControl>,
    reopen: Option<serde_json::Value>,
    loading: bool,
    child: Option<ChildPane>,
    split_weights: Vec<f64>,
    active_split_drag: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct DocumentWindowState {
    tabs: Vec<DocumentTab>,
    active: Option<String>,
}

impl DocumentTab {
    fn info(&self, active: bool) -> DocumentTabInfo {
        DocumentTabInfo {
            id: self.id.clone(),
            label: self.label.clone(),
            target: self.target.clone(),
            title: self.title.clone(),
            url: self.url.clone(),
            kind: self.kind.clone(),
            active,
            loading: self.loading,
            controls: self.controls.clone(),
            split_ratios: split_ratios_for_layout(&self.child, &self.split_weights),
            reopen: self.reopen.clone(),
        }
    }
}

fn tab_strip_url(owner: &str) -> String {
    format!(
        "index.html#surface=document-tabs&owner={}",
        urlencoding::encode(owner)
    )
}

fn ensure_tab_strip(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    owner: &str,
) -> Result<(), String> {
    if app.get_webview(TAB_STRIP_LABEL).is_some() {
        return Ok(());
    }
    let strip = tauri::webview::WebviewBuilder::new(
        TAB_STRIP_LABEL,
        tauri::WebviewUrl::App(tab_strip_url(owner).into()),
    )
    .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
    .auto_resize();
    window
        .add_child(
            strip,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, 0.0)),
            tauri::Size::Logical(tauri::LogicalSize::new(DEFAULT_WIDTH, TAB_STRIP_HEIGHT)),
        )
        .map(|_| ())
        .map_err(|e| format!("タブバー作成失敗: {}", e))
}

fn emit_tabs_changed(app: &tauri::AppHandle, owner: &str) {
    let tabs = list_tabs_for_owner(owner);
    let payload = serde_json::json!({
        "owner": owner,
        "tabs": tabs,
    });
    let mut labels = vec![TAB_STRIP_LABEL.to_string(), AGENT_PANEL_LABEL.to_string()];
    for tab in list_tabs_for_owner(owner) {
        for index in 0..MAX_SPLIT_DIVIDERS {
            labels.push(split_divider_target(&tab.target, index));
        }
    }
    for label in labels {
        let _ = app.emit_to(
            tauri::EventTarget::AnyLabel { label },
            "document-tabs-changed",
            payload.clone(),
        );
    }
}

fn list_tabs_for_owner(owner: &str) -> Vec<DocumentTabInfo> {
    let state = DOCUMENT_WINDOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(owner)
        .cloned()
        .unwrap_or_default();
    state
        .tabs
        .iter()
        .map(|tab| tab.info(state.active.as_deref() == Some(tab.id.as_str())))
        .collect()
}

fn active_tab_for_owner(owner: &str) -> Option<DocumentTab> {
    let state = DOCUMENT_WINDOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(owner)
        .cloned()?;
    let active = state.active.as_deref()?;
    state.tabs.iter().find(|tab| tab.id == active).cloned()
}

/// Targets of every **live** pane that makes up the owner's current view: the
/// active tab's main webview plus its split child panes (pane2, pane3, …), in
/// left-to-right order. One element when there is no split. Used so the agent
/// can read/operate on any pane of the current split view, not just the active
/// one.
pub fn active_view_panes(app: &tauri::AppHandle, owner: &str) -> Vec<String> {
    let Some(tab) = active_tab_for_owner(owner) else {
        return Vec::new();
    };
    std::iter::once(tab.target.clone())
        .chain(
            pane_chain(&tab.child)
                .into_iter()
                .map(|(target, _label)| target),
        )
        .filter(|target| app.get_webview(target).is_some())
        .collect()
}

// Creating child webviews (`Window::add_child`) must never run on the event-loop
// (main) thread on Windows: the creation is posted back to the event loop and the
// caller blocks until it completes, so a synchronous #[tauri::command] — which
// executes on that very thread — deadlocks the entire app. Every command that can
// reach ensure_window / open_tab / open_agent_panel is therefore `async` (async
// commands run on the tokio pool), and main-thread callbacks defer creation to a
// background thread instead.
fn ensure_window(app: &tauri::AppHandle, owner: &str) -> Result<tauri::Window, String> {
    if let Some(window) = app.get_window(owner) {
        if AGENT_PANEL_OPEN.load(Ordering::Relaxed) && app.get_webview(AGENT_PANEL_LABEL).is_none()
        {
            AGENT_PANEL_OPEN.store(false, Ordering::Relaxed);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        ensure_tab_strip(app, &window, owner)?;
        let _ = resize_current_for_owner(app, owner);
        return Ok(window);
    }

    AGENT_PANEL_OPEN.store(false, Ordering::Relaxed);
    let builder = tauri::window::WindowBuilder::new(app, owner)
        .title("Copilot")
        .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
        .min_inner_size(640.0, 420.0)
        .resizable(true);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    // On Windows we drop the native title bar and draw our own min/max/close in
    // the tab strip, matching the main window (see lib.rs set_decorations(false)).
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false);

    let window = builder
        .build()
        .map_err(|e| format!("タブウィンドウ作成失敗: {}", e))?;

    // Open as an independent window: cascade off the main window so it never
    // spawns exactly stacked on top of it. Falls back to screen-center.
    if let Some(main) = app.get_webview_window("main") {
        if let Ok(pos) = main.outer_position() {
            // 48 logical px cascade, kept constant across DPI.
            let offset = (48.0 * main.scale_factor().unwrap_or(1.0)).round() as i32;
            let _ =
                window.set_position(tauri::PhysicalPosition::new(pos.x + offset, pos.y + offset));
        } else {
            let _ = window.center();
        }
    } else {
        let _ = window.center();
    }

    ensure_tab_strip(app, &window, owner)?;

    DOCUMENT_WINDOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(owner.to_string())
        .or_default();

    let app_resize = app.clone();
    let owner_resize = owner.to_string();
    let win_for_scale = window.clone();
    window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::Resized(phys_size) => {
                let scale = win_for_scale.scale_factor().unwrap_or(1.0);
                resize_document_window(
                    &app_resize,
                    &owner_resize,
                    phys_size.width as f64 / scale,
                    phys_size.height as f64 / scale,
                );
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                // User hit the close button: keep the tabs alive by hiding instead
                // of destroying. A programmatic close (last tab) sets FORCE_CLOSE.
                if !FORCE_CLOSE.swap(false, Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = win_for_scale.hide();
                }
            }
            _ => {}
        }
    });

    Ok(window)
}

/// Truly close (destroy) the document-tabs window, bypassing the hide-on-close
/// guard. Used when there are no tabs left to preserve.
fn force_close_window(window: &tauri::Window) {
    FORCE_CLOSE.store(true, Ordering::Relaxed);
    let _ = window.close();
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn activation_probe_script(owner: &str, tab: &DocumentTab) -> String {
    format!(
        r#"(function () {{
  try {{
    window.dispatchEvent(new Event('resize'));
    if (typeof window.__selahDetailActivate === 'function') {{
      window.__selahDetailActivate();
    }} else {{
      window.dispatchEvent(new CustomEvent('selah-tab-activated'));
    }}
  }} catch (_) {{}}
  try {{
    setTimeout(function () {{
      try {{
    var content = document.getElementById('content');
    var body = document.body;
    var source = content || body;
    var text = source ? String(source.textContent || '') : '';
    var html = content ? String(content.innerHTML || '') : '';
    var rect = body ? body.getBoundingClientRect() : null;
    var invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
    if (!invoke) return;
    invoke('document_tabs_report_probe', {{
      report: {{
        owner: {owner},
        id: {id},
        label: {label},
        target: {target},
        title: {title},
        url: {url},
        kind: {kind},
        href: String(window.location && window.location.href || ''),
        readyState: String(document.readyState || ''),
        visibilityState: String(document.visibilityState || ''),
        hasContent: !!content,
        contentChildren: content ? content.children.length : -1,
        contentTextLength: text.replace(/\s+/g, '').length,
        contentHtmlLength: html.length,
        hasLoading: !!(content && content.querySelector('.loading')),
        hasError: !!(content && content.querySelector('.error')),
        hasDetailWrap: !!(content && content.querySelector('.detail-wrap')),
        hasPageTitle: !!(content && content.querySelector('.page-title')),
        viewportWidth: Math.round(window.innerWidth || 0),
        viewportHeight: Math.round(window.innerHeight || 0),
        bodyWidth: rect ? Math.round(rect.width) : 0,
        bodyHeight: rect ? Math.round(rect.height) : 0,
        preview: text.replace(/\s+/g, ' ').trim().slice(0, 180)
      }}
    }}).catch(function () {{}});
      }} catch (_) {{}}
    }}, 900);
  }} catch (_) {{}}
}})();"#,
        owner = js_string(owner),
        id = js_string(&tab.id),
        label = js_string(&tab.label),
        target = js_string(&tab.target),
        title = js_string(&tab.title),
        url = js_string(&tab.url),
        kind = js_string(&tab.kind),
    )
}

fn title_report_script(target: &str) -> String {
    format!(
        r#"(function () {{
  function send() {{
    try {{
      var title = String(document.title || '').trim();
      var invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
      if (invoke && title) {{
        invoke('document_tabs_report_title', {{ target: {target}, title: title }}).catch(function () {{}});
      }}
    }} catch (_) {{}}
  }}
  send();
  // Many sites set <title> via JS just after load; re-read shortly after.
  setTimeout(send, 700);
}})();"#,
        target = js_string(target),
    )
}

fn notify_tab_activated(app: &tauri::AppHandle, owner: &str, tab: &DocumentTab) {
    if let Some(webview) = app.get_webview(&tab.target) {
        // Make the freshly activated content webview the key responder so clicks
        // and keyboard input land on the page, not the (now hidden) old tab.
        let _ = webview.set_focus();
        if let Err(err) = webview.eval(activation_probe_script(owner, tab)) {
            log::warn!(
                "document tab activation probe failed owner={} target={} kind={} err={}",
                owner,
                tab.target,
                tab.kind,
                err
            );
        }
    }
}

/// Cheap read of just the active tab's kind (no full-state clone), so the strip
/// can be resized on every drag tick without waiting on a heavy clone.
fn active_kind_for_owner(owner: &str) -> Option<String> {
    DOCUMENT_WINDOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(owner)
        .and_then(|state| {
            let active = state.active.as_deref()?;
            state
                .tabs
                .iter()
                .find(|tab| tab.id == active)
                .map(|tab| tab.kind.clone())
        })
}

fn activate_tab_inner(app: &tauri::AppHandle, owner: &str, tab_id: &str) -> Result<(), String> {
    let active_tab = {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let tab = state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id || tab.target == tab_id || tab.label == tab_id)
            .cloned()
            .ok_or_else(|| format!("タブが見つかりません: {}", tab_id))?;
        state.active = Some(tab.id.clone());
        tab
    };

    crate::webview_toolbar::set_owner_active_target(
        owner,
        &active_tab.target,
        &active_tab.title,
        &active_tab.kind,
    );
    if let Some(window) = app.get_window(owner) {
        let _ = window.set_title(&active_tab.title);
        let _ = window.set_focus();
    }
    resize_current_for_owner(app, owner)?;
    notify_tab_activated(app, owner, &active_tab);
    emit_tabs_changed(app, owner);
    Ok(())
}

fn activate_tab_without_focus(
    app: &tauri::AppHandle,
    owner: &str,
    tab_id: &str,
) -> Result<(), String> {
    let active_tab = {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let tab = state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id || tab.target == tab_id || tab.label == tab_id)
            .cloned()
            .ok_or_else(|| format!("タブが見つかりません: {}", tab_id))?;
        state.active = Some(tab.id.clone());
        tab
    };

    crate::webview_toolbar::set_owner_active_target(
        owner,
        &active_tab.target,
        &active_tab.title,
        &active_tab.kind,
    );
    if let Some(window) = app.get_window(owner) {
        let _ = window.set_title(&active_tab.title);
    }
    resize_current_for_owner(app, owner)?;
    emit_tabs_changed(app, owner);
    Ok(())
}
