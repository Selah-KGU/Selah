use serde_json::Value;

use super::super::{Plan, ToolCall};

pub(in crate::agent) fn single_tool_plan(name: &str, args: Value) -> Plan {
    Plan {
        tools: vec![ToolCall {
            name: name.into(),
            args,
        }],
        image_only: false,
    }
}
