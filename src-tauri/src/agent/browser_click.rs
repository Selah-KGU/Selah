//! Browser click-target inference for the local agent loop.
//!
//! Turns a user phrase plus a page observation (or a home-click screenshot)
//! into webview coordinates. Planning and tool execution stay in the parent
//! module.

#[path = "browser_click/candidates.rs"]
mod candidates;
#[path = "browser_click/infer.rs"]
mod infer;
#[path = "browser_click/labels.rs"]
mod labels;

struct BrowserClickCandidate {
    label: String,
    url: String,
    center_x: i64,
    center_y: i64,
}

pub(in crate::agent) use infer::{
    infer_mouse_click_from_observation, infer_mouse_click_from_screenshot,
    infer_tab_browse_click_from_observation, local_browser_action_answer,
};
pub(in crate::agent) use labels::browser_click_labels_for_turn;
#[allow(unused_imports)]
pub(in crate::agent) use labels::requested_click_labels;
