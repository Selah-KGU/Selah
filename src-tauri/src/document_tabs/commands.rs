//! Tauri commands for the document-tabs window.

use super::*;

#[tauri::command]
pub fn document_tabs_list(owner: Option<String>) -> Vec<DocumentTabInfo> {
    list_tabs_for_owner(owner.as_deref().unwrap_or(OWNER_LABEL))
}

#[tauri::command]
pub fn document_tabs_set_controls(
    app: tauri::AppHandle,
    owner: Option<String>,
    target: Option<String>,
    controls: Vec<DocumentTabControl>,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    if set_controls_for_owner(&owner, target.as_deref(), controls)? {
        emit_tabs_changed(&app, &owner);
    }
    Ok(())
}

#[tauri::command]
pub fn document_tabs_report_title(app: tauri::AppHandle, target: String, title: String) {
    let title = title.trim().to_string();
    if title.is_empty() {
        return;
    }
    let mut changed_owner: Option<String> = None;
    let mut is_active = false;
    {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        for (owner, state) in states.iter_mut() {
            if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.target == target) {
                // Only browser tabs carry a placeholder host title; surface tabs
                // (reader/detail/files/home) manage their own labels.
                if tab.kind == "browser" && tab.title != title {
                    is_active = state.active.as_deref() == Some(tab.id.as_str());
                    tab.title = title.clone();
                    changed_owner = Some(owner.clone());
                }
                break;
            }
        }
    }
    if let Some(owner) = changed_owner {
        if is_active {
            crate::webview_toolbar::set_owner_active_target(&owner, &target, &title, "browser");
            if let Some(window) = app.get_window(&owner) {
                let _ = window.set_title(&title);
            }
        }
        emit_tabs_changed(&app, &owner);
    }
}

#[tauri::command]
pub fn document_tabs_send_control(
    app: tauri::AppHandle,
    owner: Option<String>,
    tab_id: Option<String>,
    action: String,
    payload: Option<serde_json::Value>,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let tab = match tab_for_owner(&owner, tab_id.as_deref()) {
        Some(tab) => tab,
        // A captured tab may have closed while the command was being delivered.
        // Never redirect its action to a different currently active page.
        None if tab_id.is_some() => return Ok(()),
        None => return Err("アクティブなタブがありません".to_string()),
    };
    let mut targets = vec![tab.target.clone()];
    if action == "detail.refresh" {
        targets.extend(pane_chain(&tab.child).into_iter().map(|(target, _)| target));
    }
    let payload = payload.unwrap_or(serde_json::Value::Null);
    for target in targets {
        app.emit_to(
            tauri::EventTarget::AnyLabel {
                label: target.clone(),
            },
            "document-tab-control",
            serde_json::json!({
                "owner": owner,
                "target": target,
                "tabId": tab.id,
                "action": action,
                "payload": payload,
            }),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn document_tabs_report_probe(report: DocumentTabProbeReport) {
    let suspicious = report.kind == "detail"
        && (!report.has_content
            || report.content_children <= 0
            || report.content_text_length <= 0
            || (report.has_loading && !report.has_detail_wrap));
    if suspicious {
        log::warn!(
            "document tab probe suspicious owner={} id={} label={} target={} title={} kind={} ready={} visibility={} url={} href={} content={} children={} text={} html={} loading={} error={} detailWrap={} pageTitle={} viewport={}x{} body={}x{} preview={:?}",
            report.owner,
            report.id,
            report.label,
            report.target,
            report.title,
            report.kind,
            report.ready_state,
            report.visibility_state,
            report.url,
            report.href,
            report.has_content,
            report.content_children,
            report.content_text_length,
            report.content_html_length,
            report.has_loading,
            report.has_error,
            report.has_detail_wrap,
            report.has_page_title,
            report.viewport_width,
            report.viewport_height,
            report.body_width,
            report.body_height,
            report.preview,
        );
    } else {
        log::info!(
            "document tab probe owner={} id={} label={} target={} title={} kind={} ready={} visibility={} url={} href={} content={} children={} text={} html={} loading={} error={} detailWrap={} pageTitle={} viewport={}x{} body={}x{} preview={:?}",
            report.owner,
            report.id,
            report.label,
            report.target,
            report.title,
            report.kind,
            report.ready_state,
            report.visibility_state,
            report.url,
            report.href,
            report.has_content,
            report.content_children,
            report.content_text_length,
            report.content_html_length,
            report.has_loading,
            report.has_error,
            report.has_detail_wrap,
            report.has_page_title,
            report.viewport_width,
            report.viewport_height,
            report.body_width,
            report.body_height,
            report.preview,
        );
    }
}

#[tauri::command]
pub async fn document_tabs_activate(
    app: tauri::AppHandle,
    owner: Option<String>,
    id: String,
) -> Result<(), String> {
    activate_tab_inner(&app, owner.as_deref().unwrap_or(OWNER_LABEL), &id)
}

/// Bring the Copilot window to front (showing it if hidden) and, if an id is
/// given, activate that tab. Used by the sidebar Copilot dock in the main window.
#[tauri::command]
pub async fn document_tabs_reveal(
    app: tauri::AppHandle,
    owner: Option<String>,
    id: Option<String>,
) -> Result<(), String> {
    let owner = owner.as_deref().unwrap_or(OWNER_LABEL);
    // Reveal only shows an existing (possibly hidden) window — it never conjures
    // an empty one. Creating windows is document_tabs_new_tab's job.
    if app.get_window(owner).is_none() {
        return Ok(());
    }
    ensure_window(&app, owner)?;
    if let Some(id) = id {
        activate_tab_inner(&app, owner, &id)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn document_tabs_close(
    app: tauri::AppHandle,
    owner: Option<String>,
    id: String,
    focus: Option<bool>,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let (closed, next_active) = {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(&owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let Some(index) = state
            .tabs
            .iter()
            .position(|tab| tab.id == id || tab.target == id || tab.label == id)
        else {
            return Err(format!("タブが見つかりません: {}", id));
        };
        let closed = state.tabs.remove(index);
        crate::commands::discard_pending_markdown_payload(&closed.target);
        let next_active = if state.active.as_deref() == Some(closed.id.as_str()) {
            state
                .tabs
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|i| state.tabs.get(i)))
                .map(|tab| tab.id.clone())
        } else {
            state.active.clone()
        };
        state.active = next_active.clone();
        (closed, next_active)
    };

    crate::webview_toolbar::unregister_readable_label(&closed.label);
    if let Some(webview) = app.get_webview(&closed.target) {
        let _ = webview.close();
    }
    // Tear down all split panes along with their parent tab.
    for (target, label) in pane_chain(&closed.child) {
        crate::webview_toolbar::unregister_readable_label(&label);
        if let Some(child_view) = app.get_webview(&target) {
            let _ = child_view.close();
        }
    }
    close_split_dividers(&app, &closed.target, 0);
    if closed.kind == "browser" {
        // Browser controls live in the shared Svelte tab chrome.
    }

    if let Some(next) = next_active {
        if focus.unwrap_or(true) {
            activate_tab_inner(&app, &owner, &next)?;
        } else {
            activate_tab_without_focus(&app, &owner, &next)?;
        }
    } else if AGENT_PANEL_OPEN.load(Ordering::Relaxed) {
        resize_current_for_owner(&app, &owner)?;
        emit_tabs_changed(&app, &owner);
    } else if let Some(window) = app.get_window(&owner) {
        force_close_window(&window);
        DOCUMENT_WINDOWS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&owner);
        emit_tabs_changed(&app, &owner);
    }
    Ok(())
}

#[tauri::command]
pub async fn document_tabs_new_tab(app: tauri::AppHandle) -> Result<DocumentTabInfo, String> {
    open_new_tab(&app)
}

#[tauri::command]
pub fn document_tabs_reorder(
    app: tauri::AppHandle,
    owner: Option<String>,
    ids: Vec<String>,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(&owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        // Rebuild the tab order to follow `ids`; ids not found are skipped and any
        // tabs missing from `ids` are appended so nothing is ever dropped.
        let mut reordered: Vec<DocumentTab> = Vec::with_capacity(state.tabs.len());
        for id in &ids {
            if let Some(pos) = state.tabs.iter().position(|tab| &tab.id == id) {
                reordered.push(state.tabs.remove(pos));
            }
        }
        reordered.append(&mut state.tabs);
        state.tabs = reordered;
    }
    emit_tabs_changed(&app, &owner);
    Ok(())
}

#[tauri::command]
pub async fn document_tabs_close_split(
    app: tauri::AppHandle,
    owner: Option<String>,
    parent: Option<String>,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let parent_id = match parent {
        Some(id) => id,
        None => active_tab_for_owner(&owner)
            .map(|tab| tab.id)
            .ok_or_else(|| "アクティブなタブがありません".to_string())?,
    };
    close_child_pane(&app, &owner, &parent_id)
}

/// Close one split pane (identified by its webview target) and any panes nested
/// below it, leaving the rest of the split intact.
#[tauri::command]
pub async fn document_tabs_close_pane(
    app: tauri::AppHandle,
    owner: Option<String>,
    target: String,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let removed = {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(&owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let mut removed = Vec::new();
        for tab in state.tabs.iter_mut() {
            if let Some(panes) = truncate_pane_at(&mut tab.child, &target) {
                removed = panes;
                let columns = 1 + pane_chain(&tab.child).len();
                tab.split_weights = if columns <= 1 {
                    Vec::new()
                } else {
                    clamped_split_weights(&tab.split_weights, columns)
                };
                break;
            }
        }
        removed
    };
    for (pane_target, label) in removed {
        crate::webview_toolbar::unregister_readable_label(&label);
        if let Some(webview) = app.get_webview(&pane_target) {
            let _ = webview.close();
        }
    }
    resize_current_for_owner(&app, &owner)?;
    emit_tabs_changed(&app, &owner);
    Ok(())
}

#[tauri::command]
pub fn document_tabs_drag_split(
    app: tauri::AppHandle,
    owner: Option<String>,
    parent: Option<String>,
    handle_index: usize,
    base_ratios: Vec<f64>,
    delta_px: f64,
) -> Result<Vec<f64>, String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let parent_id = match parent {
        Some(id) => id,
        None => active_tab_for_owner(&owner)
            .map(|tab| tab.id)
            .ok_or_else(|| "アクティブなタブがありません".to_string())?,
    };
    let content_width = content_width_for_owner(&app, &owner)?;
    let applied = {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(&owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == parent_id || tab.target == parent_id || tab.label == parent_id)
            .ok_or_else(|| format!("タブが見つかりません: {}", parent_id))?;
        let columns = 1 + pane_chain(&tab.child).len();
        if columns <= 1 {
            return Err("分割ペインがありません".to_string());
        }
        if handle_index >= columns.saturating_sub(1) {
            return Err(format!("無効な分割バー index: {}", handle_index));
        }
        let usable_width =
            (content_width - SPLIT_DIVIDER_WIDTH * (columns.saturating_sub(1)) as f64).max(120.0);
        let delta_ratio = if usable_width <= f64::EPSILON {
            0.0
        } else {
            delta_px / usable_width
        };
        let min_ratio = MIN_SPLIT_RATIO;
        let mut w = clamped_split_weights(&base_ratios, columns);
        let i = handle_index;
        const EPS: f64 = 1e-9;
        if delta_ratio >= 0.0 {
            // Drag right: grow pane i, taking space from panes to its right,
            // cascading to the next one each time one bottoms out at the minimum.
            let mut need = delta_ratio;
            let mut j = i + 1;
            while need > EPS && j < columns {
                let take = (w[j] - min_ratio).max(0.0).min(need);
                w[j] -= take;
                w[i] += take;
                need -= take;
                j += 1;
            }
        } else {
            // Drag left: grow pane i+1, taking space from panes to its left
            // (pane i, then i-1, …), cascading through the minimum.
            let mut need = -delta_ratio;
            let mut j = i as isize;
            while need > EPS && j >= 0 {
                let idx = j as usize;
                let take = (w[idx] - min_ratio).max(0.0).min(need);
                w[idx] -= take;
                w[i + 1] += take;
                need -= take;
                j -= 1;
            }
        }
        let next = clamped_split_weights(&w, columns);
        tab.split_weights = next.clone();
        next
    };
    resize_current_for_owner(&app, &owner)?;
    emit_tabs_changed(&app, &owner);
    Ok(applied)
}

#[tauri::command]
pub fn document_tabs_begin_split_drag(
    app: tauri::AppHandle,
    owner: Option<String>,
    parent: Option<String>,
    handle_index: usize,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let parent_id = match parent {
        Some(id) => id,
        None => active_tab_for_owner(&owner)
            .map(|tab| tab.id)
            .ok_or_else(|| "アクティブなタブがありません".to_string())?,
    };
    {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(&owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == parent_id || tab.target == parent_id || tab.label == parent_id)
            .ok_or_else(|| format!("タブが見つかりません: {}", parent_id))?;
        let columns = 1 + pane_chain(&tab.child).len();
        if handle_index >= columns.saturating_sub(1) {
            return Err(format!("無効な分割バー index: {}", handle_index));
        }
        tab.active_split_drag = Some(handle_index);
    }
    resize_current_for_owner(&app, &owner)?;
    emit_tabs_changed(&app, &owner);
    Ok(())
}

#[tauri::command]
pub fn document_tabs_end_split_drag(
    app: tauri::AppHandle,
    owner: Option<String>,
    parent: Option<String>,
) -> Result<(), String> {
    let owner = owner.unwrap_or_else(|| OWNER_LABEL.to_string());
    let parent_id = match parent {
        Some(id) => id,
        None => active_tab_for_owner(&owner)
            .map(|tab| tab.id)
            .ok_or_else(|| "アクティブなタブがありません".to_string())?,
    };
    {
        let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let state = states
            .get_mut(&owner)
            .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == parent_id || tab.target == parent_id || tab.label == parent_id)
            .ok_or_else(|| format!("タブが見つかりません: {}", parent_id))?;
        tab.active_split_drag = None;
    }
    resize_current_for_owner(&app, &owner)?;
    emit_tabs_changed(&app, &owner);
    Ok(())
}

#[tauri::command]
pub async fn document_tabs_open_bookmark(
    app: tauri::AppHandle,
    spec: serde_json::Value,
) -> Result<(), String> {
    let kind = spec.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "browser" => {
            let url = spec
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("ブックマークにURLがありません")?;
            open_external_tab(&app, url.to_string(), None).map(|_| ())
        }
        "detail" => {
            let params = spec
                .get("params")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = spec
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("詳細")
                .to_string();
            open_university_detail_tab(&app, params, title).map(|_| ())
        }
        "reader" => {
            let path = spec
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("ブックマークにパスがありません")?;
            crate::commands::open_markdown_file_window(app.clone(), path.to_string()).await
        }
        other => Err(format!("未対応のブックマーク種別: {}", other)),
    }
}

#[tauri::command]
pub async fn document_tabs_open_agent(
    app: tauri::AppHandle,
    _owner: Option<String>,
) -> Result<(), String> {
    if AGENT_PANEL_OPEN.load(Ordering::Relaxed) {
        close_agent_panel(&app)
    } else {
        open_agent_workspace(&app)
    }
}

#[tauri::command]
pub fn document_tabs_agent_is_open() -> bool {
    AGENT_PANEL_OPEN.load(Ordering::Relaxed)
}

#[tauri::command]
pub fn document_tabs_resize_agent_panel(app: tauri::AppHandle, width: f64) -> Result<f64, String> {
    if !AGENT_PANEL_OPEN.load(Ordering::Relaxed) {
        return Err("Agent パネルが開いていません".to_string());
    }
    let window = app
        .get_window(OWNER_LABEL)
        .ok_or_else(|| "タブウィンドウが見つかりません".to_string())?;
    let (window_width, _) = logical_window_size(&window)?;
    let ratio = set_agent_panel_ratio(width / window_width.max(1.0));
    resize_current_for_owner(&app, OWNER_LABEL)?;
    Ok(ratio)
}
