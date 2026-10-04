//! Keyword shortcuts and attached-browser plans.
//!
//! Unambiguous requests skip the model. Browser follow-ups stay deterministic
//! so a hung planner cannot drop a navigation the user already asked for.

#[path = "heuristic/browser.rs"]
mod browser;
#[cfg(test)]
#[path = "heuristic/plan.rs"]
mod plan;
#[cfg(test)]
#[path = "heuristic/rules.rs"]
mod rules;
#[path = "heuristic/single.rs"]
mod single;

pub(super) use browser::{
    attached_browser_control_plan, extract_browser_click_text, is_browser_operation_intent,
};
#[cfg(test)]
pub(super) use plan::heuristic_plan;
pub(super) use single::single_tool_plan;
