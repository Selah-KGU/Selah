//! Plan finalization and browser-target policy.
//!
//! Rejects unknown tools, locks browser actions to the attached pane, and
//! decides whether a lookup should continue into another tool.

use super::*;

pub(super) fn finalize_plan(
    plan: Plan,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> Plan {
    finalize_plan_with_diagnostics(plan, history, user_text, turn_context).plan
}

#[derive(Default)]
pub(super) struct FinalizedPlan {
    pub(super) plan: Plan,
    pub(super) unknown_tools: Vec<String>,
    pub(super) invalid_args: Vec<String>,
}

impl FinalizedPlan {
    pub(super) fn has_rejections(&self) -> bool {
        !self.unknown_tools.is_empty() || !self.invalid_args.is_empty()
    }
}

pub(super) fn plan_repair_note(finalized: &FinalizedPlan) -> String {
    let unknown = if finalized.unknown_tools.is_empty() {
        "none".to_string()
    } else {
        finalized.unknown_tools.join(", ")
    };
    let invalid = if finalized.invalid_args.is_empty() {
        "none".to_string()
    } else {
        finalized.invalid_args.join(", ")
    };
    format!(
        "The previous plan selected unknown tools [{unknown}] or tools with invalid arguments [{invalid}]. \
         Do not repeat those names. If no exact listed tool fits, output {{\"tools\":[]}}."
    )
}

pub(super) fn contains_unresolved_plan_placeholder(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            let Some(inner) = trimmed.strip_prefix('<').and_then(|s| s.strip_suffix('>')) else {
                return false;
            };
            let ascii_letters: Vec<char> =
                inner.chars().filter(|c| c.is_ascii_alphabetic()).collect();
            !inner.is_empty()
                && (inner.contains('_')
                    || (!ascii_letters.is_empty()
                        && ascii_letters.iter().all(|c| c.is_ascii_uppercase())))
        }
        Value::Array(items) => items.iter().any(contains_unresolved_plan_placeholder),
        Value::Object(map) => map.values().any(contains_unresolved_plan_placeholder),
        _ => false,
    }
}

pub(super) fn is_browser_target_scoped_tool(name: &str) -> bool {
    matches!(
        name,
        "read_browser_page"
            | "browser_back"
            | "browser_forward"
            | "browser_reload_page"
            | "browser_click"
            | "browser_mouse_click"
            | "browser_mouse_drag"
            | "computer_screenshot"
            | "computer_mouse_click"
            | "computer_mouse_drag"
            | "computer_scroll"
            | "browser_fill"
            | "browser_select_option"
            | "browser_press"
            | "browser_scroll"
            | "browser_wait_for"
            | "browser_close"
    )
}

pub(super) fn is_browser_action_tool(name: &str) -> bool {
    matches!(
        name,
        "browser_back"
            | "browser_forward"
            | "browser_reload_page"
            | "browser_click"
            | "browser_mouse_click"
            | "browser_mouse_drag"
            | "computer_mouse_click"
            | "computer_mouse_drag"
            | "computer_scroll"
            | "browser_fill"
            | "browser_select_option"
            | "browser_press"
            | "browser_scroll"
            | "browser_close"
    )
}

pub(super) fn is_browser_mutation_tool(name: &str) -> bool {
    matches!(
        name,
        "browser_back"
            | "browser_forward"
            | "browser_reload_page"
            | "browser_click"
            | "browser_mouse_click"
            | "browser_mouse_drag"
            | "computer_mouse_click"
            | "computer_mouse_drag"
            | "browser_fill"
            | "browser_select_option"
            | "browser_press"
            | "browser_close"
    )
}

fn is_agent_action_tool(name: &str) -> bool {
    is_browser_action_tool(name)
        || matches!(
            name,
            "write_downloaded_text_file"
                | "open_downloaded_file"
                | "delete_downloaded_file"
                | "download_url"
                | "open_luna_attachment"
                | "download_luna_attachment"
                | "download_course_material"
                | "open_browser_url"
                | "open_copilot_page"
                | "create_google_calendar_event"
                | "delete_google_calendar_event"
                | "update_google_calendar_event"
        )
}

pub(super) fn should_continue_after_browser_observation(
    _plan: &Plan,
    results: &[(String, Value)],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> bool {
    turn_context.browser_target.is_some()
        && is_browser_operation_intent(&normalize_planner_text(user_text))
        && !results.iter().any(|(name, result)| {
            is_browser_action_tool(name.as_str()) && result.get("error").is_none()
        })
        && results.iter().any(|(name, result)| {
            matches!(name.as_str(), "read_browser_page" | "computer_screenshot")
                && result.get("error").is_none()
        })
}

pub(super) fn should_continue_after_actionable_lookup(results: &[(String, Value)]) -> bool {
    if results
        .iter()
        .any(|(name, _)| is_agent_action_tool(name.as_str()))
    {
        return false;
    }
    !allowed_lookup_followup_actions(results).is_empty()
}

pub(super) fn allowed_lookup_followup_actions(results: &[(String, Value)]) -> Vec<&'static str> {
    let successful = |name: &str| {
        results
            .iter()
            .any(|(result_name, result)| result_name == name && result.get("error").is_none())
    };
    let mut allowed = Vec::new();
    if successful("list_google_calendar_events") {
        allowed.push("delete_google_calendar_event");
        allowed.push("update_google_calendar_event");
    }
    if successful("list_downloaded_files") {
        allowed.push("open_downloaded_file");
        allowed.push("delete_downloaded_file");
        if !successful("read_downloaded_file") {
            allowed.push("read_downloaded_file");
        }
    }
    let has_luna_list_lookup = results.iter().any(|(name, result)| {
        matches!(name.as_str(), "list_luna_announcements" | "list_luna_todos")
            && result.get("error").is_none()
    });
    let has_luna_lookup = results.iter().any(|(name, result)| {
        matches!(
            name.as_str(),
            "get_luna_activity_detail" | "list_luna_announcements" | "list_luna_todos"
        ) && result.get("error").is_none()
    });
    if has_luna_list_lookup && !successful("get_luna_activity_detail") {
        allowed.push("get_luna_activity_detail");
    }
    if has_luna_lookup {
        allowed.push("open_copilot_page");
        allowed.push("open_luna_attachment");
        allowed.push("download_luna_attachment");
        allowed.push("download_course_material");
    }
    let has_notification_list = results.iter().any(|(name, result)| {
        matches!(
            name.as_str(),
            "list_recent_notifications" | "search_notifications"
        ) && result.get("error").is_none()
    });
    let has_notification_detail = successful("get_notification_detail");
    if has_notification_list && !has_notification_detail {
        allowed.push("get_notification_detail");
    }
    if has_notification_list || has_notification_detail {
        allowed.push("open_copilot_page");
    }
    allowed
}

pub(super) fn apply_browser_target_lock(
    tool_name: &str,
    mut args: Value,
    turn_context: &AgentTurnContext,
) -> Value {
    let Some(target) = turn_context.browser_target.as_deref() else {
        return args;
    };
    if !is_browser_target_scoped_tool(tool_name) {
        return args;
    }
    if let Value::Object(map) = &mut args {
        let old_target = map.get("target").and_then(|v| v.as_str()).unwrap_or("");
        // Allow any pane of the current split view (active tab's main webview +
        // split children). Only force the attached/active target when the request
        // has no target or points outside the current view (e.g. an unrelated
        // window), preserving the "don't wander off" guard.
        let in_current_view = !old_target.is_empty()
            && turn_context
                .view_pane_targets
                .iter()
                .any(|pane| pane == old_target);
        if old_target != target && !in_current_view {
            if !old_target.is_empty() {
                log::warn!(
                    "[agent plan] browser target locked: tool={} requested={} forced={}",
                    tool_name,
                    old_target,
                    target
                );
            }
            map.insert("target".into(), Value::String(target.to_string()));
        }
    }
    args
}

pub(super) fn finalize_plan_with_diagnostics(
    plan: Plan,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> FinalizedPlan {
    if should_skip_tools(history, user_text) {
        log::debug!(
            "[agent plan] skip_tools=true (smalltalk/followup), dropping {} tool(s)",
            plan.tools.len()
        );
        return FinalizedPlan::default();
    }
    let mut seen = HashSet::new();
    let mut unknown_tools = Vec::new();
    let mut invalid_args = Vec::new();
    let tools: Vec<ToolCall> = plan
        .tools
        .into_iter()
        .filter_map(|call| {
            let Some(name) = agent_tools::canonical_tool_name(&call.name) else {
                log::warn!("[agent plan] unknown tool dropped: {}", call.name);
                unknown_tools.push(call.name);
                return None;
            };
            if contains_unresolved_plan_placeholder(&call.args) {
                log::warn!(
                    "[agent plan] tool dropped because args contain unresolved placeholder: name={} args={}",
                    name,
                    call.args
                );
                invalid_args.push(name.to_string());
                return None;
            }
            let sanitized = agent_tools::sanitize_tool_args(name, &call.args);
            if sanitized.is_none() {
                log::warn!(
                    "[agent plan] tool dropped by sanitize: name={} args={}",
                    name,
                    call.args
                );
                invalid_args.push(name.to_string());
            }
            let args = apply_browser_target_lock(name, sanitized?, turn_context);
            let key = format!(
                "{}:{}",
                name,
                serde_json::to_string(&args).unwrap_or_default()
            );
            if !seen.insert(key) {
                return None;
            }
            Some(ToolCall {
                name: name.to_string(),
                args,
            })
        })
        .take(CFG.max_tools)
        .collect();
    FinalizedPlan {
        plan: Plan {
            tools,
            image_only: plan.image_only,
        },
        unknown_tools,
        invalid_args,
    }
}
