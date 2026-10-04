use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tauri::Manager;

#[path = "webview_toolbar/actions.rs"]
mod actions;
#[path = "webview_toolbar/bridge_script.rs"]
mod bridge_script;
#[path = "webview_toolbar/commands.rs"]
mod commands;

pub use actions::*;
use bridge_script::BROWSER_BRIDGE_SCRIPT;
pub use commands::*;

static PAGE_TEXT_WAITERS: LazyLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<PageTextPayload>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_ACTION_WAITERS: LazyLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<Value>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
#[cfg(debug_assertions)]
static BROWSER_MOUSE_SELFTEST_WAITERS: LazyLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<BrowserMouseSelftestReport>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_WINDOW_LABELS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static BROWSER_WINDOW_TARGETS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_WINDOW_OWNERS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_WINDOW_TITLES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_WINDOW_KINDS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
// WKWebView.URL can briefly be nil while navigating or closing, and Wry 0.54
// unwraps it internally. Keep event-driven snapshots instead of calling url().
static BROWSER_WINDOW_URLS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct BrowserWindowInfo {
    pub label: String,
    pub target: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

pub fn browser_bridge_script() -> &'static str {
    BROWSER_BRIDGE_SCRIPT
}

pub fn browser_window_label_from_target(target: &str) -> String {
    target
        .strip_suffix("-ct")
        .or_else(|| target.strip_suffix("-tb"))
        .unwrap_or(target)
        .to_string()
}

pub fn emit_browser_agent_status(app: &tauri::AppHandle, target: &str, active: bool, action: &str) {
    crate::document_tabs::emit_agent_status(app, target, active, action);
}

pub fn register_readable_child(
    app: &tauri::AppHandle,
    owner_label: &str,
    label: &str,
    target: &str,
    title: &str,
    kind: &str,
) {
    if app.get_window(owner_label).is_none() || app.get_webview(target).is_none() {
        return;
    }
    BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string());
    BROWSER_WINDOW_TARGETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string(), target.to_string());
    BROWSER_WINDOW_OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string(), owner_label.to_string());
    BROWSER_WINDOW_TITLES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string(), title.to_string());
    BROWSER_WINDOW_KINDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string(), kind.to_string());
}

pub fn unregister_readable_label(label: &str) {
    BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(label);
    let target = BROWSER_WINDOW_TARGETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(label);
    if let Some(target) = target {
        BROWSER_WINDOW_URLS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&target);
    }
    BROWSER_WINDOW_OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(label);
    BROWSER_WINDOW_TITLES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(label);
    BROWSER_WINDOW_KINDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(label);
}

pub fn set_readable_url(target: &str, url: &str) {
    BROWSER_WINDOW_URLS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(target.to_string(), url.to_string());
}

fn readable_url(target: &str) -> String {
    BROWSER_WINDOW_URLS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(target)
        .cloned()
        .unwrap_or_default()
}

pub fn set_owner_active_target(owner_label: &str, target: &str, title: &str, kind: &str) {
    BROWSER_WINDOW_TARGETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(owner_label.to_string(), target.to_string());
    BROWSER_WINDOW_TITLES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(owner_label.to_string(), title.to_string());
    BROWSER_WINDOW_KINDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(owner_label.to_string(), kind.to_string());
}

#[cfg(debug_assertions)]
fn browser_popup_title(url: &url::Url) -> String {
    url.host_str()
        .filter(|host| !host.trim().is_empty())
        .unwrap_or_else(|| url.as_str())
        .to_string()
}

#[cfg(debug_assertions)]
fn open_browser_popup_window(
    app: &tauri::AppHandle,
    url: url::Url,
) -> Result<BrowserWindowInfo, String> {
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported popup URL scheme: {}", scheme));
    }
    let title = browser_popup_title(&url);
    crate::document_tabs::open_external_tab(app, url.to_string(), Some(title))
}

pub struct BrowserAgentStatusGuard {
    app: tauri::AppHandle,
    target: String,
    active: bool,
}

impl BrowserAgentStatusGuard {
    pub fn start(app: &tauri::AppHandle, target: &str, action: &str) -> Self {
        emit_browser_agent_status(app, target, true, action);
        Self {
            app: app.clone(),
            target: target.to_string(),
            active: true,
        }
    }

    pub fn finish(mut self) {
        self.clear();
    }

    fn clear(&mut self) {
        if self.active {
            emit_browser_agent_status(&self.app, &self.target, false, "");
            self.active = false;
        }
    }
}

impl Drop for BrowserAgentStatusGuard {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserElementRect {
    #[serde(default)]
    pub x: i64,
    #[serde(default)]
    pub y: i64,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
    #[serde(default)]
    pub center_x: i64,
    #[serde(default)]
    pub center_y: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserViewportPayload {
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
    #[serde(default)]
    pub scroll_x: i64,
    #[serde(default)]
    pub scroll_y: i64,
    #[serde(default)]
    pub scroll_height: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserLinkPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub rect: Option<BrowserElementRect>,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserButtonPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub rect: Option<BrowserElementRect>,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserInputPayload {
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub rect: Option<BrowserElementRect>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTextPayload {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub viewport: Option<BrowserViewportPayload>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub headings: Vec<String>,
    #[serde(default)]
    pub links: Vec<BrowserLinkPayload>,
    #[serde(default)]
    pub buttons: Vec<BrowserButtonPayload>,
    #[serde(default)]
    pub inputs: Vec<BrowserInputPayload>,
    #[serde(default)]
    pub content_source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPageTextReport {
    request_id: String,
    payload: PageTextPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionReport {
    request_id: String,
    payload: Value,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMouseSelftestReport {
    request_id: String,
    #[serde(default)]
    count: u32,
    #[serde(default)]
    href: String,
}

/// Create a standalone content window used only by the browser mouse selftest.
#[cfg(debug_assertions)]
pub fn create_browser_window(
    app: &tauri::AppHandle,
    label: &str,
    url: tauri::WebviewUrl,
    title: &str,
    width: f64,
    height: f64,
    init_scripts: &[&str],
) -> Result<BrowserWindowInfo, String> {
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
    BROWSER_WINDOW_TARGETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string(), content_label.clone());
    BROWSER_WINDOW_OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string(), label.to_string());

    let target_for_load = content_label.clone();
    let mut content_builder = tauri::webview::WebviewBuilder::new(&content_label, url)
        .initialization_script(BROWSER_BRIDGE_SCRIPT)
        .on_page_load(move |_webview, payload| {
            set_readable_url(&target_for_load, payload.url().as_str());
        });
    for script in init_scripts {
        content_builder = content_builder.initialization_script(*script);
    }

    let app_for_new_window = app.clone();
    content_builder = content_builder.on_new_window(move |popup_url, _features| {
        let app_for_open = app_for_new_window.clone();
        let log_url = popup_url.to_string();
        // This callback runs on the event-loop thread; creating the popup's
        // webviews there deadlocks on Windows (add_child blocks on the event
        // loop), so hand the work to a background thread.
        std::thread::spawn(move || {
            if let Err(err) = open_browser_popup_window(&app_for_open, popup_url) {
                log::warn!(
                    "[browser] failed to open requested popup window url={}: {}",
                    log_url,
                    err
                );
            }
        });
        tauri::webview::NewWindowResponse::Deny
    });

    window
        .add_child(
            content_builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, 0.0)),
            tauri::Size::Logical(tauri::LogicalSize::new(width, height)),
        )
        .map_err(|e| format!("コンテンツ作成失敗: {}", e))?;

    Ok(BrowserWindowInfo {
        label: label.to_string(),
        target: content_label,
        url: String::new(),
        title: title.to_string(),
        kind: "browser".to_string(),
    })
}
