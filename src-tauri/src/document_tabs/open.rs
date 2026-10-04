//! Opening document tabs, detail panes, and external pages.

use super::*;

#[allow(clippy::too_many_arguments)] // Tab construction keeps optional identity and restore fields explicit.
pub(super) fn open_tab(
    app: &tauri::AppHandle,
    key: Option<String>,
    label: Option<String>,
    target: Option<String>,
    url: tauri::WebviewUrl,
    initial_url: String,
    title: String,
    kind: String,
    init_scripts: &[&str],
    reopen: Option<serde_json::Value>,
) -> Result<DocumentTabInfo, String> {
    let owner = OWNER_LABEL;
    let window = ensure_window(app, owner)?;
    if let Some(key) = key.as_deref() {
        let existing = DOCUMENT_WINDOWS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(owner)
            .and_then(|state| {
                state
                    .tabs
                    .iter()
                    .find(|tab| tab.key.as_deref() == Some(key))
                    .cloned()
            });
        if let Some(tab) = existing {
            activate_tab_inner(app, owner, &tab.id)?;
            return Ok(tab.info(true));
        }
    }

    let count = DOCUMENT_WINDOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(owner)
        .map(|state| state.tabs.len())
        .unwrap_or(0);
    if count >= MAX_TABS {
        return Err(format!("タブは最大 {} 件までです", MAX_TABS));
    }

    let idx = TAB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = label.unwrap_or_else(|| format!("document-tab-{}", idx));
    let target = target.unwrap_or_else(|| format!("{}-ct", label));
    if app.get_webview(&target).is_some() {
        return Err(format!("Webview target already exists: {}", target));
    }

    let mut builder = tauri::webview::WebviewBuilder::new(&target, url)
        // Without this, the first click after the window is unfocused (or after a
        // tab switch) is swallowed to focus the webview instead of reaching the
        // page — making some pages feel "uninteractive".
        .accept_first_mouse(true)
        .initialization_script(crate::webview_toolbar::browser_bridge_script())
        // Hidden tabs use Suspend so macOS 14+ stops their JS. The tab strip and
        // agent panel stay unthrottled; only page content is parked this way.
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Suspend);
    for script in init_scripts {
        builder = builder.initialization_script(*script);
    }
    if kind == "browser" {
        // WKWebView ignores target="_blank" / window.open unless the app opens a
        // new window itself; the native new-window hook is unreliable, so route
        // those clicks to a managed tab from inside the page.
        builder = builder.initialization_script(BROWSER_LINK_HANDLER_SCRIPT);
    }

    let app_for_load = app.clone();
    let owner_for_load = owner.to_string();
    let tab_id_for_load = label.clone();
    let target_for_load = target.clone();
    builder = builder.on_page_load(move |webview, payload| {
        let started = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
        let finished = matches!(payload.event(), tauri::webview::PageLoadEvent::Finished);
        if !started && !finished {
            return;
        }
        let url = payload.url().to_string();
        crate::webview_toolbar::set_readable_url(&target_for_load, &url);
        let mut should_emit = false;
        if let Ok(mut states) = DOCUMENT_WINDOWS.lock() {
            if let Some(state) = states.get_mut(&owner_for_load) {
                if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == tab_id_for_load) {
                    tab.url = url.clone();
                    tab.loading = started;
                    should_emit = true;
                }
            }
        }
        if should_emit {
            emit_tabs_changed(&app_for_load, &owner_for_load);
        }
        // Browser tabs start with a host-name placeholder title; pull the real
        // <title> from the loaded page and reflect it on the tab.
        if finished {
            let _ = webview.eval(title_report_script(&target_for_load));
        }
    });

    let app_for_popup = app.clone();
    builder = builder.on_new_window(move |popup_url, _features| {
        let app_for_open = app_for_popup.clone();
        // This callback runs on the event-loop thread; opening the tab there
        // would deadlock on Windows (see note on ensure_window).
        std::thread::spawn(move || {
            let _ = open_external_tab(&app_for_open, popup_url.to_string(), None);
        });
        tauri::webview::NewWindowResponse::Deny
    });

    window
        .add_child(
            builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, TAB_STRIP_HEIGHT)),
            tauri::Size::Logical(tauri::LogicalSize::new(
                DEFAULT_WIDTH,
                DEFAULT_HEIGHT - TAB_STRIP_HEIGHT,
            )),
        )
        .map_err(|e| format!("タブ作成失敗: {}", e))?;

    let tab = DocumentTab {
        id: label.clone(),
        label: label.clone(),
        target: target.clone(),
        key,
        title: title.clone(),
        url: initial_url,
        kind: kind.clone(),
        controls: Vec::new(),
        reopen,
        loading: false,
        child: None,
        split_weights: Vec::new(),
        active_split_drag: None,
    };
    {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states.entry(owner.to_string()).or_default();
        state.tabs.push(tab.clone());
        state.active = Some(tab.id.clone());
    }
    crate::webview_toolbar::register_readable_child(app, owner, &label, &target, &title, &kind);
    activate_tab_inner(app, owner, &tab.id)?;
    Ok(tab.info(true))
}

pub fn open_markdown_reader_tab(
    app: &tauri::AppHandle,
    key: String,
    title: String,
    source_path: Option<String>,
) -> Result<DocumentTabInfo, String> {
    let label = key;
    let target = format!("{}-ct", label);
    let url = format!(
        "index.html#surface=markdown-reader&tabLabel={}&ownerLabel={}",
        urlencoding::encode(&target),
        urlencoding::encode(OWNER_LABEL)
    );
    // Only bookmarkable when we know the file path to reopen it from.
    let reopen = source_path
        .map(|path| serde_json::json!({ "type": "reader", "path": path, "title": title }));
    open_tab(
        app,
        Some(label.clone()),
        Some(label),
        Some(target),
        tauri::WebviewUrl::App(url.clone().into()),
        url,
        title,
        "reader".to_string(),
        &[],
        reopen,
    )
}

/// Open (or focus) a tab that loads one of the app's own `#surface=…` pages in
/// the Copilot window. `key` makes it a singleton (re-open focuses the existing
/// tab); `extra_query` appends to the URL hash (e.g. "&course=…").
pub(super) fn open_app_surface_tab(
    app: &tauri::AppHandle,
    surface: &str,
    kind: &str,
    title: String,
    key: Option<&str>,
    extra_query: &str,
) -> Result<DocumentTabInfo, String> {
    let idx = TAB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("document-tab-{}-{}", kind, idx);
    let target = format!("{}-ct", label);
    let url = format!(
        "index.html#surface={}&tabLabel={}&ownerLabel={}{}",
        surface,
        urlencoding::encode(&target),
        urlencoding::encode(OWNER_LABEL),
        extra_query
    );
    open_tab(
        app,
        key.map(str::to_string),
        Some(label),
        Some(target),
        tauri::WebviewUrl::App(url.clone().into()),
        url,
        title,
        kind.to_string(),
        &[],
        None,
    )
}

pub fn open_new_tab(app: &tauri::AppHandle) -> Result<DocumentTabInfo, String> {
    open_app_surface_tab(app, "home", "home", "新しいタブ".to_string(), None, "")
}

pub(super) fn detail_kind(params: &str) -> &'static str {
    if params.contains("mode=kwic") || params.contains("mode=kwicCabinet") {
        "kwic"
    } else if params.contains("mode=kgc") || params.contains("mode=syllabus") {
        "kgc"
    } else {
        "detail"
    }
}

pub(super) fn detail_webview_url(target: &str, params: &str) -> String {
    let suffix = if params.trim().is_empty() {
        String::new()
    } else {
        format!(
            "&{}",
            params.trim_start_matches('?').trim_start_matches('&')
        )
    };
    format!(
        "index.html#surface=university-detail&tabLabel={}&ownerLabel={}{}",
        urlencoding::encode(target),
        urlencoding::encode(OWNER_LABEL),
        suffix
    )
}

/// Open (or replace) the split child pane attached to the active detail tab.
/// The child sits to the right of its parent and is only shown while the parent
/// is the active tab.
pub(super) fn create_pane_webview(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    owner: &str,
    target: &str,
    label: &str,
    params: &str,
    title: &str,
) -> Result<(), String> {
    if let Some(existing) = app.get_webview(target) {
        let _ = existing.close();
    }
    crate::webview_toolbar::unregister_readable_label(label);
    let url = detail_webview_url(target, params);
    // Inject the browser bridge (__selahBrowserExtractText / __selahBrowserRunAction)
    // just like open_tab does, so the agent's DOM tools (read_browser_page,
    // browser_click, browser_fill, …) actually work inside split child panes.
    let target_for_load = target.to_string();
    let builder = tauri::webview::WebviewBuilder::new(target, tauri::WebviewUrl::App(url.into()))
        .initialization_script(crate::webview_toolbar::browser_bridge_script())
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Suspend)
        .on_page_load(move |_webview, payload| {
            crate::webview_toolbar::set_readable_url(&target_for_load, payload.url().as_str());
        });
    window
        .add_child(
            builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(OFFSCREEN_X, TAB_STRIP_HEIGHT)),
            tauri::Size::Logical(tauri::LogicalSize::new(DEFAULT_WIDTH / 3.0, 120.0)),
        )
        .map_err(|e| format!("分割ビュー作成失敗: {}", e))?;
    if let Some(view) = app.get_webview(target) {
        let _ = view.hide();
    }
    crate::webview_toolbar::register_readable_child(
        app,
        owner,
        label,
        target,
        title,
        detail_kind(params),
    );
    Ok(())
}

/// Open a split child pane. `origin` is the target of the webview that triggered
/// the open, so we know which pane to attach to. A third pane is only added when
/// drilling out of a discussion-board pane (掲示板).
pub fn open_child_detail(
    app: &tauri::AppHandle,
    params: String,
    title: String,
    origin: &str,
) -> Result<(), String> {
    let owner = OWNER_LABEL;
    let window = ensure_window(app, owner)?;
    let parent = active_tab_for_owner(owner).ok_or_else(|| "親詳細タブがありません".to_string())?;
    if parent.kind == "home" || parent.kind == "files" {
        return open_university_detail_tab(app, params, title).map(|_| ());
    }

    let new_is_board = params.contains("mode=discussion");
    let pane2_target = format!("{}-s1", parent.target);
    let pane2_label = format!("{}-s1", parent.label);
    let pane3_target = format!("{}-s2", parent.target);
    let pane3_label = format!("{}-s2", parent.label);

    // Inspect the current chain to decide where the new pane attaches.
    #[derive(PartialEq)]
    enum Slot {
        Pane2,
        Pane3,
    }
    let (slot, to_close): (Slot, Vec<(String, String)>) = {
        let states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let child = states
            .get(owner)
            .and_then(|s| s.tabs.iter().find(|t| t.id == parent.id))
            .and_then(|t| t.child.as_ref());
        let p2_present = child.is_some();
        let p2_is_board = child.map(|c| c.is_board).unwrap_or(false);
        let p3 = child.and_then(|c| c.child.as_deref());

        if origin == pane2_target && p2_present && p2_is_board {
            // Drilling out of a board pane → (re)place the third pane, keep pane2.
            (Slot::Pane3, pane_chain(&p3.cloned()))
        } else if origin == pane3_target && p2_present {
            // Drilling within the third pane → replace it.
            (Slot::Pane3, pane_chain(&p3.cloned()))
        } else {
            // Drilling from the parent (or a non-board pane) → reset to two panes,
            // dropping any existing children.
            let mut close = Vec::new();
            if let Some(c) = child {
                close.push((c.target.clone(), c.label.clone()));
                close.extend(pane_chain(&c.child.as_deref().cloned()));
            }
            (Slot::Pane2, close)
        }
    };

    for (target, label) in &to_close {
        crate::webview_toolbar::unregister_readable_label(label);
        if let Some(wv) = app.get_webview(target) {
            let _ = wv.close();
        }
    }

    let (target, label) = match slot {
        Slot::Pane2 => (pane2_target.clone(), pane2_label.clone()),
        Slot::Pane3 => (pane3_target.clone(), pane3_label.clone()),
    };
    create_pane_webview(app, &window, owner, &target, &label, &params, &title)?;

    {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = states.get_mut(owner) {
            if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == parent.id) {
                let new_pane = ChildPane {
                    target: target.clone(),
                    label: label.clone(),
                    is_board: new_is_board,
                    child: None,
                };
                match slot {
                    Slot::Pane2 => {
                        tab.child = Some(new_pane);
                        tab.split_weights = split_weights_for_second(&tab.split_weights);
                    }
                    Slot::Pane3 => {
                        if let Some(p2) = tab.child.as_mut() {
                            p2.child = Some(Box::new(new_pane));
                            // Reset to equal thirds (1:1:1) when the third pane appears.
                            tab.split_weights = default_split_weights(3);
                        } else {
                            tab.child = Some(new_pane);
                            tab.split_weights = split_weights_for_second(&tab.split_weights);
                        }
                    }
                }
            }
        }
    }
    close_split_dividers(
        app,
        &parent.target,
        match slot {
            Slot::Pane2 => 1,
            Slot::Pane3 => 2,
        },
    );
    resize_current_for_owner(app, owner)?;
    emit_tabs_changed(app, owner);
    Ok(())
}

pub(super) fn close_child_pane(
    app: &tauri::AppHandle,
    owner: &str,
    parent_id: &str,
) -> Result<(), String> {
    let (parent_target, panes) = {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == parent_id || tab.target == parent_id)
            .ok_or_else(|| format!("タブが見つかりません: {}", parent_id))?;
        let chain = pane_chain(&tab.child);
        let parent_target = tab.target.clone();
        tab.child = None;
        tab.split_weights.clear();
        tab.active_split_drag = None;
        (parent_target, chain)
    };
    for (target, label) in panes {
        crate::webview_toolbar::unregister_readable_label(&label);
        if let Some(webview) = app.get_webview(&target) {
            let _ = webview.close();
        }
    }
    close_split_dividers(app, &parent_target, 0);
    resize_current_for_owner(app, owner)?;
    emit_tabs_changed(app, owner);
    Ok(())
}

pub fn open_university_detail_tab(
    app: &tauri::AppHandle,
    params: String,
    title: String,
) -> Result<DocumentTabInfo, String> {
    let idx = TAB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("document-tab-university-detail-{}", idx);
    let target = format!("{}-ct", label);
    let kind = detail_kind(&params);
    let url = detail_webview_url(&target, &params);
    let reopen = Some(serde_json::json!({
        "type": "detail",
        "params": params,
        "title": title,
    }));
    open_tab(
        app,
        None,
        Some(label),
        Some(target),
        tauri::WebviewUrl::App(url.clone().into()),
        url,
        title,
        kind.to_string(),
        &[],
        reopen,
    )
}

pub fn open_files_tab(
    app: &tauri::AppHandle,
    course: Option<String>,
    title: String,
) -> Result<DocumentTabInfo, String> {
    let suffix = match course.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => format!("&course={}", urlencoding::encode(c)),
        None => String::new(),
    };
    // Keyed so re-opening focuses the single existing files tab instead of
    // stacking duplicates; the caller re-emits focus-course on reuse.
    open_app_surface_tab(app, "files", "files", title, Some("files"), &suffix)
}

/// Open (or focus) the Detective ("なるほど") game as a singleton tab in the
/// Copilot window. Keyed on "detective" so re-launching just re-focuses it.
pub fn open_detective_tab(app: &tauri::AppHandle) -> Result<DocumentTabInfo, String> {
    open_app_surface_tab(
        app,
        "detective",
        "detective",
        "なるほど".to_string(),
        Some("detective"),
        "",
    )
}

/// Open (or focus) the 論文チェッカー (paper AI-rate + similarity) app as a
/// singleton tab in the Copilot window. Keyed on "paper-check".
pub fn open_paper_check_tab(app: &tauri::AppHandle) -> Result<DocumentTabInfo, String> {
    open_app_surface_tab(
        app,
        "paper-check",
        "paper-check",
        "論文チェック".to_string(),
        Some("paper-check"),
        "",
    )
}

pub fn open_external_tab(
    app: &tauri::AppHandle,
    url: String,
    title: Option<String>,
) -> Result<crate::webview_toolbar::BrowserWindowInfo, String> {
    open_external_tab_with_scripts(app, url, title, &[])
}

pub fn open_external_tab_with_scripts(
    app: &tauri::AppHandle,
    url: String,
    title: Option<String>,
    init_scripts: &[&str],
) -> Result<crate::webview_toolbar::BrowserWindowInfo, String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("URL parse error: {}", e))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }
    let title = title.unwrap_or_else(|| parsed.host_str().unwrap_or("Web").to_string());
    let reopen = Some(serde_json::json!({ "type": "browser", "url": url }));
    let tab = open_tab(
        app,
        None,
        None,
        None,
        tauri::WebviewUrl::External(parsed),
        url.clone(),
        title.clone(),
        "browser".to_string(),
        init_scripts,
        reopen,
    )?;
    Ok(crate::webview_toolbar::BrowserWindowInfo {
        label: tab.label,
        target: tab.target,
        url,
        title,
        kind: "browser".to_string(),
    })
}
