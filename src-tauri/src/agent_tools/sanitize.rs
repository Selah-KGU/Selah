//! Argument sanitizers for agent tool calls.

#[path = "sanitize/browser.rs"]
mod browser;
#[path = "sanitize/calendar.rs"]
mod calendar;
#[path = "sanitize/computer.rs"]
mod computer;
#[path = "sanitize/schema.rs"]
mod schema;
#[path = "sanitize/text.rs"]
mod text;

use super::catalog::TOOL_SPECS;
use super::*;

pub(in crate::agent_tools) use text::sanitize_text_arg;

pub fn sanitize_tool_args(name: &str, args: &Value) -> Option<Value> {
    let name = canonical_tool_name(name)?;
    let spec = TOOL_SPECS.iter().find(|s| s.name == name)?;
    schema::sanitize_by_schema(spec.schema, args)
}
