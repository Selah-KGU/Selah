use super::*;
use crate::agent_pseudo_call::parse_any as parse_any_visible_tool_call;

fn tool_row(name: &str) -> crate::db::AgentMessageRow {
    crate::db::AgentMessageRow {
        id: 1,
        conv_id: "c".into(),
        role: "tool".into(),
        content: String::new(),
        images_json: None,
        tool_name: Some(name.into()),
        tool_result_json: Some("{\"classes\":[]}".into()),
        created_at: 0,
    }
}

#[path = "tests/browser.rs"]
mod browser;
#[path = "tests/planning.rs"]
mod planning;
#[path = "tests/routing.rs"]
mod routing;
#[path = "tests/tool_calls.rs"]
mod tool_calls;
