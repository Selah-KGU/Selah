//! Read one tab or the UI projection directly from the owner state. Avoid an
//! intermediate copy of every tab, including private pane/control payloads.
use super::*;

fn selected_tab<'a>(
    state: &'a DocumentWindowState,
    tab_id: Option<&str>,
) -> Option<&'a DocumentTab> {
    let id = tab_id.or(state.active.as_deref())?;
    state.tabs.iter().find(|tab| tab.id == id)
}

fn tab_infos(state: &DocumentWindowState) -> Vec<DocumentTabInfo> {
    state
        .tabs
        .iter()
        .map(|tab| tab.info(state.active.as_deref() == Some(tab.id.as_str())))
        .collect()
}

pub(super) fn list_tabs_for_owner(owner: &str) -> Vec<DocumentTabInfo> {
    let states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    states.get(owner).map(tab_infos).unwrap_or_default()
}

pub(super) fn tab_for_owner(owner: &str, tab_id: Option<&str>) -> Option<DocumentTab> {
    let states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    selected_tab(states.get(owner)?, tab_id).cloned()
}

pub(super) fn active_tab_for_owner(owner: &str) -> Option<DocumentTab> {
    tab_for_owner(owner, None)
}

fn update_controls(
    state: Option<&mut DocumentWindowState>,
    target: Option<&str>,
    controls: Vec<DocumentTabControl>,
) -> Result<bool, String> {
    let missing = || match target {
        Some(target) => format!("タブが見つかりません: {}", target),
        None => "アクティブなタブがありません".to_string(),
    };
    let state = state.ok_or_else(missing)?;
    let target = target
        .or_else(|| selected_tab(state, None).map(|tab| tab.target.as_str()))
        .ok_or_else(missing)?;
    let index = state
        .tabs
        .iter()
        .position(|tab| tab.target == target || tab.id == target || tab.label == target)
        .ok_or_else(|| format!("タブが見つかりません: {}", target))?;
    let tab = &mut state.tabs[index];
    if tab.controls == controls {
        return Ok(false);
    }
    tab.controls = controls;
    Ok(true)
}

pub(super) fn set_controls_for_owner(
    owner: &str,
    target: Option<&str>,
    controls: Vec<DocumentTabControl>,
) -> Result<bool, String> {
    let mut states = DOCUMENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    update_controls(states.get_mut(owner), target, controls)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
