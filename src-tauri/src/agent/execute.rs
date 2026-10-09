//! Execute a planned batch of agent tools.
//!
//! Runs each tool with its timeout, records the result, and applies the
//! browser and live-note follow-ups that the planner does not spell out.

use super::*;

// ─────────────────────── Tool Execution ───────────────────────

pub(super) async fn execute_tools(
    app: &AppHandle,
    conv_id: &str,
    plan: &Plan,
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> Result<Vec<(String, Value)>, AgentError> {
    let mut results = Vec::new();
    let mut auto_read_done = false;
    let mut auto_mouse_done = false;
    let mut browser_mutation_failed = false;
    let plan_already_reads_file = plan
        .tools
        .iter()
        .any(|call| call.name == "read_downloaded_file");
    let plan_already_reads_browser_page = plan
        .tools
        .iter()
        .any(|call| call.name == "read_browser_page");
    if !plan.tools.is_empty() {
        emit(
            app,
            conv_id,
            &StreamEvent::Plan {
                steps: plan
                    .tools
                    .iter()
                    .map(|call| StreamPlanStep {
                        name: call.name.as_str(),
                        detail: plan_step_detail(call),
                    })
                    .collect(),
            },
        );
    }
    for call in plan.tools.iter().take(CFG.max_tools) {
        if AgentProvider::is_cancelled(conv_id) {
            return Err(AgentError::Cancelled);
        }
        if browser_mutation_failed && is_browser_mutation_tool(&call.name) {
            let result = json!({
                "error": "skipped because an earlier browser interaction failed; re-observe the page before another interaction",
            });
            let preview = preview_of(&result);
            log::warn!(
                "[agent tool] skipped browser interaction after earlier failure name={}",
                call.name
            );
            emit(
                app,
                conv_id,
                &StreamEvent::ToolResult {
                    name: &call.name,
                    preview: &preview,
                    ok: false,
                },
            );
            let result = persistence::save_tool_result(app, conv_id, &call.name, result).await?;
            results.push((call.name.clone(), result));
            continue;
        }
        emit(app, conv_id, &StreamEvent::ToolCall { name: &call.name });
        let started = std::time::Instant::now();
        log::debug!(
            "[agent tool] start name={} args={}",
            call.name,
            serde_json::to_string(&call.args).unwrap_or_default()
        );
        let timeout = timeout_for(&call.name);
        let result = await_tool_result(
            conv_id,
            timeout,
            agent_tools::dispatch(app, &call.name, &call.args),
        )
        .await?;
        let ok = result.get("error").is_none();
        if !ok && is_browser_mutation_tool(&call.name) {
            browser_mutation_failed = true;
        }
        let preview = preview_of(&result);
        log::debug!(
            "[agent tool] finish name={} ok={} elapsed_ms={} preview={}",
            call.name,
            ok,
            started.elapsed().as_millis(),
            truncate_for_log(&preview, 200)
        );
        emit(
            app,
            conv_id,
            &StreamEvent::ToolResult {
                name: &call.name,
                preview: &preview,
                ok,
            },
        );

        // Persist tool result.
        let result = persistence::save_tool_result(app, conv_id, &call.name, result).await?;

        results.push((call.name.clone(), result));

        if AgentProvider::is_cancelled(conv_id) {
            return Err(AgentError::Cancelled);
        }

        if !ok
            && is_browser_mutation_tool(&call.name)
            && !plan_already_reads_browser_page
            && turn_context.browser_target.is_some()
        {
            let read_args = apply_browser_target_lock("read_browser_page", json!({}), turn_context);
            let read_result =
                execute_auto_tool(app, conv_id, "read_browser_page", read_args).await?;
            results.push(("read_browser_page".into(), read_result));
            if AgentProvider::is_cancelled(conv_id) {
                return Err(AgentError::Cancelled);
            }
        }

        if !auto_mouse_done
            && call.name == "read_browser_page"
            && turn_context.browser_target.is_some()
        {
            let last_page = &results[results.len() - 1].1;
            let mouse_args = infer_mouse_click_from_observation(user_text, last_page, turn_context)
                .or_else(|| infer_tab_browse_click_from_observation(user_text, last_page));
            if let Some(mouse_args) = mouse_args {
                let mouse_args =
                    apply_browser_target_lock("computer_mouse_click", mouse_args, turn_context);
                let mouse_result =
                    execute_auto_tool(app, conv_id, "computer_mouse_click", mouse_args).await?;
                results.push(("computer_mouse_click".into(), mouse_result));
                auto_mouse_done = true;
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }

                let read_args =
                    apply_browser_target_lock("read_browser_page", json!({}), turn_context);
                let read_result =
                    execute_auto_tool(app, conv_id, "read_browser_page", read_args).await?;
                results.push(("read_browser_page".into(), read_result));
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }
            }
        }

        if !auto_mouse_done
            && call.name == "computer_screenshot"
            && turn_context.browser_target.is_some()
        {
            let screenshot_mouse_args = infer_mouse_click_from_screenshot(
                user_text,
                &results[results.len() - 1].1,
                turn_context,
            );
            let observation_mouse_args = if screenshot_mouse_args.is_none() {
                let read_args =
                    apply_browser_target_lock("read_browser_page", json!({}), turn_context);
                let read_result =
                    execute_auto_tool(app, conv_id, "read_browser_page", read_args).await?;
                let mouse_args =
                    infer_mouse_click_from_observation(user_text, &read_result, turn_context)
                        .or_else(|| {
                            infer_tab_browse_click_from_observation(user_text, &read_result)
                        });
                results.push(("read_browser_page".into(), read_result));
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }
                mouse_args
            } else {
                None
            };
            if let Some(mouse_args) = screenshot_mouse_args.or(observation_mouse_args) {
                let mouse_args =
                    apply_browser_target_lock("computer_mouse_click", mouse_args, turn_context);
                let mouse_result =
                    execute_auto_tool(app, conv_id, "computer_mouse_click", mouse_args).await?;
                results.push(("computer_mouse_click".into(), mouse_result));
                auto_mouse_done = true;
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }

                let shot_args =
                    apply_browser_target_lock("computer_screenshot", json!({}), turn_context);
                let shot_result =
                    execute_auto_tool(app, conv_id, "computer_screenshot", shot_args).await?;
                results.push(("computer_screenshot".into(), shot_result));
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }

                let read_args =
                    apply_browser_target_lock("read_browser_page", json!({}), turn_context);
                let read_result =
                    execute_auto_tool(app, conv_id, "read_browser_page", read_args).await?;
                results.push(("read_browser_page".into(), read_result));
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }
            }
        }

        if !auto_read_done
            && !plan_already_reads_file
            && should_auto_read_live_note(user_text, &call.name)
        {
            let preferred_courses = preferred_live_courses(user_text, &results);
            if let Some(path) =
                pick_live_markdown_path(&results[results.len() - 1].1, &preferred_courses)
            {
                let auto_args = json!({ "path": path });
                let auto_result =
                    execute_auto_tool(app, conv_id, "read_downloaded_file", auto_args).await?;
                results.push(("read_downloaded_file".into(), auto_result));
                auto_read_done = true;
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }
            }
        }
    }
    Ok(results)
}

async fn execute_auto_tool(
    app: &AppHandle,
    conv_id: &str,
    name: &str,
    args: Value,
) -> Result<Value, AgentError> {
    emit(app, conv_id, &StreamEvent::ToolCall { name });
    let started = std::time::Instant::now();
    log::debug!(
        "[agent tool] auto-follow name={} args={}",
        name,
        serde_json::to_string(&args).unwrap_or_default()
    );
    let timeout = timeout_for(name);
    let result =
        await_tool_result(conv_id, timeout, agent_tools::dispatch(app, name, &args)).await?;
    let ok = result.get("error").is_none();
    let preview = preview_of(&result);
    log::debug!(
        "[agent tool] finish name={} ok={} elapsed_ms={} preview={}",
        name,
        ok,
        started.elapsed().as_millis(),
        truncate_for_log(&preview, 200)
    );
    emit(
        app,
        conv_id,
        &StreamEvent::ToolResult {
            name,
            preview: &preview,
            ok,
        },
    );
    let result = persistence::save_tool_result(app, conv_id, name, result).await?;
    Ok(result)
}

// One cancellation/deadline boundary for planned and automatic tools. No 120 ms
// polling timer is needed while a tool awaits IO. Admitted writes and external
// operations that have already executed are not rolled back by dropping a waiter.
async fn await_tool_result(
    conv_id: &str,
    timeout: std::time::Duration,
    dispatch: impl std::future::Future<Output = Value>,
) -> Result<Value, AgentError> {
    let owner = crate::agent_turn_scope::current(conv_id);
    crate::agent_turn_scope::until_cancelled(owner.as_deref(), async {
        Ok(match tokio::time::timeout(timeout, dispatch).await {
            Ok(result) => result,
            Err(_) => json!({ "error": format!("tool timed out after {}s", timeout.as_secs()) }),
        })
    })
    .await
}

#[cfg(test)]
#[path = "execute/tests.rs"]
mod wait_tests;
