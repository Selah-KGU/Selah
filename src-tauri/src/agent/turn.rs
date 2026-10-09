use super::*;

// ─────────────────────── Public Entry Point ───────────────────────

#[derive(Debug, Clone, Default)]
pub struct AgentTurnContext {
    pub documents: Vec<crate::agent_attachments::DocumentPart>,
    pub has_documents: bool,
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

// The decoder owns the IPC message and runs only on the blocking worker. Its
// borrowed validation, save reservation and request identity were admitted by
// the command adapter before any async response task can be delayed.
pub(crate) fn submit_rpc_turn(
    app: AppHandle,
    conv_id: String,
    request_id: Option<String>,
    save: crate::pending_persistence::SavePermit,
    decode: impl FnOnce() -> Result<(String, Vec<ImagePart>, AgentTurnContext), AgentError>
        + Send
        + 'static,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    start_turn(app, conv_id, request_id, move || {
        let (text, images, context) = decode()?;
        Ok((input::TurnInput::New { text, images, save }, context))
    })
}

/// Admit the native request before its accepted speech save can be delayed.
/// One blocking preparation job persists speech, then resolves model/history.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn submit_voice_turn(
    app: AppHandle,
    conv_id: String,
    request_id: String,
    save: impl FnOnce() -> Result<SavedVoiceInput, String> + Send + 'static,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    let prepare_app = app.clone();
    let prepare_id = conv_id.clone();
    let admitted = admit_voice(&conv_id, request_id, save, move |input, owner| {
        Ok((
            prepare_turn(
                &prepare_app,
                &prepare_id,
                input::TurnInput::Voice(input),
                &owner,
                &[],
            )?,
            AgentTurnContext::default(),
        ))
    });
    run_admitted(app, conv_id, admitted)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn admit_voice<T: Send + 'static>(
    conversation: &str,
    request: String,
    save: impl FnOnce() -> Result<SavedVoiceInput, String> + Send + 'static,
    prepare: impl FnOnce(
            SavedVoiceInput,
            std::sync::Arc<crate::agent_turn_scope::Turn>,
        ) -> Result<T, AgentError>
        + Send
        + 'static,
) -> crate::agent_turn_scope::Admission<T> {
    crate::agent_turn_scope::Admission::start(conversation, Some(request), move |owner| {
        // Supersession and dropped async waiters must not discard accepted text.
        let input = save().map_err(AgentError::db)?;
        if owner.cancelled() {
            return Err(AgentError::Cancelled);
        }
        prepare(input, owner)
    })
}

struct PreparedTurn {
    provider: AgentProvider,
    history: Vec<crate::db::AgentMessageRow>,
    user_text: String,
    user_images: Vec<ImagePart>,
}

fn start_turn(
    app: AppHandle,
    conv_id: String,
    request_id: Option<String>,
    decode: impl FnOnce() -> Result<(input::TurnInput, AgentTurnContext), AgentError> + Send + 'static,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    let prepare_app = app.clone();
    let prepare_id = conv_id.clone();
    let admitted = crate::agent_turn_scope::Admission::start(&conv_id, request_id, move |owner| {
        let (input, mut context) = decode()?;
        let prepared = prepare_turn(&prepare_app, &prepare_id, input, &owner, &context.documents)?;
        context.has_documents = !context.documents.is_empty();
        context.documents.clear();
        Ok((prepared, context))
    });
    run_admitted(app, conv_id, admitted)
}

fn run_admitted(
    app: AppHandle,
    conv_id: String,
    mut admitted: crate::agent_turn_scope::Admission<(PreparedTurn, AgentTurnContext)>,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    let scope = admitted.running.turn.clone();
    async move {
        let result = crate::agent_turn_scope::CURRENT
            .scope(scope, async {
                let result = async {
                    let (prepared, mut context) = admitted.prepared().await?;
                    // Collect live panes after durable preparation; the async waiter
                    // owns the same admitted request and never registers a second one.
                    if context.browser_target.is_some() && context.view_pane_targets.is_empty() {
                        context.view_pane_targets =
                            crate::document_tabs::active_view_panes(&app, "document-tabs");
                    }
                    run_turn(&app, &conv_id, prepared, context).await
                }
                .await;
                match &result {
                    Ok(()) => emit(&app, &conv_id, &StreamEvent::Done),
                    Err(AgentError::Cancelled) => {
                        log::info!("[agent] turn cancelled conv_id={}", conv_id);
                        emit(&app, &conv_id, &StreamEvent::Done);
                    }
                    Err(error) => {
                        let message = error.to_string();
                        emit(&app, &conv_id, &StreamEvent::Error { message: &message });
                    }
                }
                match result {
                    Ok(()) | Err(AgentError::Cancelled) => Ok(()),
                    Err(error) => Err(error.to_string()),
                }
            })
            .await;
        admitted.running.finish();
        result
    }
}

fn prepare_turn(
    app: &AppHandle,
    conv_id: &str,
    input: input::TurnInput,
    owner: &crate::agent_turn_scope::Turn,
    documents: &[crate::agent_attachments::DocumentPart],
) -> Result<PreparedTurn, AgentError> {
    let db = app.state::<Database>();
    let (committed, provider, history) = prepare::persisted_turn(
        || {
            let mut committed = input.persist_with(conv_id, |text, images| {
                persist_user_message(app, &db, conv_id, text, images, documents)
            })?;
            if !documents.is_empty() {
                committed.text =
                    crate::agent_attachments::model_content(&committed.text, documents);
            }
            owner.set_input_message(committed.message_id)?;
            Ok(committed)
        },
        || {
            if crate::app_shutdown::is_shutting_down() || owner.cancelled() {
                return Err(AgentError::Cancelled);
            }
            AgentProvider::resolve()
        },
        || {
            db.agent_load_turn_prior_messages(
                conv_id,
                owner.history()?.input_message,
                CFG.history_window
                    .max(BROWSER_CLICK_HISTORY_ROWS.saturating_sub(1)),
            )
            .map_err(AgentError::db)
        },
    )?;
    Ok(PreparedTurn {
        provider,
        history,
        user_text: committed.text,
        user_images: committed.images,
    })
}

/// Match the issuing UI request; legacy callers may cancel the current turn.
pub fn cancel_request(conv_id: &str, request: Option<&str>) {
    crate::agent_turn_scope::cancel(conv_id, request);
}

// ─────────────────────── Turn Pipeline ───────────────────────

async fn run_turn(
    app: &AppHandle,
    conv_id: &str,
    prepared: PreparedTurn,
    mut turn_context: AgentTurnContext,
) -> Result<(), AgentError> {
    let PreparedTurn {
        provider,
        history,
        user_text,
        user_images,
    } = prepared;
    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }
    let history_slice = slice_history(&history, CFG.history_window);
    turn_context.browser_click_labels = browser_click_labels_for_turn(&history, &user_text);

    // 3. Phase 1 — plan (skip for image-only turns).
    let plan = plan_phase(
        app,
        conv_id,
        &provider,
        history_slice,
        &user_text,
        &user_images,
        &turn_context,
    )
    .await?;

    // 4. Execute tools.
    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }
    let mut tool_results = execute_tools(app, conv_id, &plan, &user_text, &turn_context).await?;
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
        let follow_history =
            persistence::load_planning_history(app, conv_id, provider.supports_vision()).await?;
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
            execute_tools(app, conv_id, &next_plan, &user_text, &turn_context).await?;
        last_batch_len = follow_results.len();
        tool_results.extend(follow_results);
        if last_batch_len == 0 {
            break;
        }
    }

    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }
    if let Some(answer) = local_browser_action_answer(&user_text, &tool_results, &turn_context) {
        emit(app, conv_id, &StreamEvent::Token { text: &answer });
        persistence::save_answer(app, conv_id, answer).await?;
        return Ok(());
    }

    // 5. Phase 2 — stream answer.
    let mut answer = answer_phase(
        app,
        conv_id,
        &provider,
        history_slice,
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
                        history_slice,
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
                    history_slice,
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
                    history_slice,
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
                    history_slice,
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
            execute_tools(app, conv_id, &follow_plan, &user_text, &turn_context).await?;
        if follow_results.is_empty() {
            break;
        }
        tool_results.extend(follow_results);
        answer = answer_phase(
            app,
            conv_id,
            &provider,
            history_slice,
            &user_text,
            &user_images,
            &tool_results,
            &turn_context,
        )
        .await?;
    }

    // 6. Persist assistant response.
    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }
    persistence::save_answer(app, conv_id, answer).await?;

    Ok(())
}

fn persist_user_message(
    app: &AppHandle,
    db: &Database,
    conv_id: &str,
    user_text: &str,
    user_images: &[ImagePart],
    documents: &[crate::agent_attachments::DocumentPart],
) -> Result<i64, AgentError> {
    let message_id = persist_user_documents(db, conv_id, user_text, user_images, documents)?;
    maybe_autotitle(db, conv_id, user_text);
    // Metadata changed even for a fixed Voice Shortcut/manual title. Publish
    // only after persistence, before a configuration error can end the turn.
    let _ = app.emit("agent-conversations-changed", conv_id);
    Ok(message_id)
}

pub(in crate::agent) fn persist_user_documents(
    db: &Database,
    conv_id: &str,
    content: &str,
    images: &[ImagePart],
    documents: &[crate::agent_attachments::DocumentPart],
) -> Result<i64, AgentError> {
    if documents.is_empty() {
        return persist_user_body(db, conv_id, content, images);
    }
    crate::agent_attachments::validate_documents(documents).map_err(AgentError::config)?;
    let saved = crate::agent_attachments::SavedDocuments {
        content: content.into(),
        documents: documents.to_vec(),
    };
    let documents_json =
        serde_json::to_string(&saved).map_err(|e| AgentError::db(e.to_string()))?;
    let images_json = if images.is_empty() {
        None
    } else {
        Some(serde_json::to_string(images).map_err(|e| AgentError::db(e.to_string()))?)
    };
    db.agent_append_document_message(
        conv_id,
        &crate::agent_attachments::model_content(content, documents),
        images_json.as_deref(),
        &documents_json,
    )
    .map_err(AgentError::db)
}

// Kept separate from title/UI notification so preparation can be verified with
// real SQLite without creating an application or resolving the user's model.
pub(in crate::agent) fn persist_user_body(
    db: &Database,
    conv_id: &str,
    user_text: &str,
    user_images: &[ImagePart],
) -> Result<i64, AgentError> {
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
    .map_err(AgentError::db)
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
#[path = "turn/voice_tests.rs"]
mod voice_tests;

#[cfg(test)]
#[path = "turn/document_tests.rs"]
mod document_tests;
