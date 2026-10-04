//! Split ratios, divider webviews, and document-window layout.

use super::*;

/// Flatten a child-pane chain into (target, label) pairs (pane2, pane3, …).
pub(super) fn pane_chain(child: &Option<ChildPane>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = child.as_ref();
    while let Some(c) = cur {
        out.push((c.target.clone(), c.label.clone()));
        cur = c.child.as_deref();
    }
    out
}

/// Remove the pane with `target` (and any panes below it) from a chain. Returns
/// the (target, label) pairs that were removed, or None if not found.
pub(super) fn truncate_pane_at(
    node: &mut Option<ChildPane>,
    target: &str,
) -> Option<Vec<(String, String)>> {
    // The pane itself (pane2).
    if node.as_ref().map(|c| c.target == target).unwrap_or(false) {
        let removed = pane_chain(node);
        *node = None;
        return Some(removed);
    }
    // A deeper pane (pane3).
    if let Some(parent) = node.as_mut() {
        let is_deeper = parent
            .child
            .as_ref()
            .map(|b| b.target == target)
            .unwrap_or(false);
        if is_deeper {
            let removed = parent
                .child
                .as_deref()
                .map(|b| {
                    let mut v = vec![(b.target.clone(), b.label.clone())];
                    v.extend(pane_chain(&b.child.as_deref().cloned()));
                    v
                })
                .unwrap_or_default();
            parent.child = None;
            return Some(removed);
        }
    }
    None
}

pub(super) fn default_split_weights(columns: usize) -> Vec<f64> {
    if columns <= 1 {
        Vec::new()
    } else {
        vec![1.0 / columns as f64; columns]
    }
}

pub(super) fn normalize_split_weights(weights: &[f64], columns: usize) -> Vec<f64> {
    if columns <= 1 {
        return Vec::new();
    }
    let cleaned: Vec<f64> = weights
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    if cleaned.len() != columns {
        return default_split_weights(columns);
    }
    let sum: f64 = cleaned.iter().sum();
    if sum <= f64::EPSILON {
        return default_split_weights(columns);
    }
    cleaned.into_iter().map(|value| value / sum).collect()
}

pub(super) fn clamped_split_weights_with_min(
    weights: &[f64],
    columns: usize,
    min_ratio: f64,
) -> Vec<f64> {
    if columns <= 1 {
        return Vec::new();
    }
    let base = normalize_split_weights(weights, columns);
    let min_ratio = min_ratio.min(1.0 / columns as f64);
    if min_ratio <= 0.0 {
        return base;
    }

    let mut out = vec![0.0; columns];
    let mut locked = vec![false; columns];
    let mut remaining_total = 1.0;

    loop {
        let unlocked: Vec<usize> = (0..columns).filter(|index| !locked[*index]).collect();
        if unlocked.is_empty() {
            break;
        }
        let unlocked_weight: f64 = unlocked.iter().map(|index| base[*index]).sum();
        let unlocked_count = unlocked.len();
        let mut changed = false;

        for index in unlocked {
            let projected = if unlocked_weight <= f64::EPSILON {
                remaining_total / unlocked_count as f64
            } else {
                base[index] / unlocked_weight * remaining_total
            };
            if projected + 1e-9 < min_ratio {
                out[index] = min_ratio;
                locked[index] = true;
                remaining_total = (remaining_total - min_ratio).max(0.0);
                changed = true;
            }
        }

        if !changed {
            let unlocked: Vec<usize> = (0..columns).filter(|index| !locked[*index]).collect();
            let unlocked_weight: f64 = unlocked.iter().map(|index| base[*index]).sum();
            let unlocked_count = unlocked.len();
            for index in unlocked {
                out[index] = if unlocked_weight <= f64::EPSILON {
                    remaining_total / unlocked_count as f64
                } else {
                    base[index] / unlocked_weight * remaining_total
                };
            }
            break;
        }
    }

    normalize_split_weights(&out, columns)
}

pub(super) fn clamped_split_weights(weights: &[f64], columns: usize) -> Vec<f64> {
    clamped_split_weights_with_min(weights, columns, MIN_SPLIT_RATIO)
}

pub(super) fn split_ratios_for_layout(child: &Option<ChildPane>, weights: &[f64]) -> Vec<f64> {
    let columns = 1 + pane_chain(child).len();
    if columns <= 1 {
        Vec::new()
    } else {
        clamped_split_weights(weights, columns)
    }
}

pub(super) fn split_widths_for_layout(
    total_width: f64,
    columns: usize,
    weights: &[f64],
) -> Vec<f64> {
    if columns == 0 {
        return Vec::new();
    }
    if columns == 1 {
        return vec![total_width.max(0.0)];
    }
    let divider_total = SPLIT_DIVIDER_WIDTH * (columns.saturating_sub(1)) as f64;
    let usable_width = (total_width - divider_total).max(120.0);
    clamped_split_weights(weights, columns)
        .into_iter()
        .map(|weight| usable_width * weight)
        .collect()
}

pub(super) fn split_weights_for_second(weights: &[f64]) -> Vec<f64> {
    match clamped_split_weights(weights, weights.len()).as_slice() {
        [left, right] => vec![*left, *right],
        [left, middle, right] => vec![*left, middle + right],
        _ => default_split_weights(2),
    }
}

pub(super) fn logical_window_size(window: &tauri::Window) -> Result<(f64, f64), String> {
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    Ok((size.width as f64 / scale, size.height as f64 / scale))
}

pub(super) fn content_width_for_owner(app: &tauri::AppHandle, owner: &str) -> Result<f64, String> {
    let window = app
        .get_window(owner)
        .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
    let (width, _) = logical_window_size(&window)?;
    let clamped_width = width.max(320.0);
    let panel_width = agent_panel_width(clamped_width);
    Ok((clamped_width - panel_width).max(260.0))
}

pub(super) fn resize_document_window(app: &tauri::AppHandle, owner: &str, width: f64, height: f64) {
    // Resize the strip FIRST, with the least possible work, so its responsive
    // width @media (max-width: 760px) tracks the window edge closely during a
    // drag. Detective collapses the toolbar row away (shorter strip); only a
    // cheap active-kind read is needed — the heavier full-state clone for the
    // content layout happens afterwards.
    let strip_h = if matches!(
        active_kind_for_owner(owner).as_deref(),
        Some("detective") | Some("paper-check")
    ) {
        COMPACT_STRIP_HEIGHT
    } else {
        TAB_STRIP_HEIGHT
    };
    let width = width.max(320.0);
    let height = height.max(strip_h + 120.0);
    if let Some(strip) = app.get_webview(TAB_STRIP_LABEL) {
        let _ = strip.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            0.0, 0.0,
        )));
        let _ = strip.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            width, strip_h,
        )));
    }

    let state = DOCUMENT_WINDOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(owner)
        .cloned()
        .unwrap_or_default();
    let panel_width = agent_panel_width(width);
    let content_width = (width - panel_width).max(260.0);
    let document_height = (height - strip_h).max(120.0);
    let window = app.get_window(owner);
    let active = state.active.as_deref();
    let content_height = document_height.max(120.0);
    for tab in state.tabs {
        let is_active = active == Some(tab.id.as_str());
        // All split panes attached to this tab (pane2, pane3, …).
        let all_panes = pane_chain(&tab.child);
        // The contiguous panes that actually have a live webview, in order.
        let live_panes: Vec<&(String, String)> = all_panes
            .iter()
            .take_while(|(target, _)| app.get_webview(target).is_some())
            .collect();

        if is_active {
            let columns = 1 + live_panes.len();
            let widths = split_widths_for_layout(content_width, columns, &tab.split_weights);
            if let Some(webview) = app.get_webview(&tab.target) {
                let main_width = widths.first().copied().unwrap_or(content_width);
                let _ = webview.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                    main_width,
                    content_height,
                )));
                let _ = webview.set_position(tauri::Position::Logical(
                    tauri::LogicalPosition::new(0.0, strip_h),
                ));
                let _ = webview.show();
            }
            if let Some(window) = window.as_ref() {
                for index in 0..live_panes.len() {
                    if let Err(err) =
                        ensure_split_divider_webview(app, window, owner, &tab.target, index)
                    {
                        log::warn!(
                            "split divider ensure failed owner={} parent={} index={} err={}",
                            owner,
                            tab.target,
                            index,
                            err
                        );
                    }
                }
            }
            close_split_dividers(app, &tab.target, live_panes.len());

            let mut x = widths.first().copied().unwrap_or(content_width);
            for (index, (target, _)) in live_panes.iter().enumerate() {
                let divider_x = x;
                if let Some(divider) = app.get_webview(&split_divider_target(&tab.target, index)) {
                    let dragging_this_divider = tab.active_split_drag == Some(index);
                    let divider_width = if dragging_this_divider {
                        content_width
                    } else {
                        SPLIT_DIVIDER_WIDTH
                    };
                    let divider_pos_x = if dragging_this_divider {
                        0.0
                    } else {
                        divider_x
                    };
                    let _ = divider.set_position(tauri::Position::Logical(
                        tauri::LogicalPosition::new(divider_pos_x, TAB_STRIP_HEIGHT),
                    ));
                    let _ = divider.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                        divider_width,
                        content_height,
                    )));
                    let _ = divider.show();
                }
                let pane_x = divider_x + SPLIT_DIVIDER_WIDTH;
                if let Some(view) = app.get_webview(target) {
                    let pane_width = widths.get(index + 1).copied().unwrap_or_else(|| {
                        ((content_width - SPLIT_DIVIDER_WIDTH * live_panes.len() as f64)
                            / columns as f64)
                            .max(120.0)
                    });
                    let _ = view.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                        pane_width,
                        content_height,
                    )));
                    let _ = view.set_position(tauri::Position::Logical(
                        tauri::LogicalPosition::new(pane_x, TAB_STRIP_HEIGHT),
                    ));
                    let _ = view.show();
                }
                x = pane_x + widths.get(index + 1).copied().unwrap_or(0.0);
            }
        } else {
            // Hide instead of parking a full-size visible webview at x=50000.
            // A shown child still composites and, with the old policy, kept running.
            hide_split_dividers(app, &tab.target);
            if let Some(webview) = app.get_webview(&tab.target) {
                let _ = webview.hide();
            }
            for (target, _) in &all_panes {
                if let Some(view) = app.get_webview(target) {
                    let _ = view.hide();
                }
            }
        }
    }

    if panel_width > 0.0 {
        if let Some(panel) = app.get_webview(AGENT_PANEL_LABEL) {
            let _ = panel.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                content_width,
                strip_h,
            )));
            let _ = panel.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                panel_width,
                document_height,
            )));
            let _ = panel.show();
        }
    } else if let Some(panel) = app.get_webview(AGENT_PANEL_LABEL) {
        let _ = panel.hide();
    }
}

pub fn resize_current_for_owner(app: &tauri::AppHandle, owner: &str) -> Result<(), String> {
    let window = app
        .get_window(owner)
        .ok_or_else(|| format!("タブウィンドウが見つかりません: {}", owner))?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    resize_document_window(
        app,
        owner,
        size.width as f64 / scale,
        size.height as f64 / scale,
    );
    Ok(())
}

pub(super) fn split_divider_target(parent_target: &str, index: usize) -> String {
    format!("{}-divider-{}", parent_target, index)
}

pub(super) fn split_divider_url(owner: &str, parent_target: &str, index: usize) -> String {
    format!(
        "index.html#surface=split-divider&ownerLabel={}&parentTarget={}&handleIndex={}",
        urlencoding::encode(owner),
        urlencoding::encode(parent_target),
        index
    )
}

pub(super) fn add_split_divider_webview(
    window: &tauri::Window,
    owner: &str,
    parent_target: &str,
    index: usize,
) -> Result<(), String> {
    let target = split_divider_target(parent_target, index);
    let url = split_divider_url(owner, parent_target, index);
    let builder = tauri::webview::WebviewBuilder::new(&target, tauri::WebviewUrl::App(url.into()))
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Suspend);
    window
        .add_child(
            builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(OFFSCREEN_X, TAB_STRIP_HEIGHT)),
            tauri::Size::Logical(tauri::LogicalSize::new(SPLIT_DIVIDER_WIDTH, 120.0)),
        )
        .map(|_| ())
        .map_err(|e| format!("分割バー作成失敗: {}", e))
}

pub(super) fn ensure_split_divider_webview(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    owner: &str,
    parent_target: &str,
    index: usize,
) -> Result<String, String> {
    let target = split_divider_target(parent_target, index);
    if app.get_webview(&target).is_none() {
        if on_main_thread() {
            // add_child would deadlock here (see note on ensure_window): create
            // the divider from a background thread and re-run the layout once it
            // exists so it gets positioned.
            let app = app.clone();
            let window = window.clone();
            let owner = owner.to_string();
            let parent_target = parent_target.to_string();
            std::thread::spawn(move || {
                if app
                    .get_webview(&split_divider_target(&parent_target, index))
                    .is_some()
                {
                    return;
                }
                if add_split_divider_webview(&window, &owner, &parent_target, index).is_ok() {
                    let _ = resize_current_for_owner(&app, &owner);
                }
            });
        } else {
            add_split_divider_webview(window, owner, parent_target, index)?;
        }
    }
    Ok(target)
}

pub(super) fn close_split_dividers(app: &tauri::AppHandle, parent_target: &str, keep: usize) {
    for index in keep..MAX_SPLIT_DIVIDERS {
        if let Some(webview) = app.get_webview(&split_divider_target(parent_target, index)) {
            let _ = webview.close();
        }
    }
}

pub(super) fn hide_split_dividers(app: &tauri::AppHandle, parent_target: &str) {
    for index in 0..MAX_SPLIT_DIVIDERS {
        if let Some(webview) = app.get_webview(&split_divider_target(parent_target, index)) {
            let _ = webview.hide();
            let _ = webview.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                OFFSCREEN_X,
                TAB_STRIP_HEIGHT,
            )));
            let _ = webview.set_size(tauri::Size::Logical(tauri::LogicalSize::new(0.0, 0.0)));
        }
    }
}
