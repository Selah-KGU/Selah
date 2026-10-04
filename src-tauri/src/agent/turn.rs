use super::*;

// ─────────────────────── Public Entry Point ───────────────────────

#[derive(Debug, Clone, Default)]
pub struct AgentTurnContext {
    pub browser_target: Option<String>,
    pub browser_click_labels: Vec<String>,
    pub page_title: Option<String>,
    pub page_kind: Option<String>,
    /// Targets of every live pane in the current split view (active tab's main
    /// webview + split child panes). Empty = no relaxation (behaves as before).
    /// The browser target lock allows any target in this set, so the agent can
    /// read/operate on any pane of the current view, not only the active one.
    pub view_pane_targets: Vec<String>,
}

/// Called from the Tauri command layer.
pub async fn agent_send(
    app: AppHandle,
    conv_id: String,
    user_text: String,
    user_images: Vec<ImagePart>,
) -> Result<(), String> {
    agent_send_with_context(
        app,
        conv_id,
        user_text,
        user_images,
        AgentTurnContext::default(),
    )
    .await
}

/// Called from an Agent panel attached to a specific browser/detail webview.
pub async fn agent_send_with_context(
    app: AppHandle,
    conv_id: String,
    user_text: String,
    user_images: Vec<ImagePart>,
    turn_context: AgentTurnContext,
) -> Result<(), String> {
    AgentProvider::clear_cancel(&conv_id);
    let mut turn_context = turn_context;
    // Widen the browser target lock to the whole current split view: collect the
    // live pane targets of the Copilot window's active tab so the agent may read
    // and operate on any pane, not just the attached/active one.
    if turn_context.browser_target.is_some() && turn_context.view_pane_targets.is_empty() {
        turn_context.view_pane_targets =
            crate::document_tabs::active_view_panes(&app, "document-tabs");
    }
    let result = run_turn(&app, &conv_id, user_text, user_images, turn_context).await;
    AgentProvider::clear_cancel(&conv_id);
    match &result {
        Ok(()) => emit(&app, &conv_id, &StreamEvent::Done),
        Err(AgentError::Cancelled) => {
            log::info!("[agent] turn cancelled conv_id={}", conv_id);
            emit(&app, &conv_id, &StreamEvent::Done);
        }
        Err(e) => {
            let msg = e.to_string();
            emit(&app, &conv_id, &StreamEvent::Error { message: &msg });
        }
    }
    match result {
        Ok(()) | Err(AgentError::Cancelled) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Exposed for the cancel command.
pub fn cancel(conv_id: &str) {
    AgentProvider::cancel(conv_id);
}

// ─────────────────────── Turn Pipeline ───────────────────────

async fn run_turn(
    app: &AppHandle,
    conv_id: &str,
    user_text: String,
    user_images: Vec<ImagePart>,
    mut turn_context: AgentTurnContext,
) -> Result<(), AgentError> {
    let provider = AgentProvider::resolve()?;
    let db = app.state::<Database>();

    // 1. Persist user message.
    persist_user_message(app, &db, conv_id, &user_text, &user_images)?;

    // 2. Load conversation history.
    let history = db.agent_load_messages(conv_id).unwrap_or_default();
    let history_slice = slice_history(&history, CFG.history_window);
    turn_context.browser_click_labels = browser_click_labels_for_turn(&history, &user_text);

    // 3. Phase 1 — plan (skip for image-only turns).
    let plan = plan_phase(
        app,
        conv_id,
        &provider,
        &history_slice,
        &user_text,
        &user_images,
        &turn_context,
    )
    .await?;

    // 4. Execute tools.
    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }
    let mut tool_results =
        execute_tools(app, conv_id, &db, &plan, &user_text, &turn_context).await?;
    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }

    // Adaptive agent loop: after each batch of tools, feed the results (including
    // any errors and screenshots) back to the model and let it decide the NEXT
    // step — observe, retry differently, or stop. This replaces the old fixed
    // two-phase continuation so the agent can work step by step and recover from
    // problems instead of failing on the first one.
    let mut last_batch_len = tool_results.len();
    for _step in 1..CFG.max_agent_steps {
        if AgentProvider::is_cancelled(conv_id) {
            return Err(AgentError::Cancelled);
        }
        let batch_start = tool_results.len().saturating_sub(last_batch_len);
        let last_batch = &tool_results[batch_start..];
        if !agent_loop_should_continue(last_batch, &tool_results, &user_text, &turn_context) {
            break;
        }
        let follow_history = db.agent_load_messages(conv_id).unwrap_or_default();
        let next_plan = match plan_next_step(
            app,
            &provider,
            &follow_history,
            &user_text,
            conv_id,
            &turn_context,
        )
        .await
        {
            Ok(next_plan) => next_plan,
            Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
            Err(error) => {
                log::warn!("[agent loop] next-step planning failed: {}", error);
                break;
            }
        };
        // An empty plan is the model signalling it has everything it needs.
        if next_plan.tools.is_empty() {
            break;
        }
        let follow_results =
            execute_tools(app, conv_id, &db, &next_plan, &user_text, &turn_context).await?;
        last_batch_len = follow_results.len();
        tool_results.extend(follow_results);
        if last_batch_len == 0 {
            break;
        }
    }

    if let Some(answer) = local_browser_action_answer(&user_text, &tool_results, &turn_context) {
        emit(app, conv_id, &StreamEvent::Token { text: &answer });
        db.agent_append_message(conv_id, "assistant", &answer, None, None, None)
            .map_err(AgentError::db)?;
        return Ok(());
    }

    // 5. Phase 2 — stream answer.
    let mut answer = answer_phase(
        app,
        conv_id,
        &provider,
        &history_slice,
        &user_text,
        &user_images,
        &tool_results,
        &turn_context,
    )
    .await?;

    let mut handled_visible_tool_calls = HashSet::new();
    let mut answer_repairs = 0usize;
    loop {
        let raw = parse_any_raw_tool_call(&answer);
        let Some(raw_call) = raw.as_ref() else {
            if has_any_pseudo_tool_call(&answer) {
                log::warn!(
                    "[agent answer] suppressed unparsable visible pseudo tool call before persistence"
                );
                if answer_repairs < CFG.max_answer_repairs {
                    answer_repairs += 1;
                    let repair_note = pseudo_tool_repair_note(None, &answer);
                    answer = answer_phase_with_repair(
                        app,
                        conv_id,
                        &provider,
                        &history_slice,
                        &user_text,
                        &user_images,
                        &tool_results,
                        &repair_note,
                        &turn_context,
                    )
                    .await?;
                    continue;
                }
                answer = pseudo_tool_repair_failed_message(&user_text, raw.as_ref());
                emit(app, conv_id, &StreamEvent::Token { text: &answer });
            }
            break;
        };
        let Some(exact_name) = agent_tools::exact_tool_name(&raw_call.name) else {
            log::warn!(
                "[agent answer] suppressed nonexistent visible pseudo tool call before persistence: {}",
                raw_call.name
            );
            if answer_repairs < CFG.max_answer_repairs {
                answer_repairs += 1;
                let repair_note = pseudo_tool_repair_note(Some(raw_call), &answer);
                answer = answer_phase_with_repair(
                    app,
                    conv_id,
                    &provider,
                    &history_slice,
                    &user_text,
                    &user_images,
                    &tool_results,
                    &repair_note,
                    &turn_context,
                )
                .await?;
                continue;
            }
            answer = pseudo_tool_repair_failed_message(&user_text, Some(raw_call));
            emit(app, conv_id, &StreamEvent::Token { text: &answer });
            break;
        };
        let Some(args) = agent_tools::sanitize_tool_args(exact_name, &raw_call.args) else {
            log::warn!(
                "[agent answer] suppressed visible pseudo tool call with invalid args: {}",
                raw_call.name
            );
            if answer_repairs < CFG.max_answer_repairs {
                answer_repairs += 1;
                let repair_note = pseudo_tool_repair_note(Some(raw_call), &answer);
                answer = answer_phase_with_repair(
                    app,
                    conv_id,
                    &provider,
                    &history_slice,
                    &user_text,
                    &user_images,
                    &tool_results,
                    &repair_note,
                    &turn_context,
                )
                .await?;
                continue;
            }
            answer = pseudo_tool_repair_failed_message(&user_text, Some(raw_call));
            emit(app, conv_id, &StreamEvent::Token { text: &answer });
            break;
        };
        let args = apply_browser_target_lock(exact_name, args, &turn_context);
        let call = ToolCall {
            name: exact_name.to_string(),
            args,
        };
        let key = format!(
            "{}:{}",
            call.name,
            serde_json::to_string(&call.args).unwrap_or_default()
        );
        if !handled_visible_tool_calls.insert(key) {
            log::warn!(
                "[agent answer] repeated visible pseudo tool call suppressed: {}",
                call.name
            );
            if answer_repairs < CFG.max_answer_repairs {
                answer_repairs += 1;
                let repair_note = pseudo_tool_repair_note(None, &answer);
                answer = answer_phase_with_repair(
                    app,
                    conv_id,
                    &provider,
                    &history_slice,
                    &user_text,
                    &user_images,
                    &tool_results,
                    &repair_note,
                    &turn_context,
                )
                .await?;
                continue;
            }
            answer = pseudo_tool_repair_failed_message(&user_text, None);
            emit(app, conv_id, &StreamEvent::Token { text: &answer });
            break;
        }
        log::warn!(
            "[agent answer] intercepted visible pseudo tool call; executing real tool name={} args={}",
            call.name,
            serde_json::to_string(&call.args).unwrap_or_default()
        );
        let follow_plan = Plan {
            tools: vec![call],
            image_only: false,
        };
        let follow_results =
            execute_tools(app, conv_id, &db, &follow_plan, &user_text, &turn_context).await?;
        if follow_results.is_empty() {
            break;
        }
        tool_results.extend(follow_results);
        answer = answer_phase(
            app,
            conv_id,
            &provider,
            &history_slice,
            &user_text,
            &user_images,
            &tool_results,
            &turn_context,
        )
        .await?;
    }

    // 6. Persist assistant response.
    db.agent_append_message(conv_id, "assistant", &answer, None, None, None)
        .map_err(AgentError::db)?;

    Ok(())
}

fn persist_user_message(
    app: &AppHandle,
    db: &Database,
    conv_id: &str,
    user_text: &str,
    user_images: &[ImagePart],
) -> Result<(), AgentError> {
    let images_json = if user_images.is_empty() {
        None
    } else {
        serde_json::to_string(user_images).ok()
    };
    db.agent_append_message(
        conv_id,
        "user",
        user_text,
        images_json.as_deref(),
        None,
        None,
    )
    .map_err(AgentError::db)?;
    maybe_autotitle(app, db, conv_id, user_text);
    Ok(())
}
