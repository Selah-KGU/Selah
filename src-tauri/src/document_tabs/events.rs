//! Deliver one coherent tab snapshot to each interested listener.
use super::*;
use std::collections::HashSet;

fn snapshot_targets(tabs: &[DocumentTabInfo]) -> HashSet<String> {
    let mut labels = HashSet::from([TAB_STRIP_LABEL.to_string(), AGENT_PANEL_LABEL.to_string()]);
    for tab in tabs {
        for index in 0..MAX_SPLIT_DIVIDERS {
            labels.insert(split_divider_target(&tab.target, index));
        }
    }
    labels
}

fn matches_target(labels: &HashSet<String>, target: &tauri::EventTarget) -> bool {
    match target {
        tauri::EventTarget::AnyLabel { label }
        | tauri::EventTarget::Window { label }
        | tauri::EventTarget::Webview { label }
        | tauri::EventTarget::WebviewWindow { label } => labels.contains(label),
        _ => false,
    }
}

pub(super) fn emit_tab_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    owner: &str,
    tabs: Vec<DocumentTabInfo>,
) {
    let labels = snapshot_targets(&tabs);
    let payload = serde_json::json!({ "owner": owner, "tabs": tabs });
    // Tauri also delivers targeted emissions to EventTarget::Any listeners.
    // A per-label loop therefore repeated every global JS subscription 2+2N
    // times. One filter preserves label subscriptions and delivers Any once.
    let _ = app.emit_filter("document-tabs-changed", payload, |target| {
        matches_target(&labels, target)
    });
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
