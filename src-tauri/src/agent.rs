//! Local-only agent loop (Selah persona).
//!
//! Two-phase design:
//!   Phase 1 — Planning: asks the model to pick tools (JSON, non-streaming).
//!   Phase 2 — Answering: streams the final reply with persona + tool results.
//!
//! The on-device Apple model is unreliable at multi-turn ReAct, so we constrain
//! it to a single planning step per turn.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, Manager};

use crate::agent_error::AgentError;
use crate::agent_prompts;
use crate::agent_provider::AgentProvider;
use crate::agent_pseudo_call::{
    find_start as find_pseudo_tool_call_start, has_any as has_any_pseudo_tool_call,
    parse_any_raw as parse_any_raw_tool_call, parse_leading as parse_visible_tool_call,
    RawToolCall, ToolCall,
};
use crate::agent_text;
use crate::agent_tools;
use crate::ai::{ChatMessage, ImagePart};
use crate::db::Database;

// ─────────────────────── Date/Time Context ───────────────────────

/// Builds a one-line date/time context string in JST.
/// Used by both the planner and answer phases so the model understands
/// relative time references (今日, 明日, 来週, etc.).
fn datetime_context() -> String {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let dow = match now.weekday() {
        chrono::Weekday::Mon => "月曜日",
        chrono::Weekday::Tue => "火曜日",
        chrono::Weekday::Wed => "水曜日",
        chrono::Weekday::Thu => "木曜日",
        chrono::Weekday::Fri => "金曜日",
        chrono::Weekday::Sat => "土曜日",
        chrono::Weekday::Sun => "日曜日",
    };
    format!(
        "Today: {}-{:02}-{:02} ({}) {:02}:{:02} JST",
        now.year(),
        now.month(),
        now.day(),
        dow,
        now.hour(),
        now.minute()
    )
}

/// Returns the week offset for 明日/tomorrow.
/// If today is Sunday → tomorrow is Monday (next academic week) → offset 1.
/// Otherwise → tomorrow is still within this week → offset 0.
#[cfg(test)]
fn tomorrow_week_offset() -> i32 {
    use chrono::{Datelike, Local};
    let dow = Local::now().weekday().number_from_monday(); // 1=Mon..7=Sun
    if dow == 7 {
        1
    } else {
        0
    }
}

// ─────────────────────── Agent Configuration ───────────────────────

/// Centralised knobs for the agent pipeline.  All tuning constants in one
/// place so they can be adjusted (or overridden for tests) without hunting
/// through scattered `const` blocks.
struct AgentConfig {
    /// Max historical messages (excluding the new user turn) in Phase 2.
    history_window: usize,
    /// Max tools executed per turn.
    max_tools: usize,
    /// Temperature for Phase 1 (planning) — low for determinism.
    plan_temperature: f32,
    /// Max tokens for Phase 1 output.
    plan_max_tokens: u32,
    /// Phase 1 think budget percentage.
    plan_think_budget_pct: u32,
    /// Number of recent history turns fed into Phase 1.
    plan_history_turns: usize,
    /// Max chars for a persisted tool result summary in the planning prompt.
    plan_tool_result_chars: usize,
    /// Prefill injected into the assistant turn for Phase 1.
    plan_prefill: &'static str,
    /// Think budget percentage for Phase 2.
    answer_think_budget_pct: u32,
    /// Rough prompt token budget (chars / 3).
    prompt_token_budget: usize,
    /// Max chars for a single tool result in the answer prompt.
    tool_result_chars: usize,
    /// Max chars for recent (prior-turn) tool results in the answer prompt.
    recent_tool_result_chars: usize,
    /// Recent persisted tool results exposed as follow-up context.
    recent_tool_context: usize,
    /// Bytes shown in the tool_result event preview.
    preview_bytes: usize,
    /// Hard timeout for a single tool execution.
    tool_timeout_secs: u64,
    /// Extended timeout for slow refresh-style tools.
    slow_tool_timeout_secs: u64,
    /// Hard timeout for answer generation so the UI cannot stay in thinking forever.
    answer_timeout_secs: u64,
    /// How many times Phase 2 may retry after emitting an invalid pseudo-tool call.
    max_answer_repairs: usize,
    /// How many times Phase 1 may retry after selecting unknown/invalid tools.
    max_plan_repairs: usize,
    /// Max adaptive plan→execute→observe steps per turn (incl. the first plan).
    /// Enables "act, observe, re-plan" instead of failing on the first problem.
    max_agent_steps: usize,
}

/// Tools that are known to take much longer than `tool_timeout_secs` because
/// they hit the network across many courses. Returning a timeout for them
/// while the work continues in the background creates "failed but actually
/// succeeded" inconsistencies, so they get their own ceiling.
const SLOW_TOOLS: &[&str] = &["refresh_data", "download_url"];

fn timeout_for(tool: &str) -> std::time::Duration {
    let secs = if SLOW_TOOLS.contains(&tool) {
        CFG.slow_tool_timeout_secs
    } else {
        CFG.tool_timeout_secs
    };
    std::time::Duration::from_secs(secs)
}

const CFG: AgentConfig = AgentConfig {
    history_window: 10,
    max_tools: 6,
    plan_temperature: 0.1,
    // Give reasoning models full headroom — thinking produces better tool choices.
    plan_max_tokens: 8192,
    plan_think_budget_pct: 60,
    plan_history_turns: 8,
    plan_tool_result_chars: 900,
    plan_prefill: "{\"tools\":[",
    answer_think_budget_pct: 75,
    prompt_token_budget: 120_000,
    tool_result_chars: 7000,
    recent_tool_result_chars: 4000,
    recent_tool_context: 3,
    preview_bytes: 180,
    tool_timeout_secs: 35,
    slow_tool_timeout_secs: 120,
    answer_timeout_secs: 90,
    max_answer_repairs: 2,
    max_plan_repairs: 2,
    max_agent_steps: 8,
};

// ─────────────────────── Stream Events ───────────────────────

#[derive(Debug, Serialize)]
struct StreamPlanStep<'a> {
    name: &'a str,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent<'a> {
    Phase {
        stage: &'a str,
    },
    Plan {
        steps: Vec<StreamPlanStep<'a>>,
    },
    ToolCall {
        name: &'a str,
    },
    ToolResult {
        name: &'a str,
        preview: &'a str,
        ok: bool,
    },
    Think {
        text: &'a str,
    },
    Token {
        text: &'a str,
    },
    Done,
    Error {
        message: &'a str,
    },
}

fn emit(app: &AppHandle, conv_id: &str, ev: &StreamEvent) {
    let topic = format!("agent_stream:{}", conv_id);
    let _ = app.emit(&topic, ev);
}

fn plan_step_detail(call: &ToolCall) -> Option<String> {
    let key = match call.name.as_str() {
        "browser_click" => "text",
        "browser_fill" | "browser_select_option" => "label",
        "browser_wait_for" => "text",
        "open_browser_url" | "download_url" => "url",
        "open_copilot_page" => {
            return call
                .args
                .get("context")
                .or_else(|| call.args.get("page"))
                .and_then(|v| v.as_str())
                .map(|value| trim_to(value.trim(), 80));
        }
        "read_downloaded_file"
        | "open_downloaded_file"
        | "delete_downloaded_file"
        | "write_downloaded_text_file" => "path",
        "get_course_context" | "search_courses" => "query",
        "list_downloaded_files"
        | "search_notifications"
        | "search_mail"
        | "list_luna_announcements" => "keyword",
        "get_luna_activity_detail"
        | "open_luna_attachment"
        | "download_luna_attachment"
        | "create_google_calendar_event" => "title",
        "download_course_material" => "filename",
        "update_google_calendar_event" | "delete_google_calendar_event" => "event_id",
        _ => return None,
    };
    let raw = call.args.get(key).and_then(|v| v.as_str())?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = if key == "path" {
        raw.rsplit(['/', '\\']).next().unwrap_or(raw)
    } else if key == "url" {
        raw.split(['?', '#']).next().unwrap_or(raw)
    } else {
        raw
    };
    Some(trim_to(safe, 80))
}

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

// ─────────────────────── Phase 1: Planning ───────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
struct Plan {
    #[serde(default)]
    tools: Vec<ToolCall>,
    #[serde(default)]
    image_only: bool,
}

async fn plan_phase(
    app: &AppHandle,
    conv_id: &str,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    turn_context: &AgentTurnContext,
) -> Result<Plan, AgentError> {
    if !user_images.is_empty() {
        return Ok(Plan {
            tools: vec![],
            image_only: true,
        });
    }
    emit(app, conv_id, &StreamEvent::Phase { stage: "planning" });
    choose_plan(app, provider, history, user_text, conv_id, turn_context).await
}

async fn choose_plan(
    app: &AppHandle,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    conv_id: &str,
    turn_context: &AgentTurnContext,
) -> Result<Plan, AgentError> {
    if let Some(plan) = deterministic_preplan(history, user_text, turn_context) {
        return Ok(finalize_plan(plan, history, user_text, turn_context));
    }

    // Business intent belongs to the model planner. Keyword routing used to
    // steal ambiguous requests here (for example, "open Luna details" jumping
    // to the Luna root). Keep only attached-page controls above, where the
    // target is concrete and the operation is local to the visible page.
    match run_plan_inference(app, provider, history, user_text, conv_id, turn_context).await {
        Ok(plan) => {
            let mut finalized =
                finalize_plan_with_diagnostics(plan, history, user_text, turn_context);
            let mut repairs = 0usize;
            while finalized.has_rejections() && repairs < CFG.max_plan_repairs {
                repairs += 1;
                let repair_note = plan_repair_note(&finalized);
                log::warn!(
                    "[agent plan] retrying plan after rejected tool(s): {}",
                    repair_note
                );
                match run_plan_inference_with_note(
                    app,
                    provider,
                    history,
                    user_text,
                    conv_id,
                    Some(&repair_note),
                    turn_context,
                )
                .await
                {
                    Ok(next_plan) => {
                        finalized = finalize_plan_with_diagnostics(
                            next_plan,
                            history,
                            user_text,
                            turn_context,
                        );
                    }
                    Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                    Err(e) => {
                        log::warn!("agent plan repair failed: {}", e);
                        break;
                    }
                }
            }
            if finalized.plan.tools.is_empty()
                && should_retry_empty_plan(history, user_text, turn_context)
            {
                for attempt in 1..=CFG.max_plan_repairs {
                    let note = format!(
                        "Empty plan attempt {attempt} is not sufficient for this request because it clearly needs current data or an available tool. Select the focused tools needed to make progress."
                    );
                    match run_plan_inference_with_note(
                        app,
                        provider,
                        history,
                        user_text,
                        conv_id,
                        Some(&note),
                        turn_context,
                    )
                    .await
                    {
                        Ok(next_plan) => {
                            let next = finalize_plan_with_diagnostics(
                                next_plan,
                                history,
                                user_text,
                                turn_context,
                            );
                            if !next.plan.tools.is_empty() {
                                return Ok(next.plan);
                            }
                        }
                        Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                        Err(error) => {
                            log::warn!("[agent plan] empty-plan retry failed: {}", error)
                        }
                    }
                }
                return Ok(planner_failure_fallback(history, user_text, turn_context));
            }
            if finalized.plan.tools.is_empty() && finalized.has_rejections() {
                return Ok(planner_failure_fallback(history, user_text, turn_context));
            }
            Ok(finalized.plan)
        }
        Err(AgentError::Cancelled) => Err(AgentError::Cancelled),
        Err(e) => {
            let mut repair_note = format!(
                "The previous planning attempt failed: {e}. Return one valid tools JSON object."
            );
            for attempt in 1..=CFG.max_plan_repairs {
                log::warn!(
                    "[agent plan] retrying invalid plan attempt {}/{}: {}",
                    attempt,
                    CFG.max_plan_repairs,
                    repair_note
                );
                match run_plan_inference_with_note(
                    app,
                    provider,
                    history,
                    user_text,
                    conv_id,
                    Some(&repair_note),
                    turn_context,
                )
                .await
                {
                    Ok(plan) => {
                        let finalized =
                            finalize_plan_with_diagnostics(plan, history, user_text, turn_context);
                        if !finalized.plan.tools.is_empty() || !finalized.has_rejections() {
                            return Ok(finalized.plan);
                        }
                        repair_note = plan_repair_note(&finalized);
                    }
                    Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                    Err(next_error) => {
                        repair_note = format!(
                            "The previous planning attempt failed: {next_error}. Return one valid tools JSON object."
                        );
                    }
                }
            }
            log::warn!("agent plan repair exhausted — using safe fallback");
            Ok(planner_failure_fallback(history, user_text, turn_context))
        }
    }
}

fn deterministic_preplan(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> Option<Plan> {
    if should_skip_tools(history, user_text) {
        return Some(Plan::default());
    }
    attached_browser_control_plan(&normalize_planner_text(user_text), turn_context)
}

fn planner_failure_fallback(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> Plan {
    if should_skip_tools(history, user_text) {
        return Plan::default();
    }
    if turn_context.browser_target.is_some() {
        return finalize_plan(
            single_tool_plan("read_browser_page", json!({})),
            history,
            user_text,
            turn_context,
        );
    }
    Plan::default()
}

fn should_retry_empty_plan(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> bool {
    if should_skip_tools(history, user_text) {
        return false;
    }
    if turn_context.browser_target.is_some() {
        return true;
    }
    let norm = normalize_planner_text(user_text);
    let has_recent_tool = history.iter().rev().take(6).any(|row| row.role == "tool");
    if has_recent_tool
        && contains_any(
            &norm,
            &[
                "总结",
                "總結",
                "要約",
                "まとめ",
                "解释",
                "説明",
                "どういう意味",
                "感想",
            ],
        )
    {
        return false;
    }
    contains_any(
        &norm,
        &[
            "授業",
            "课程",
            "course",
            "時間割",
            "schedule",
            "今日",
            "今天",
            "明日",
            "明天",
            "来週",
            "下周",
            "課題",
            "レポート",
            "todo",
            "締切",
            "deadline",
            "メール",
            "mail",
            "通知",
            "お知らせ",
            "成績",
            "grade",
            "単位",
            "ファイル",
            "資料",
            "添付",
            "file",
            "luna",
            "kwic",
            "kgc",
            "ブラウザ",
            "browser",
            "ページ",
            "page",
            "http",
            "カレンダー",
            "calendar",
            "日历",
            "天気",
            "weather",
            "更新",
            "refresh",
        ],
    )
}

/// Plan the next step of the adaptive agent loop. Unlike the old fixed
/// continuations, this may return an empty plan (the model is done) and is told
/// to observe-and-adapt on failure rather than give up.
async fn plan_next_step(
    app: &AppHandle,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    conv_id: &str,
    turn_context: &AgentTurnContext,
) -> Result<Plan, AgentError> {
    let note = "You have already executed one or more tools; their results — including any errors and screenshots — are in the context above. Decide the NEXT step:\n\
        - If the request still needs work, return the focused next tool(s) for it.\n\
        - If a previous step FAILED, do NOT give up: first observe (read_browser_page, or computer_screenshot to actually see the page), then try a different approach (different selector/text, coordinates, scroll, or wait_for).\n\
        - After an action, verify it worked by re-reading or screenshotting before moving on.\n\
        - When you already have everything needed to answer, return an empty tools array to finish.\n\
        Never submit, send, delete, purchase, or take any other irreversible action unless the user explicitly asked for it.";
    let plan = run_plan_inference_with_note(
        app,
        provider,
        history,
        user_text,
        conv_id,
        Some(note),
        turn_context,
    )
    .await?;
    Ok(finalize_plan(plan, history, user_text, turn_context))
}

/// Whether the adaptive loop should ask the model for another step after the
/// just-executed batch. Keeps pure information lookups single-shot, but keeps
/// going for browser/computer operations, lookup→action follow-ups, and — the
/// key fix — recoverable failures, so the agent adapts instead of failing.
fn agent_loop_should_continue(
    last_batch: &[(String, Value)],
    all_results: &[(String, Value)],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> bool {
    let norm = normalize_planner_text(user_text);
    if turn_context.browser_target.is_some() && is_browser_operation_intent(&norm) {
        return true;
    }
    if should_continue_after_browser_observation(
        &Plan::default(),
        all_results,
        user_text,
        turn_context,
    ) || should_continue_after_actionable_lookup(all_results)
    {
        return true;
    }
    // A failed browser/computer/page-scoped tool is recoverable: let the model
    // observe and try a different approach rather than stopping here.
    last_batch.iter().any(|(name, result)| {
        result.get("error").is_some()
            && (is_browser_action_tool(name)
                || is_browser_target_scoped_tool(name)
                || name.starts_with("computer_"))
    })
}

async fn run_plan_inference(
    app: &AppHandle,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    conv_id: &str,
    turn_context: &AgentTurnContext,
) -> Result<Plan, AgentError> {
    run_plan_inference_with_note(
        app,
        provider,
        history,
        user_text,
        conv_id,
        None,
        turn_context,
    )
    .await
}

async fn run_plan_inference_with_note(
    app: &AppHandle,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    conv_id: &str,
    repair_note: Option<&str>,
    turn_context: &AgentTurnContext,
) -> Result<Plan, AgentError> {
    let supports_prefill = provider.supports_prefill();
    log::debug!(
        "[agent plan] user_text={:?} history_tool_turns={}",
        truncate_for_log(user_text, 200),
        history.iter().filter(|r| r.role == "tool").count()
    );
    let msgs = build_plan_messages_with_note(
        Some(app),
        history,
        user_text,
        supports_prefill,
        repair_note,
        turn_context,
        provider.supports_vision(),
        provider.is_local(),
    );
    let prefill = if supports_prefill {
        CFG.plan_prefill
    } else {
        ""
    };

    let plan_max_tokens = if provider.is_local() {
        crate::local_ai::APPLE_MAX_RESPONSE_TOKENS
    } else {
        CFG.plan_max_tokens
    };
    let raw = provider
        .plan(
            msgs,
            plan_max_tokens,
            CFG.plan_temperature,
            prefill,
            CFG.plan_think_budget_pct,
            conv_id,
        )
        .await?;

    log::debug!(
        "[agent plan] prefill={} raw_len={} raw={:?}",
        supports_prefill,
        raw.len(),
        truncate_for_log(&raw, 400)
    );
    let parsed = parse_plan(&raw).map_err(AgentError::model)?;
    log::debug!(
        "[agent plan] parsed tools: {:?}",
        parsed
            .tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
    );
    Ok(parsed)
}

fn truncate_for_log(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

/// Build the ChatML message list for the planner.  Pure function — does not
/// touch the model or database, so it can be unit-tested.
#[cfg(test)]
fn build_plan_messages(
    app: Option<&AppHandle>,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    supports_prefill: bool,
) -> Vec<ChatMessage> {
    build_plan_messages_with_note(
        app,
        history,
        user_text,
        supports_prefill,
        None,
        &AgentTurnContext::default(),
        false,
        false,
    )
}

/// Pull the most recent screenshot image(s) out of the persisted tool history
/// (newest first) so a vision-capable model can actually see what it just
/// captured. Returns at most `limit` images.
fn recent_screenshot_images(
    history: &[crate::db::AgentMessageRow],
    limit: usize,
) -> Vec<ImagePart> {
    let mut out = Vec::new();
    for row in history.iter().rev() {
        if row.role != "tool" {
            continue;
        }
        let Some(json) = row.tool_result_json.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(json) else {
            continue;
        };
        if let Some(img) = value.get("image") {
            if let (Some(mime), Some(data)) = (
                img.get("mime").and_then(|x| x.as_str()),
                img.get("data_base64").and_then(|x| x.as_str()),
            ) {
                out.push(ImagePart {
                    mime: mime.to_string(),
                    data_base64: data.to_string(),
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
    }
    out
}

fn build_plan_messages_with_note(
    app: Option<&AppHandle>,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    supports_prefill: bool,
    repair_note: Option<&str>,
    turn_context: &AgentTurnContext,
    vision: bool,
    local: bool,
) -> Vec<ChatMessage> {
    let mut system = if local {
        agent_prompts::apple_plan_system_prompt(&datetime_context())
    } else {
        agent_prompts::plan_system_prompt(&datetime_context(), supports_prefill)
    };
    append_browser_context(&mut system, app, turn_context, local);
    if let Some(note) = repair_note {
        system.push_str("\n\n=== INVALID PREVIOUS PLAN ===\n");
        system.push_str(note);
        system.push_str("\nRe-plan now. Use exact tool names from Available tools only.");
    }
    let mut msgs = vec![ChatMessage {
        role: "system".into(),
        content: system,
        images: Vec::new(),
    }];

    let history_turns = if local { 2 } else { CFG.plan_history_turns };
    let history_chars = if local { 180 } else { 400 };
    for row in history
        .iter()
        .rev()
        .take(history_turns)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        match row.role.as_str() {
            "user" | "assistant" => msgs.push(ChatMessage {
                role: row.role.clone(),
                content: if local {
                    local_text(&row.content, 80)
                } else {
                    trim_to(&row.content, history_chars)
                },
                images: Vec::new(),
            }),
            "tool" => {
                if let (Some(name), Some(json)) =
                    (row.tool_name.as_deref(), row.tool_result_json.as_deref())
                {
                    let summary = summarize_plan_tool_result(name, json);
                    let summary = if local {
                        let trimmed = summary.trim_start();
                        if trimmed.starts_with('{') || trimmed.starts_with('[') {
                            serde_json::from_str::<Value>(json)
                                .map(|value| render_local_json(&value, 80))
                                .unwrap_or_else(|_| local_text(&summary, 80))
                        } else {
                            local_text(&summary, 80)
                        }
                    } else {
                        summary
                    };
                    msgs.push(ChatMessage {
                        role: "assistant".into(),
                        content: format!("[tool result: {name}] {summary}"),
                        images: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }

    msgs.push(ChatMessage {
        role: "user".into(),
        content: if local {
            local_text(user_text, 400)
        } else {
            user_text.to_string()
        },
        // Attach the latest screenshot so a vision model can see the page it is
        // operating on when deciding the next step.
        images: if vision {
            recent_screenshot_images(history, 1)
        } else {
            Vec::new()
        },
    });

    // Merge consecutive same-role messages so the list is always strictly
    // alternating user/assistant. Gemini API rejects requests where two
    // consecutive content blocks have the same role; this situation arises
    // naturally when multiple tool rows from the same turn are each mapped
    // to "assistant" above.  OpenAI tolerates it, but merging is cleaner.
    let mut merged: Vec<ChatMessage> = Vec::new();
    for msg in msgs {
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role && last.role != "system" {
                last.content.push('\n');
                last.content.push_str(&msg.content);
                last.images.extend(msg.images);
                continue;
            }
        }
        merged.push(msg);
    }
    merged
}

fn summarize_plan_tool_result(name: &str, json: &str) -> String {
    let parsed: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return trim_to(json, 260),
    };
    let summary = match name {
        "list_recent_mail" => parsed.get("mails").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .take(3)
                .map(|m| {
                    format!(
                        "mail[id={}, subject={}]",
                        m.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        m.get("subject").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        "list_luna_todos" => parsed.get("todos").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .take(3)
                .map(|t| {
                    format!(
                        "todo[title={}, course={}, luna_id={}, type={}, deadline={}]",
                        t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("course").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("luna_id").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("deadline").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        "get_upcoming_deadlines" => {
            parsed
                .get("deadlines")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(3)
                        .map(|t| {
                            format!(
                                "deadline[title={}, deadline={}]",
                                t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                t.get("deadline").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "list_downloaded_files" => parsed.get("files").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .take(3)
                .map(|f| {
                    format!(
                        "file[path={}, filename={}]",
                        f.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                        f.get("filename").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        "get_course_context" => parsed.get("course").map(|course| {
            let name = course.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let materials = course
                .get("materials")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(2)
                        .map(|m| {
                            format!(
                                "material[title={}, url={}]",
                                m.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                m.get("url").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            format!("course[name={}] {}", name, materials)
        }),
        "list_browser_windows" => parsed
            .get("windows")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|w| {
                        format!(
                            "browser[target={}, type={}, title={}, url={}]",
                            w.get("target").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("url").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "read_browser_page" => {
            let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = parsed.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let headings = parsed
                .get("headings")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(2)
                        .filter_map(|h| h.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();
            Some(format!("page[title={}, url={}] {}", title, url, headings))
        }
        "computer_screenshot" => {
            let rect = parsed.get("screen_rect").unwrap_or(&Value::Null);
            let width = rect.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
            let height = rect.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
            let target = parsed.get("target").and_then(|v| v.as_str()).unwrap_or("");
            Some(format!(
                "screenshot[target={}, size={}x{}]",
                target, width, height
            ))
        }
        "browser_click"
        | "browser_mouse_click"
        | "browser_mouse_drag"
        | "computer_mouse_click"
        | "computer_mouse_drag"
        | "computer_scroll"
        | "browser_fill"
        | "browser_select_option"
        | "browser_press"
        | "browser_scroll"
        | "browser_wait_for" => {
            let action = parsed
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or(name);
            let url = parsed
                .get("current_url")
                .or_else(|| parsed.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let text = parsed
                .get("element")
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "action[name={}, text={}, url={}]",
                action, text, url
            ))
        }
        "open_browser_url" | "browser_back" | "browser_forward" | "browser_reload_page" => parsed
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| format!("browser[url={}]", url)),
        "open_copilot_page" => Some(format!(
            "copilot[page={}, title={}, target={}]",
            parsed.get("page").and_then(|v| v.as_str()).unwrap_or(""),
            parsed.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            parsed.get("target").and_then(|v| v.as_str()).unwrap_or(""),
        )),
        "search_notifications" | "list_recent_notifications" => parsed
            .get("notifications")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|n| {
                        format!(
                            "notification[source={}, identifier={}, title={}]",
                            n.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                            n.get("identifier").and_then(|v| v.as_str()).unwrap_or(""),
                            n.get("title").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "list_google_calendar_events" => {
            parsed
                .get("events")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(5)
                        .map(|e| {
                            format!(
                                "cal[id={}, title={}, date={} {}-{}]",
                                e.get("event_id").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("start_time").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("end_time").and_then(|v| v.as_str()).unwrap_or(""),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "create_google_calendar_event"
        | "delete_google_calendar_event"
        | "update_google_calendar_event" => parsed
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| format!("cal_action[{}]", s)),
        "get_today_brief" => {
            let class_count = parsed
                .get("classes")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let deadline_count = parsed
                .get("urgent_deadlines")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let first_class = parsed
                .get("classes")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "today_brief[date={}, classes={}, urgent_deadlines={}, first={}]",
                parsed.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                class_count,
                deadline_count,
                first_class,
            ))
        }
        "get_weekly_summary" => {
            let week = parsed
                .get("current_week")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = parsed
                .get("weekly_summary")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(60).collect::<String>())
                .unwrap_or_default();
            Some(format!(
                "weekly_summary[week={}, preview={}]",
                week, preview
            ))
        }
        "get_grades" => {
            let items = parsed
                .get("curriculum")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let deficit_count = items
                .iter()
                .filter(|c| c.get("deficit").and_then(|v| v.as_bool()).unwrap_or(false))
                .count();
            Some(format!(
                "grades[categories={}, deficits={}]",
                items.len(),
                deficit_count
            ))
        }
        "get_luna_activity_detail" => {
            let title = parsed
                .get("matched_title")
                .or_else(|| parsed.get("detail_title"))
                .or_else(|| parsed.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let luna_id = parsed
                .pointer("/source/luna_id")
                .or_else(|| parsed.get("luna_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let deadline = parsed
                .get("deadline")
                .or_else(|| parsed.get("period"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let attachments = parsed
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(5)
                        .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();
            let body_preview = parsed
                .get("body")
                .or_else(|| parsed.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(80).collect::<String>())
                .unwrap_or_default();
            Some(format!(
                "activity[title={}, luna_id={}, attachments={}, deadline={}, body_preview={}]",
                title, luna_id, attachments, deadline, body_preview
            ))
        }
        "list_luna_announcements" => {
            parsed
                .get("announcements")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(5)
                        .map(|a| {
                            format!(
                                "announce[title={}, luna_id={}, course={}, period={}]",
                                a.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("luna_id").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("course").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("period").and_then(|v| v.as_str()).unwrap_or(""),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "get_notification_detail" => {
            let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let source = parsed.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let body_preview = parsed
                .get("body")
                .or_else(|| parsed.get("body_html"))
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(120).collect::<String>())
                .unwrap_or_default();
            let attachment_count = parsed
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "notification_detail[source={}, title={}, attachments={}, body={}]",
                source, title, attachment_count, body_preview
            ))
        }
        "get_weather" => {
            let temp = parsed
                .get("current")
                .and_then(|c| c.get("temperature_c"))
                .and_then(|v| v.as_f64())
                .map(|t| format!("{}°C", t))
                .unwrap_or_default();
            let weather = parsed
                .get("current")
                .and_then(|c| c.get("weather"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!("weather[{} {}]", weather, temp))
        }
        "get_student_profile" => {
            let name = parsed.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let faculty = parsed.get("faculty").and_then(|v| v.as_str()).unwrap_or("");
            let dept = parsed
                .get("department")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "profile[name={}, faculty={}, dept={}]",
                name, faculty, dept
            ))
        }
        "get_mail_profile" => {
            let name = parsed
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mail = parsed.get("mail").and_then(|v| v.as_str()).unwrap_or("");
            Some(format!("mail_profile[name={}, mail={}]", name, mail))
        }
        "list_syllabus_favorites" => {
            parsed
                .get("favorites")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(3)
                        .map(|f| {
                            format!(
                                "syllabus[{}]",
                                f.get("course_title").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "list_today_classes" | "list_week_classes" => parsed
            .get("classes")
            .and_then(|v| v.as_array())
            .map(|items| {
                let label = parsed
                    .get("day_of_week")
                    .or_else(|| parsed.get("week_label"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let classes: String = items
                    .iter()
                    .take(5)
                    .map(|c| {
                        format!(
                            "[{}{}]",
                            c.get("period").and_then(|v| v.as_str()).unwrap_or(""),
                            c.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!("classes[{}] {}", label, classes)
            }),
        "get_cancellations" => {
            parsed
                .get("cancellations")
                .and_then(|v| v.as_array())
                .map(|items| {
                    let entries: String = items
                        .iter()
                        .take(3)
                        .map(|c| {
                            format!(
                                "[{} {}]",
                                c.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                                c.get("course_name").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    format!("cancellations[{}] {}", items.len(), entries)
                })
        }
        "get_makeup_classes" => {
            parsed
                .get("makeup_classes")
                .and_then(|v| v.as_array())
                .map(|items| {
                    let entries: String = items
                        .iter()
                        .take(3)
                        .map(|c| {
                            format!(
                                "[{} {}]",
                                c.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                                c.get("course_name").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    format!("makeup_classes[{}] {}", items.len(), entries)
                })
        }
        "get_room_changes" => parsed
            .get("room_changes")
            .and_then(|v| v.as_array())
            .map(|items| {
                let entries: String = items
                    .iter()
                    .take(3)
                    .map(|c| {
                        format!(
                            "[{} {} → {}]",
                            c.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                            c.get("course_name").and_then(|v| v.as_str()).unwrap_or(""),
                            c.get("room").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!("room_changes[{}] {}", items.len(), entries)
            }),
        "get_exam_timetable" => parsed.get("exams").and_then(|v| v.as_array()).map(|items| {
            let entries: String = items
                .iter()
                .take(4)
                .map(|e| {
                    format!(
                        "[{} {}]",
                        e.get("day").and_then(|v| v.as_str()).unwrap_or(""),
                        e.get("course_name").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            format!("exams[{}] {}", items.len(), entries)
        }),
        "get_registration" => {
            let year = parsed
                .get("year_semester")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let course_count = parsed
                .get("courses")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "registration[semester={}, courses={}]",
                year, course_count
            ))
        }
        "get_todo_guide" => {
            let age = parsed
                .get("generated_hours_ago")
                .and_then(|v| v.as_i64())
                .map(|h| format!("{}h ago", h))
                .unwrap_or_default();
            let priority = parsed
                .get("priority_summary")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(80).collect::<String>())
                .unwrap_or_default();
            Some(format!(
                "todo_guide[generated={}, priority={}]",
                age, priority
            ))
        }
        "refresh_data" => {
            let refreshed = parsed
                .get("refreshed")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!("refresh_data[refreshed_count={}]", refreshed))
        }
        "search_courses" => parsed
            .get("matches")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|m| {
                        format!(
                            "course[{}]",
                            m.get("display_name").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "get_course_detail" => {
            let code = parsed
                .get("kgc_code")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let plan_count = parsed
                .get("session_plan")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "course_detail[code={}, plan_sessions={}]",
                code, plan_count
            ))
        }
        _ => None,
    };
    trim_to(
        summary.as_deref().unwrap_or(json),
        CFG.plan_tool_result_chars,
    )
}

fn append_browser_context(
    system: &mut String,
    app: Option<&AppHandle>,
    turn_context: &AgentTurnContext,
    compact: bool,
) {
    let Some(app) = app else {
        return;
    };
    let active_target = turn_context.browser_target.as_deref();
    let windows = crate::webview_toolbar::list_browser_windows(app);
    if compact {
        system.push_str("\n\n=== CURRENT BROWSER ===\n");
        if let Some(active) = active_target {
            let title = turn_context.page_title.as_deref().unwrap_or("");
            system.push_str(&format!(
                "Attached target={active} title={}. Use this target for this page.\n",
                trim_to(title, 80)
            ));
        }
        for window in windows.iter().take(3) {
            system.push_str(&format!(
                "- target={} title={} url={}\n",
                window.target,
                trim_to(&window.title, 60),
                trim_to(&window.url, 80),
            ));
        }
        return;
    }
    system.push_str("\n\n=== CURRENT BROWSER WINDOWS ===\n");
    if let Some(active) = active_target {
        let title = turn_context.page_title.as_deref().unwrap_or("");
        let kind = turn_context.page_kind.as_deref().unwrap_or("");
        system.push_str(&format!(
            "ACTIVE ATTACHED TARGET: {active}\n\
             ACTIVE PAGE TITLE: {title}\n\
             ACTIVE PAGE TYPE: {kind}\n\
             The current Agent panel is attached to this exact webview. For references like \
             \"this page\", \"current page\", \"这里\", \"这个页面\", \"このページ\", or \
             \"今見ている内容\", use target=\"{active}\" exactly. Do not use another window \
             unless the user explicitly asks to operate a different named window.\n\
             IMPORTANT: When the user asks about ANYTHING shown on this page — its content, \
             course materials/教材/资料, lists, details, an item visible on screen — your FIRST \
             step is to call read_browser_page(target=\"{active}\") and answer from what it \
             returns. This attached page is the source of truth; its rendered content (e.g. a \
             Luna course's material list) is already on screen. Do NOT say you lack the data or \
             offer to fetch it from elsewhere before you have actually read this page. Prefer \
             reading this page over data/list tools when the user is clearly referring to what \
             they are currently looking at.\n"
        ));
    }
    let panes = &turn_context.view_pane_targets;
    if panes.len() > 1 {
        system.push_str(&format!(
            "\n=== CURRENT SPLIT VIEW ({n} panes side by side) ===\n\
             The user sees these {n} panes at once — they are ONE split view, not \
             separate windows. For whole-view references (\"both\", \"两边\", \"全部\", \
             \"比较\", \"この画面全体\", \"整个画面\") cover ALL of them. Read or operate \
             each pane by passing its exact target to the browser tools \
             (read_browser_page / browser_click / browser_fill / …); target is NOT \
             restricted to the active pane inside this view.\n",
            n = panes.len()
        ));
        for (idx, target) in panes.iter().enumerate() {
            let is_active = active_target == Some(target.as_str());
            let info = windows
                .iter()
                .find(|w| &w.target == target || &w.label == target);
            let (title, url, kind) = info
                .map(|w| (w.title.as_str(), w.url.as_str(), w.kind.as_str()))
                .unwrap_or(("", "", ""));
            system.push_str(&format!(
                "- pane[{}]{} target={} type={} title={} url={}\n",
                idx,
                if is_active { " (active)" } else { "" },
                target,
                kind,
                trim_to(title, 120),
                trim_to(url, 240),
            ));
        }
    }
    if windows.is_empty() {
        system.push_str("No app browser window is currently registered.\n");
        return;
    }
    system.push_str(
        "These are live app browser windows. Use target exactly when reading or operating a specific page; if only one window exists, browser tools may omit target.\n",
    );
    for window in windows.iter().take(6) {
        let active = active_target
            .map(|target| target == window.target || target == window.label)
            .unwrap_or(false);
        system.push_str(&format!(
            "- label={} target={} active={} type={} title={} url={}\n",
            window.label,
            window.target,
            active,
            window.kind,
            trim_to(&window.title, 120),
            trim_to(&window.url, 240)
        ));
    }
}

fn finalize_plan(
    plan: Plan,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> Plan {
    finalize_plan_with_diagnostics(plan, history, user_text, turn_context).plan
}

#[derive(Default)]
struct FinalizedPlan {
    plan: Plan,
    unknown_tools: Vec<String>,
    invalid_args: Vec<String>,
}

impl FinalizedPlan {
    fn has_rejections(&self) -> bool {
        !self.unknown_tools.is_empty() || !self.invalid_args.is_empty()
    }
}

fn plan_repair_note(finalized: &FinalizedPlan) -> String {
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

fn contains_unresolved_plan_placeholder(value: &Value) -> bool {
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

fn is_browser_target_scoped_tool(name: &str) -> bool {
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

fn is_browser_action_tool(name: &str) -> bool {
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

fn is_browser_mutation_tool(name: &str) -> bool {
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

fn should_continue_after_browser_observation(
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

fn should_continue_after_actionable_lookup(results: &[(String, Value)]) -> bool {
    if results
        .iter()
        .any(|(name, _)| is_agent_action_tool(name.as_str()))
    {
        return false;
    }
    !allowed_lookup_followup_actions(results).is_empty()
}

fn allowed_lookup_followup_actions(results: &[(String, Value)]) -> Vec<&'static str> {
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

fn apply_browser_target_lock(
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

fn finalize_plan_with_diagnostics(
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

// ─────────────────────── Tool Execution ───────────────────────

async fn execute_tools(
    app: &AppHandle,
    conv_id: &str,
    db: &Database,
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
            let tool_json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
            let _ = db.agent_append_message(
                conv_id,
                "tool",
                "",
                None,
                Some(&call.name),
                Some(&tool_json),
            );
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
        let dispatch = agent_tools::dispatch(app, &call.name, &call.args);
        tokio::pin!(dispatch);
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        let mut cancel_tick = tokio::time::interval(std::time::Duration::from_millis(120));
        let result = loop {
            tokio::select! {
                value = &mut dispatch => break value,
                _ = &mut deadline => {
                    break json!({
                        "error": format!("tool timed out after {}s", timeout.as_secs()),
                    });
                }
                _ = cancel_tick.tick() => {
                    if AgentProvider::is_cancelled(conv_id) {
                        return Err(AgentError::Cancelled);
                    }
                }
            }
        };
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
        let tool_json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
        let _ = db.agent_append_message(
            conv_id,
            "tool",
            "",
            None,
            Some(&call.name),
            Some(&tool_json),
        );

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
                execute_auto_tool(app, conv_id, db, "read_browser_page", read_args).await?;
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
                    execute_auto_tool(app, conv_id, db, "computer_mouse_click", mouse_args).await?;
                results.push(("computer_mouse_click".into(), mouse_result));
                auto_mouse_done = true;
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }

                let read_args =
                    apply_browser_target_lock("read_browser_page", json!({}), turn_context);
                let read_result =
                    execute_auto_tool(app, conv_id, db, "read_browser_page", read_args).await?;
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
                    execute_auto_tool(app, conv_id, db, "read_browser_page", read_args).await?;
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
                    execute_auto_tool(app, conv_id, db, "computer_mouse_click", mouse_args).await?;
                results.push(("computer_mouse_click".into(), mouse_result));
                auto_mouse_done = true;
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }

                let shot_args =
                    apply_browser_target_lock("computer_screenshot", json!({}), turn_context);
                let shot_result =
                    execute_auto_tool(app, conv_id, db, "computer_screenshot", shot_args).await?;
                results.push(("computer_screenshot".into(), shot_result));
                if AgentProvider::is_cancelled(conv_id) {
                    return Err(AgentError::Cancelled);
                }

                let read_args =
                    apply_browser_target_lock("read_browser_page", json!({}), turn_context);
                let read_result =
                    execute_auto_tool(app, conv_id, db, "read_browser_page", read_args).await?;
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
                emit(
                    app,
                    conv_id,
                    &StreamEvent::ToolCall {
                        name: "read_downloaded_file",
                    },
                );
                let auto_started = std::time::Instant::now();
                log::debug!(
                    "[agent tool] auto-follow name=read_downloaded_file args={}",
                    serde_json::to_string(&auto_args).unwrap_or_default()
                );
                let auto_timeout = timeout_for("read_downloaded_file");
                let auto_result = match tokio::time::timeout(
                    auto_timeout,
                    agent_tools::dispatch(app, "read_downloaded_file", &auto_args),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => json!({
                        "error": format!("tool timed out after {}s", auto_timeout.as_secs()),
                    }),
                };
                let auto_ok = auto_result.get("error").is_none();
                let auto_preview = preview_of(&auto_result);
                log::debug!(
                    "[agent tool] finish name=read_downloaded_file ok={} elapsed_ms={} preview={}",
                    auto_ok,
                    auto_started.elapsed().as_millis(),
                    truncate_for_log(&auto_preview, 200)
                );
                emit(
                    app,
                    conv_id,
                    &StreamEvent::ToolResult {
                        name: "read_downloaded_file",
                        preview: &auto_preview,
                        ok: auto_ok,
                    },
                );
                let auto_json = serde_json::to_string(&auto_result).unwrap_or_else(|_| "{}".into());
                let _ = db.agent_append_message(
                    conv_id,
                    "tool",
                    "",
                    None,
                    Some("read_downloaded_file"),
                    Some(&auto_json),
                );
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
    db: &Database,
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
    let result = match tokio::time::timeout(timeout, agent_tools::dispatch(app, name, &args)).await
    {
        Ok(result) => result,
        Err(_) => json!({
            "error": format!("tool timed out after {}s", timeout.as_secs()),
        }),
    };
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
    let tool_json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
    let _ = db.agent_append_message(conv_id, "tool", "", None, Some(name), Some(&tool_json));
    Ok(result)
}

fn local_browser_action_answer(
    user_text: &str,
    tool_results: &[(String, Value)],
    turn_context: &AgentTurnContext,
) -> Option<String> {
    if turn_context.browser_target.is_none()
        || !is_browser_operation_intent(&normalize_planner_text(user_text))
    {
        return None;
    }

    let (_, result) = tool_results.iter().rev().find(|(name, result)| {
        matches!(
            name.as_str(),
            "computer_mouse_click" | "browser_mouse_click" | "browser_click"
        ) && result.get("error").is_none()
    })?;
    let answer = result
        .get("current_url")
        .and_then(|v| v.as_str())
        .filter(|url| !url.trim().is_empty())
        .map(|url| format!("已点击。当前页面：{url}"))
        .unwrap_or_else(|| "已点击。".to_string());
    Some(answer)
}

fn infer_mouse_click_from_observation(
    user_text: &str,
    page: &Value,
    turn_context: &AgentTurnContext,
) -> Option<Value> {
    let norm = normalize_planner_text(user_text);
    let labels = if turn_context.browser_click_labels.is_empty() {
        requested_click_labels(&norm)?
    } else {
        turn_context.browser_click_labels.clone()
    };
    let matched = browser_observation_candidates(page)
        .into_iter()
        .filter(|item| {
            let hay = normalize_click_match_text(&item.label);
            labels
                .iter()
                .map(|label| normalize_click_match_text(label))
                .any(|label| !label.is_empty() && (hay.contains(&label) || label.contains(&hay)))
        })
        .max_by_key(|item| click_candidate_priority(item, page));
    if let Some(item) = matched {
        return Some(json!({
            "x": item.center_x,
            "y": item.center_y,
            "coordinate_space": "webview",
        }));
    }
    if labels_indicate_home(&labels) {
        if let Some(item) = top_left_click_candidate(page) {
            return Some(json!({
                "x": item.center_x,
                "y": item.center_y,
                "coordinate_space": "webview",
            }));
        }
    }
    None
}

fn click_candidate_priority(item: &BrowserClickCandidate, page: &Value) -> i64 {
    let mut score = 0_i64;
    if item.center_y <= 220 {
        score += 1_000;
    }
    if same_host(
        page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
        &item.url,
    )
    .unwrap_or(false)
    {
        score += 180;
    }
    let url = item.url.to_ascii_lowercase();
    if url.ends_with(".pdf") || url.contains(".pdf?") {
        score -= 800;
    }
    if contains_any(
        &normalize_planner_text(&item.label),
        &["申込", "予約", "login", "ログイン"],
    ) {
        score -= 500;
    }
    score - item.center_y.max(0)
}

fn infer_tab_browse_click_from_observation(user_text: &str, page: &Value) -> Option<Value> {
    let norm = normalize_planner_text(user_text);
    if !wants_to_browse_visible_tabs(&norm) {
        return None;
    }
    top_navigation_click_candidate(page).map(|item| {
        json!({
            "x": item.center_x,
            "y": item.center_y,
            "coordinate_space": "webview",
        })
    })
}

fn infer_mouse_click_from_screenshot(
    user_text: &str,
    screenshot: &Value,
    turn_context: &AgentTurnContext,
) -> Option<Value> {
    let norm = normalize_planner_text(user_text);
    let labels = if turn_context.browser_click_labels.is_empty() {
        requested_click_labels(&norm)?
    } else {
        turn_context.browser_click_labels.clone()
    };
    if !labels_indicate_home(&labels) {
        return None;
    }

    let width = screenshot
        .get("screen_rect")
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1200.0)
        .max(1.0);
    let height = screenshot
        .get("screen_rect")
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_f64())
        .unwrap_or(800.0)
        .max(1.0);
    let x = (width * 0.12).clamp(48.0, 160.0).min(width - 1.0);
    let y = (height * 0.08).clamp(32.0, 92.0).min(height - 1.0);
    Some(json!({
        "x": x.round() as i64,
        "y": y.round() as i64,
        "coordinate_space": "screenshot",
    }))
}

struct BrowserClickCandidate {
    label: String,
    url: String,
    center_x: i64,
    center_y: i64,
}

fn requested_click_labels(norm: &str) -> Option<Vec<String>> {
    if contains_any(
        norm,
        &[
            "回首页",
            "回到首页",
            "返回首页",
            "去首页",
            "进入首页",
            "回主页",
            "回到主页",
            "返回主页",
            "トップページ",
            "ホーム",
            "homepage",
            "gohome",
        ],
    ) {
        let mut labels: Vec<String> = [
            "home",
            "ホーム",
            "トップ",
            "トップページ",
            "首页",
            "主页",
            "top",
            "logo",
            "ロゴ",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        labels.sort();
        labels.dedup();
        return Some(labels);
    }

    extract_browser_click_text(norm).and_then(|text| normalized_click_labels(&text))
}

fn browser_click_labels_for_turn(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
) -> Vec<String> {
    let norm = normalize_planner_text(user_text);
    if let Some(labels) = requested_click_labels(&norm) {
        return labels;
    }
    if let Some(index) = selection_index_from_norm(&norm) {
        if let Some(labels) = recent_numbered_click_labels(history, index, &norm) {
            return labels;
        }
    }
    if !is_short_click_confirmation(&norm) {
        return Vec::new();
    }
    let recent = history
        .iter()
        .rev()
        .take(6)
        .map(|row| row.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let recent_norm = normalize_planner_text(&recent);
    if contains_any(
        &recent_norm,
        &[
            "回首页",
            "回到首页",
            "返回首页",
            "回主页",
            "回到主页",
            "返回主页",
            "主页",
            "首页",
            "ホーム",
            "トップページ",
            "homepage",
            "logo",
            "ロゴ",
        ],
    ) {
        return requested_click_labels("回到主页").unwrap_or_default();
    }
    if let Some(labels) = recent_explicit_click_labels(history, &norm) {
        return labels;
    }
    Vec::new()
}

fn recent_explicit_click_labels(
    history: &[crate::db::AgentMessageRow],
    current_norm: &str,
) -> Option<Vec<String>> {
    for row in history.iter().rev().take(10) {
        let text = row.content.trim();
        if text.is_empty() || normalize_planner_text(text) == current_norm {
            continue;
        }
        if let Some(labels) = click_labels_from_recent_text(text) {
            return Some(labels);
        }
    }
    None
}

fn selection_index_from_norm(norm: &str) -> Option<usize> {
    match norm {
        "1" | "１" | "第1" | "第一" | "第一个" | "第一個" | "选1" | "選1" | "选择1" | "選擇1"
        | "一番目" | "1番目" => Some(1),
        "2" | "２" | "第2" | "第二" | "第二个" | "第二個" | "选2" | "選2" | "选择2" | "選擇2"
        | "二番目" | "2番目" => Some(2),
        "3" | "３" | "第3" | "第三" | "第三个" | "第三個" | "选3" | "選3" | "选择3" | "選擇3"
        | "三番目" | "3番目" => Some(3),
        _ => None,
    }
}

fn recent_numbered_click_labels(
    history: &[crate::db::AgentMessageRow],
    index: usize,
    current_norm: &str,
) -> Option<Vec<String>> {
    for row in history.iter().rev().take(12) {
        if row.role != "assistant" {
            continue;
        }
        let text = row.content.trim();
        if text.is_empty() || normalize_planner_text(text) == current_norm {
            continue;
        }
        if let Some(labels) = numbered_click_labels_from_text(text, index) {
            return Some(labels);
        }
    }
    None
}

fn numbered_click_labels_from_text(text: &str, index: usize) -> Option<Vec<String>> {
    for line in text.lines() {
        if numbered_line_index(line) != Some(index) {
            continue;
        }
        let labels = extract_click_label_candidates(line);
        if !labels.is_empty() {
            return Some(labels);
        }
    }
    None
}

fn numbered_line_index(line: &str) -> Option<usize> {
    let trimmed = line
        .trim_start()
        .trim_start_matches(['*', '-', '・', '•'])
        .trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    let value = match first {
        '1' | '１' => 1,
        '2' | '２' => 2,
        '3' | '３' => 3,
        _ => return None,
    };
    let next = chars.next().unwrap_or(' ');
    if matches!(next, '.' | '．' | ')' | '）' | '、' | ':' | '：') || next.is_whitespace() {
        Some(value)
    } else {
        None
    }
}

fn click_labels_from_recent_text(text: &str) -> Option<Vec<String>> {
    let mut candidates = Vec::new();
    for line in text.lines().rev() {
        let line_norm = normalize_planner_text(line);
        if !contains_any(
            &line_norm,
            &[
                "点击",
                "點擊",
                "点",
                "クリック",
                "押して",
                "标签",
                "タブ",
                "ボタン",
                "リンク",
                "上方",
                "导航",
                "ナビ",
            ],
        ) {
            continue;
        }
        candidates.extend(extract_click_label_candidates(line));
        if !candidates.is_empty() {
            break;
        }
    }
    if candidates.is_empty() {
        candidates.extend(extract_click_label_candidates(text));
    }
    candidates.dedup();
    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

fn extract_click_label_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for value in extract_all_between_any(
        text,
        &[
            ("“", "”"),
            ("\"", "\""),
            ("「", "」"),
            ("『", "』"),
            ("`", "`"),
            ("**", "**"),
        ],
    ) {
        out.extend(normalized_click_labels(&value).unwrap_or_default());
    }
    for value in extract_markdown_link_labels(text) {
        out.extend(normalized_click_labels(&value).unwrap_or_default());
    }
    out.sort();
    out.dedup();
    out
}

fn normalized_click_labels(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if !is_meaningful_click_label(trimmed) {
        return None;
    }
    let mut labels = Vec::new();
    push_click_label_variant(&mut labels, trimmed);
    for part in extract_all_between_any(trimmed, &[("（", "）"), ("(", ")")]) {
        push_click_label_variant(&mut labels, &part);
    }
    let before_paren = trimmed.split(['（', '(']).next().unwrap_or(trimmed).trim();
    push_click_label_variant(&mut labels, before_paren);

    labels.sort();
    labels.dedup();
    if labels.is_empty() {
        None
    } else {
        Some(labels)
    }
}

fn normalize_click_match_text(s: &str) -> String {
    normalize_planner_text(s)
        .chars()
        .map(agent_tools::normalize_cjk_char)
        .collect()
}

fn push_click_label_variant(out: &mut Vec<String>, value: &str) {
    let Some(cleaned) = strip_click_label_generic_words(value) else {
        return;
    };
    let normalized = normalize_planner_text(&cleaned);
    if !normalized.is_empty() {
        out.push(normalized);
    }
}

fn strip_click_label_generic_words(text: &str) -> Option<String> {
    let mut value = text
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '「' | '」' | '『' | '』'))
        .to_string();
    for suffix in [
        "というボタン",
        "这个按钮",
        "這個按鈕",
        "的按钮",
        "的按鈕",
        "ボタン",
        "按钮",
        "按鈕",
        "button",
        "リンク",
        "链接",
        "連結",
        "link",
        "タブ",
        "标签",
        "頁籤",
        "选项卡",
        "選項卡",
        "菜单",
        "メニュー",
        "入口",
        "选项",
        "選項",
    ] {
        loop {
            let trimmed = value.trim();
            if !trimmed.ends_with(suffix) {
                break;
            }
            value = trimmed[..trimmed.len().saturating_sub(suffix.len())]
                .trim()
                .to_string();
        }
    }
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn is_meaningful_click_label(text: &str) -> bool {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if !(2..=80).contains(&count) {
        return false;
    }
    let norm = normalize_planner_text(trimmed);
    if norm.is_empty() || norm.starts_with("http") {
        return false;
    }
    if contains_any(
        &norm,
        &["标签", "頁籤", "选项卡", "タブ", "tab", "导航", "菜单"],
    ) && contains_any(&norm, &["全部", "看看", "看", "all", "一覧"])
    {
        return false;
    }
    !matches!(
        norm.as_str(),
        "标签"
            | "全部"
            | "这里"
            | "這里"
            | "この"
            | "これ"
            | "这个"
            | "這個"
            | "哪一个具体部分"
            | "哪个"
            | "哪個"
            | "点击"
            | "點擊"
            | "click"
            | "button"
    )
}

fn extract_all_between_any(s: &str, pairs: &[(&str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (open, close) in pairs {
        let mut rest = s;
        while let Some((_, tail)) = rest.split_once(open) {
            let Some((inside, next)) = tail.split_once(close) else {
                break;
            };
            let inside = inside.trim();
            if !inside.is_empty() {
                out.push(inside.to_string());
            }
            rest = next;
        }
    }
    out
}

fn extract_markdown_link_labels(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some((_, tail)) = rest.split_once('[') {
        let Some((label, after_label)) = tail.split_once("](") else {
            break;
        };
        let Some((_, next)) = after_label.split_once(')') else {
            break;
        };
        let label = label.trim();
        if !label.is_empty() {
            out.push(label.to_string());
        }
        rest = next;
    }
    out
}

fn is_short_click_confirmation(norm: &str) -> bool {
    matches!(
        norm,
        "点" | "点啊"
            | "点击"
            | "點"
            | "點啊"
            | "点吧"
            | "好"
            | "好的"
            | "执行"
            | "去点"
            | "点logo"
            | "重试"
            | "再试"
            | "再试一次"
            | "重新试"
            | "重新点击"
            | "retry"
            | "tryagain"
            | "click"
            | "doit"
            | "yes"
            | "ok"
            | "押して"
            | "クリック"
    )
}

fn browser_observation_candidates(page: &Value) -> Vec<BrowserClickCandidate> {
    let mut out = Vec::new();
    if let Some(links) = page.get("links").and_then(|v| v.as_array()) {
        for link in links {
            if let Some(candidate) = click_candidate_from_value(link, &["text", "url"]) {
                out.push(candidate);
            }
        }
    }
    let elements = page.get("interactive_elements").unwrap_or(&Value::Null);
    if let Some(buttons) = elements.get("buttons").and_then(|v| v.as_array()) {
        for button in buttons {
            if let Some(candidate) = click_candidate_from_value(button, &["text", "type"]) {
                out.push(candidate);
            }
        }
    }
    if let Some(inputs) = elements.get("inputs").and_then(|v| v.as_array()) {
        for input in inputs {
            if let Some(candidate) =
                click_candidate_from_value(input, &["label", "name", "placeholder", "value"])
            {
                out.push(candidate);
            }
        }
    }
    out
}

fn labels_indicate_home(labels: &[String]) -> bool {
    labels.iter().any(|label| {
        matches!(
            label.as_str(),
            "home"
                | "ホーム"
                | "トップ"
                | "トップページ"
                | "首页"
                | "主页"
                | "top"
                | "logo"
                | "ロゴ"
        )
    })
}

fn top_left_click_candidate(page: &Value) -> Option<BrowserClickCandidate> {
    let viewport_width = page
        .get("viewport")
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1200)
        .max(1);
    let max_x = ((viewport_width as f64) * 0.42).round() as i64;
    browser_observation_candidates(page)
        .into_iter()
        .filter(|item| item.center_x >= 0 && item.center_y >= 0)
        .filter(|item| item.center_x <= max_x && item.center_y <= 180)
        .min_by_key(|item| item.center_y * 10_000 + item.center_x)
}

fn wants_to_browse_visible_tabs(norm: &str) -> bool {
    contains_any(
        norm,
        &[
            "标签",
            "頁籤",
            "选项卡",
            "タブ",
            "tab",
            "导航",
            "ナビ",
            "菜单",
            "メニュー",
        ],
    ) && contains_any(
        norm,
        &[
            "点击",
            "点",
            "看看",
            "看",
            "全部",
            "全て",
            "すべて",
            "all",
            "一覧",
            "打开",
            "開く",
        ],
    )
}

fn top_navigation_click_candidate(page: &Value) -> Option<BrowserClickCandidate> {
    let viewport_width = page
        .get("viewport")
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1200)
        .max(1);
    browser_observation_candidates(page)
        .into_iter()
        .filter(|item| item.center_x >= 0 && item.center_y >= 0)
        .filter(|item| item.center_x <= viewport_width && item.center_y <= 220)
        .filter(|item| is_safe_navigation_candidate(item, page))
        .min_by_key(|item| item.center_y * 10_000 + item.center_x)
}

fn is_safe_navigation_candidate(item: &BrowserClickCandidate, page: &Value) -> bool {
    let label = normalize_planner_text(&item.label);
    if label.is_empty()
        || label.chars().count() > 40
        || labels_indicate_home(std::slice::from_ref(&label))
        || label.starts_with("home")
        || label.starts_with("トップ")
        || contains_any(
            &label,
            &[
                "問い合わせ",
                "お問い合わせ",
                "contact",
                "login",
                "ログイン",
                "予約",
                "申込",
                "申し込み",
                "apply",
                "submit",
            ],
        )
    {
        return false;
    }
    if item.url.is_empty() {
        return true;
    }
    let url_lower = item.url.to_ascii_lowercase();
    if !(url_lower.starts_with("http://") || url_lower.starts_with("https://")) {
        return false;
    }
    let page_url = page.get("url").and_then(|v| v.as_str()).unwrap_or("");
    same_host(page_url, &item.url).unwrap_or(true)
}

fn same_host(a: &str, b: &str) -> Option<bool> {
    let a = url::Url::parse(a).ok()?;
    let b = url::Url::parse(b).ok()?;
    Some(a.host_str() == b.host_str())
}

fn click_candidate_from_value(item: &Value, label_keys: &[&str]) -> Option<BrowserClickCandidate> {
    let rect = item.get("rect")?;
    let center_x = rect
        .get("centerX")
        .or_else(|| rect.get("center_x"))?
        .as_i64()?;
    let center_y = rect
        .get("centerY")
        .or_else(|| rect.get("center_y"))?
        .as_i64()?;
    let label = label_keys
        .iter()
        .filter_map(|key| item.get(*key).and_then(|v| v.as_str()))
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if label.trim().is_empty() {
        return None;
    }
    Some(BrowserClickCandidate {
        label,
        url: item
            .get("url")
            .or_else(|| item.get("href"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        center_x,
        center_y,
    })
}

// ─────────────────────── Phase 2: Answer ───────────────────────

#[allow(clippy::too_many_arguments)] // Phase inputs are intentionally explicit for call-site auditability.
async fn answer_phase(
    app: &AppHandle,
    conv_id: &str,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    tool_results: &[(String, Value)],
    turn_context: &AgentTurnContext,
) -> Result<String, AgentError> {
    answer_phase_with_note(
        app,
        conv_id,
        provider,
        history,
        user_text,
        user_images,
        tool_results,
        None,
        turn_context,
    )
    .await
}

#[allow(clippy::too_many_arguments)] // Mirrors answer_phase plus the repair instruction.
async fn answer_phase_with_repair(
    app: &AppHandle,
    conv_id: &str,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    tool_results: &[(String, Value)],
    repair_note: &str,
    turn_context: &AgentTurnContext,
) -> Result<String, AgentError> {
    answer_phase_with_note(
        app,
        conv_id,
        provider,
        history,
        user_text,
        user_images,
        tool_results,
        Some(repair_note),
        turn_context,
    )
    .await
}

#[allow(clippy::too_many_arguments)] // Shared answer pipeline boundary; grouping would obscure inputs.
async fn answer_phase_with_note(
    app: &AppHandle,
    conv_id: &str,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    tool_results: &[(String, Value)],
    repair_note: Option<&str>,
    turn_context: &AgentTurnContext,
) -> Result<String, AgentError> {
    if AgentProvider::is_cancelled(conv_id) {
        return Err(AgentError::Cancelled);
    }
    emit(app, conv_id, &StreamEvent::Phase { stage: "answering" });

    let messages = build_answer_messages(
        Some(app),
        history,
        user_text,
        user_images,
        tool_results,
        repair_note,
        turn_context,
        provider.supports_vision(),
        provider.is_local(),
    );
    log::debug!(
        "[agent answer] start conv_id={} messages={} tool_results={}",
        conv_id,
        messages.len(),
        tool_results.len()
    );

    let gen_id = conv_id.to_string();
    let visible_chars = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let visible_guard = std::sync::Arc::new(std::sync::Mutex::new(VisibleAnswerGuard::new(
        app.clone(),
        conv_id.to_string(),
        visible_chars.clone(),
    )));
    let visible_guard_for_cb = visible_guard.clone();

    let answer_future = provider.answer(
        messages,
        &gen_id,
        CFG.answer_think_budget_pct,
        move |chunk: &str, is_think: bool| {
            if let Ok(mut guard) = visible_guard_for_cb.lock() {
                guard.feed(chunk, is_think);
            }
        },
    );
    let answer = match tokio::time::timeout(
        std::time::Duration::from_secs(CFG.answer_timeout_secs),
        answer_future,
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            if let Ok(mut guard) = visible_guard.lock() {
                guard.flush();
            }
            return Err(AgentError::model(format!(
                "AI応答が{}秒でタイムアウトしました。もう一度送信してください。",
                CFG.answer_timeout_secs
            )));
        }
    };
    if let Ok(mut guard) = visible_guard.lock() {
        guard.flush();
    }
    if visible_chars.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        let cleaned = agent_text::strip_think(&answer).trim().to_string();
        if !cleaned.is_empty() {
            if has_any_pseudo_tool_call(&cleaned) {
                log::warn!(
                    "[agent answer] no visible token was streamed; deferring/suppressing pseudo tool call"
                );
            } else {
                log::warn!(
                    "[agent answer] no visible token was streamed; emitting cleaned final answer chars={}",
                    cleaned.len()
                );
                emit(app, conv_id, &StreamEvent::Token { text: &cleaned });
            }
        }
    }
    log::debug!(
        "[agent answer] finish conv_id={} chars={} empty={}",
        conv_id,
        answer.len(),
        answer.trim().is_empty()
    );
    Ok(answer)
}

fn pseudo_tool_repair_note(raw: Option<&RawToolCall>, answer: &str) -> String {
    let visible = agent_text::strip_think(answer);
    let snippet = trim_to(visible.trim(), 700);
    let raw_summary = raw
        .map(|call| {
            format!(
                "raw tool name: {}; raw args: {}",
                call.name,
                serde_json::to_string(&call.args).unwrap_or_else(|_| "{}".into())
            )
        })
        .unwrap_or_else(|| "raw tool name: unknown or repeated".to_string());
    format!(
        "Your previous visible answer attempted an invalid or repeated tool call. \
         {raw_summary}. This answer was not shown to the user. Re-answer now in \
         natural language only. Do not print tool names, JSON, pseudo-call syntax, \
         or any call/tool block. Use only facts from the provided tool results. If \
         the requested action was not completed, say that naturally and ask for the \
         missing target. Previous hidden answer snippet: {snippet}"
    )
}

fn pseudo_tool_repair_failed_message(user_text: &str, _raw: Option<&RawToolCall>) -> String {
    if contains_any(
        user_text,
        &["吗", "你", "打开", "看看", "中文", "为什么", "工具"],
    ) {
        "模型连续尝试调用不存在或无效的工具，我已经拦截，没有把伪工具内容显示出来。请再说一次要打开或检查的目标。".to_string()
    } else if contains_any(user_text, &["して", "開いて", "見て", "なぜ", "ツール"]) {
        "存在しない、または無効なツール呼び出しを連続で検出したため、表示せずに止めました。開く対象や確認したい内容をもう一度指定してください。".to_string()
    } else {
        "The model repeatedly tried to call a nonexistent or invalid tool, so I blocked it from being shown. Please restate the page or action you want.".to_string()
    }
}

enum VisibleStreamMode {
    Pass,
    SuppressPseudoCall,
}

const VISIBLE_PSEUDO_HOLD_CHARS: usize = 64;

struct VisibleAnswerGuard {
    app: AppHandle,
    conv_id: String,
    visible_chars: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    mode: VisibleStreamMode,
    buffer: String,
}

impl VisibleAnswerGuard {
    fn new(
        app: AppHandle,
        conv_id: String,
        visible_chars: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        Self {
            app,
            conv_id,
            visible_chars,
            mode: VisibleStreamMode::Pass,
            buffer: String::new(),
        }
    }

    fn feed(&mut self, chunk: &str, is_think: bool) {
        if chunk.is_empty() {
            return;
        }
        if is_think {
            self.emit(StreamEvent::Think { text: chunk });
            return;
        }
        if matches!(self.mode, VisibleStreamMode::SuppressPseudoCall) {
            return;
        }
        self.buffer.push_str(chunk);
        self.drain_visible_buffer(false);
    }

    fn flush(&mut self) {
        self.drain_visible_buffer(true);
    }

    fn drain_visible_buffer(&mut self, complete: bool) {
        if matches!(self.mode, VisibleStreamMode::SuppressPseudoCall) {
            self.buffer.clear();
            return;
        }

        if let Some(idx) = find_pseudo_tool_call_start(&self.buffer) {
            if idx > 0 {
                let safe_prefix = self.buffer[..idx].trim_end().to_string();
                if !safe_prefix.is_empty() {
                    self.emit_visible(&safe_prefix);
                }
            }
            log::warn!("[agent answer] suppressing streamed pseudo tool call before UI emission");
            self.buffer.clear();
            self.mode = VisibleStreamMode::SuppressPseudoCall;
            return;
        }

        let emit_len = if complete {
            self.buffer.len()
        } else {
            safe_visible_emit_len(&self.buffer, VISIBLE_PSEUDO_HOLD_CHARS)
        };
        if emit_len == 0 {
            return;
        }
        let visible = self.buffer[..emit_len].to_string();
        self.buffer.drain(..emit_len);
        self.emit_visible(&visible);
    }

    fn emit_visible(&mut self, text: &str) {
        self.visible_chars
            .fetch_add(text.chars().count(), std::sync::atomic::Ordering::Relaxed);
        self.emit(StreamEvent::Token { text });
    }

    fn emit(&self, ev: StreamEvent<'_>) {
        let topic = format!("agent_stream:{}", self.conv_id);
        let _ = self.app.emit(&topic, &ev);
    }
}

fn safe_visible_emit_len(s: &str, hold_chars: usize) -> usize {
    let char_count = s.chars().count();
    if char_count <= hold_chars {
        return 0;
    }
    s.char_indices()
        .nth(char_count - hold_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

#[cfg(test)]
enum VisibleStart {
    Normal,
    MaybePseudoCall,
    PseudoCall,
}

#[cfg(test)]
fn classify_visible_stream_start(buffer: &str) -> VisibleStart {
    let trimmed = buffer.trim_start();
    if trimmed.is_empty() {
        return VisibleStart::MaybePseudoCall;
    }
    if crate::agent_pseudo_call::starts_with(trimmed) {
        return VisibleStart::PseudoCall;
    }
    if crate::agent_pseudo_call::maybe_starts_with_prefix(trimmed) {
        return VisibleStart::MaybePseudoCall;
    }
    VisibleStart::Normal
}

#[allow(clippy::too_many_arguments)] // Prompt assembly keeps each independent input visible.
fn build_answer_messages(
    app: Option<&AppHandle>,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    tool_results: &[(String, Value)],
    repair_note: Option<&str>,
    turn_context: &AgentTurnContext,
    vision: bool,
    local: bool,
) -> Vec<ChatMessage> {
    // Apple Intelligence shares one 4096-token window between the prompt and the reply.
    let mut budget = if local {
        crate::local_ai::APPLE_PROMPT_TOKEN_BUDGET
    } else {
        CFG.prompt_token_budget
    };
    let token_cost = |text: &str| -> usize {
        if local {
            crate::local_ai::estimate_apple_tokens(text)
        } else {
            estimate_tokens(text)
        }
    };
    let tool_result_chars = if local { 420 } else { CFG.tool_result_chars };
    let recent_chars = if local {
        280
    } else {
        CFG.recent_tool_result_chars
    };
    let history_chars = if local { 220 } else { 1200 };
    let user_content = if local {
        local_text(user_text, 400)
    } else {
        user_text.to_string()
    };

    // ── System prompt: persona + date + tool results ──
    let mut system = String::from(agent_prompts::PERSONA_PROMPT);
    system.push_str(&format!(
        "\n\n=== CURRENT DATE/TIME ===\n{}\n",
        datetime_context()
    ));
    system.push_str(agent_prompts::answer_tool_usage_section());
    if local {
        system.push_str(
            "\n\n=== AVAILABLE TOOLS REFERENCE (READ-ONLY) ===\n\
             Tool execution is already finished. Use only the results below. Do not invent tool names.\n",
        );
    } else {
        system.push_str("\n\n=== AVAILABLE TOOLS REFERENCE (READ-ONLY) ===\n");
        system.push_str(
            "These exact tool names/signatures exist, but this answer phase cannot execute new tools. \
             Use this only to avoid inventing capabilities or fake tool names.\n",
        );
        system.push_str(agent_tools::tool_catalog_prompt());
    }
    append_browser_context(&mut system, app, turn_context, local);

    if !tool_results.is_empty() {
        system.push_str("\n\n<tool_results>\n");
        let result_limit = if local { 3 } else { tool_results.len() };
        for (name, value) in tool_results.iter().take(result_limit) {
            let sanitized = sanitize_answer_tool_result(value);
            let rendered = if local {
                render_local_json(&sanitized, 180)
            } else {
                let json_str = serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".into());
                trim_to(&json_str, tool_result_chars)
            };
            system.push_str(&format!("[{}] {}\n", name, rendered));
        }
        system.push_str("</tool_results>\n");
    }

    let current_names: HashSet<&str> = tool_results.iter().map(|(n, _)| n.as_str()).collect();
    let recent_limit = if local { 1 } else { CFG.recent_tool_context };
    let recent: Vec<(String, String)> = recent_tool_results(history, recent_limit)
        .into_iter()
        .filter(|(name, _)| !current_names.contains(name.as_str()))
        .collect();
    if !recent.is_empty() {
        system.push_str("\n<recent_tool_results>\n");
        for (name, json) in &recent {
            let rendered = if local {
                match serde_json::from_str::<Value>(json) {
                    Ok(value) => render_local_json(&sanitize_answer_tool_result(&value), 120),
                    Err(_) => local_text(json, 120),
                }
            } else {
                let sanitized = serde_json::from_str::<Value>(json)
                    .map(|v| sanitize_answer_tool_result(&v))
                    .unwrap_or_else(|_| Value::String(trim_to(json, recent_chars)));
                let safe_json = serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".into());
                trim_to(&safe_json, recent_chars)
            };
            system.push_str(&format!("[{}] {}\n", name, rendered));
        }
        system.push_str("</recent_tool_results>\n");
    }

    if !user_images.is_empty() && !vision {
        system.push_str(
            "\n[IMAGE NOTICE] The user sent an image, but the current model cannot see images.\n\
             Briefly say you cannot view images yet and ask for a text description.\n\
             Do not guess image contents. Do not add unrelated topics.\n",
        );
    }

    if let Some(note) = repair_note {
        system.push_str("\n\n=== REPAIR INSTRUCTION ===\n");
        system.push_str(note);
        system.push('\n');
    }

    if local {
        let system_budget = crate::local_ai::APPLE_PROMPT_TOKEN_BUDGET
            .saturating_sub(token_cost(&user_content))
            .saturating_sub(32)
            .max(128);
        if token_cost(&system) > system_budget {
            system = crate::local_ai::trim_apple_text(&system, system_budget);
        }
    }

    budget = budget.saturating_sub(token_cost(&system));
    budget = budget.saturating_sub(token_cost(&user_content));

    let mut msgs = vec![ChatMessage {
        role: "system".into(),
        content: system,
        images: Vec::new(),
    }];

    // ── History: budget-aware, newest-first selection ──
    let mut history_msgs: Vec<ChatMessage> = Vec::new();
    for row in history.iter().rev() {
        if row.role != "user" && row.role != "assistant" {
            continue;
        }
        let content = if local {
            local_text(&row.content, 80)
        } else {
            trim_to(&row.content, history_chars)
        };
        let cost = token_cost(&content) + 10; // overhead for role/tags
        if budget < cost {
            break;
        }
        budget -= cost;
        history_msgs.push(ChatMessage {
            role: row.role.clone(),
            content,
            images: Vec::new(),
        });
    }
    history_msgs.reverse();
    msgs.extend(history_msgs);

    let mut images = user_images.to_vec();
    if vision {
        // Let a vision model see the latest screenshot when forming the answer.
        images.extend(recent_screenshot_images(history, 1));
    }
    msgs.push(ChatMessage {
        role: "user".into(),
        content: user_content,
        images,
    });

    msgs
}

/// Conservative token estimate: ~3 bytes per token for mixed CJK/ASCII text.
fn estimate_tokens(text: &str) -> usize {
    text.len() / 3 + 1
}

fn recent_tool_results(
    history: &[crate::db::AgentMessageRow],
    limit: usize,
) -> Vec<(String, String)> {
    history
        .iter()
        .rev()
        .filter_map(|row| {
            if row.role != "tool" {
                return None;
            }
            Some((row.tool_name.clone()?, row.tool_result_json.clone()?))
        })
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn sanitize_answer_tool_result(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if matches!(
                    key.as_str(),
                    "download_action"
                        | "download_params"
                        | "object_name"
                        | "action"
                        | "_cid"
                        | "form_params"
                        | "data_base64"
                ) {
                    continue;
                }
                out.insert(key.clone(), sanitize_answer_tool_result(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(sanitize_answer_tool_result)
                .collect::<Vec<_>>(),
        ),
        Value::String(s) => Value::String(neutralize_tool_call_syntax(s)),
        _ => value.clone(),
    }
}

fn neutralize_tool_call_syntax(s: &str) -> String {
    agent_text::neutralize_pseudo_tool_calls(s)
}

// ─────────────────────── Heuristic Planner ───────────────────────
//
// Table-driven keyword matching for unambiguous intents.  Falls through to the
// model when no rule matches.  This avoids a model round-trip for the most
// common queries and is cheaper than 20+ if-else branches.

#[cfg(test)]
struct HeuristicRule {
    keywords: &'static [&'static str],
    /// Extra keywords that must ALSO match (empty = no extra requirement).
    requires: &'static [&'static str],
    tool: &'static str,
    args: fn() -> Value,
}

#[cfg(test)]
const HEURISTIC_RULES: &[HeuristicRule] = &[
    HeuristicRule {
        keywords: &["天気", "weather", "天气"],
        requires: &[],
        tool: "get_weather",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "今天怎么过",
            "今日どう",
            "今日のまとめ",
            "今日の予定",
            "今日のブリーフ",
            "todaysummary",
            "todaybrief",
            "今天有什么安排",
            "一日の流れ",
        ],
        requires: &[],
        tool: "get_today_brief",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["今日の授業", "今天的课", "todayclasses", "todayclass"],
        requires: &[],
        tool: "list_today_classes",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["成績", "grade", "成绩", "単位", "学分"],
        requires: &[],
        tool: "get_grades",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["履修", "registration", "选课"],
        requires: &[],
        tool: "get_registration",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["休講", "停课", "cancelledclass"],
        requires: &[],
        tool: "get_cancellations",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["補講", "makeupclass", "补课"],
        requires: &[],
        tool: "get_makeup_classes",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["教室変更", "roomchange", "换教室"],
        requires: &[],
        tool: "get_room_changes",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["試験時間割", "examtimetable", "考试时间", "考试安排"],
        requires: &[],
        tool: "get_exam_timetable",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["週間サマリー", "weeklysummary", "周总结", "这周总结"],
        requires: &[],
        tool: "get_weekly_summary",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "学生情報",
            "学籍番号",
            "studentprofile",
            "学部",
            "学科",
            "个人资料",
        ],
        requires: &[],
        tool: "get_student_profile",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["お気に入りシラバス", "bookmarksyllabus", "收藏课程"],
        requires: &[],
        tool: "list_syllabus_favorites",
        args: || json!({ "limit": 10 }),
    },
    // Schedule with week offset
    HeuristicRule {
        keywords: &["来週", "nextweek", "下周"],
        requires: &["授業", "课程", "時間割", "课表", "时间", "schedule"],
        tool: "list_week_classes",
        args: || json!({ "offset": 1 }),
    },
    HeuristicRule {
        keywords: &["今週", "thisweek", "本周", "这周"],
        requires: &["授業", "课程", "時間割", "课表", "时间", "schedule"],
        tool: "list_week_classes",
        args: || json!({ "offset": 0 }),
    },
    // Mail
    HeuristicRule {
        keywords: &[
            "メールアドレス",
            "メールアカウント",
            "mail address",
            "邮箱账号",
        ],
        requires: &[],
        tool: "get_mail_profile",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["メール", "mail", "邮箱", "收件箱", "受信"],
        requires: &[],
        tool: "list_recent_mail",
        args: || json!({ "limit": 10 }),
    },
    HeuristicRule {
        keywords: &["お知らせ", "通知", "notification", "公告"],
        requires: &[],
        tool: "list_recent_notifications",
        args: || json!({ "limit": 10 }),
    },
    HeuristicRule {
        keywords: &[
            "pdf",
            "docx",
            "ファイル",
            "附件",
            "添付",
            "ダウンロード",
            "文件",
            "笔记",
            "ノート",
            "live",
            "ライブ",
        ],
        requires: &[],
        tool: "list_downloaded_files",
        args: || json!({ "limit": 10 }),
    },
    HeuristicRule {
        keywords: &[
            "ブラウザ",
            "webview",
            "网页",
            "网页内容",
            "ページ",
            "url",
            "リンク先",
            "website",
            "webpage",
        ],
        requires: &[],
        tool: "list_browser_windows",
        args: || json!({}),
    },
    // Tasks
    HeuristicRule {
        keywords: &[
            "レポート",
            "課題",
            "未提出",
            "report",
            "assignment",
            "作业",
            "报告",
        ],
        requires: &[],
        tool: "list_luna_todos",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["締め切り", "期限", "deadline", "截止", "いつまで", "due"],
        requires: &[],
        tool: "get_upcoming_deadlines",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "学習ガイド",
            "勉強計画",
            "studyplan",
            "学习计划",
            "やるべきこと",
            "怎么学",
            "どう取り組む",
            "アドバイス",
            "建议",
            "todo分析",
        ],
        requires: &[],
        tool: "get_todo_guide",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "最新化",
            "再同期",
            "强制刷新",
            "refreshdata",
            "更新して",
            "同步一下",
            "重新获取",
            "最新取得",
        ],
        requires: &[],
        tool: "refresh_data",
        args: || json!({}),
    },
    // Google Calendar — list only (create/edit/delete require model to extract args)
    HeuristicRule {
        keywords: &[
            "カレンダー一覧",
            "登録したイベント",
            "登録済みイベント",
            "calendarlist",
            "日历列表",
            "已添加的日历",
            "日历事件列表",
            "listcalendar",
        ],
        requires: &[],
        tool: "list_google_calendar_events",
        args: || json!({}),
    },
];

#[cfg(test)]
fn heuristic_plan(history: &[crate::db::AgentMessageRow], user_text: &str) -> Option<Plan> {
    if should_skip_tools(history, user_text) {
        return Some(Plan::default());
    }

    let norm = normalize_planner_text(user_text);

    if is_browser_operation_intent(&norm) {
        return None;
    }

    if has_multiple_tool_domains(&norm) {
        return None;
    }

    if let Some(plan) = campus_browser_plan(&norm) {
        return Some(plan);
    }

    if let Some(path) = recent_downloaded_file_path(history) {
        if contains_any(
            &norm,
            &[
                "看看",
                "看一下",
                "看看内容",
                "内容",
                "总结",
                "總結",
                "summary",
                "要点",
                "重點",
                "写了什么",
                "寫了什麼",
                "说了什么",
                "說了什麼",
                "読んで",
                "読んでみて",
                "見て",
                "中身",
                "内容みて",
                "何が書いてある",
                "ppt",
                "pdf",
                "doc",
                "docx",
            ],
        ) {
            return Some(single_tool_plan(
                "read_downloaded_file",
                json!({ "path": path }),
            ));
        }
        if contains_any(&norm, &["打开", "打開", "開いて", "open"]) {
            return Some(single_tool_plan(
                "open_downloaded_file",
                json!({ "path": path }),
            ));
        }
    }

    // Table-driven matching.
    for rule in HEURISTIC_RULES {
        if !contains_any(&norm, rule.keywords) {
            continue;
        }
        if !rule.requires.is_empty() && !contains_any(&norm, rule.requires) {
            continue;
        }
        return Some(single_tool_plan(rule.tool, (rule.args)()));
    }

    if contains_any(
        &norm,
        &[
            "重新连接",
            "重新連接",
            "再接続",
            "reconnect",
            "retry",
            "重新试试",
            "重新試試",
        ],
    ) && !contains_any(
        &norm,
        &[
            "課題",
            "レポート",
            "mail",
            "メール",
            "通知",
            "授業",
            "课程",
            "course",
            "资料",
            "資料",
        ],
    ) {
        return Some(single_tool_plan("refresh_data", json!({})));
    }

    // "明日" / "明天" / "tomorrow" — needs dynamic offset based on day of week.
    if contains_any(&norm, &["明日", "明天", "tomorrow"]) {
        return Some(single_tool_plan(
            "list_week_classes",
            json!({ "offset": tomorrow_week_offset() }),
        ));
    }

    // KGC code extraction (structural, not keyword-based).
    if let Some(code) = extract_kgc_code(user_text) {
        if contains_any(
            &norm,
            &[
                "授業計画",
                "教材",
                "教科書",
                "詳細",
                "syllabus",
                "detail",
                "textbook",
            ],
        ) {
            return Some(single_tool_plan(
                "get_course_detail",
                json!({ "kgc_code": code }),
            ));
        }
    }

    None // Fall through to model inference.
}

#[cfg(test)]
fn has_multiple_tool_domains(norm: &str) -> bool {
    const DOMAINS: &[&[&str]] = &[
        &["メール", "mail", "邮件", "郵件"],
        &[
            "課題",
            "レポート",
            "todo",
            "task",
            "作业",
            "作業",
            "締切",
            "deadline",
        ],
        &["授業", "時間割", "schedule", "class", "课程", "上课"],
        &["成績", "grade", "成绩", "単位", "credit"],
        &["お知らせ", "通知", "notification"],
        &["ファイル", "資料", "添付", "file", "attachment", "文件"],
        &["天気", "weather", "天气"],
        &["カレンダー", "calendar", "日历", "日程"],
        &["luna", "ルナ"],
        &["kwic"],
        &["kgcourse", "kgc"],
    ];
    DOMAINS
        .iter()
        .filter(|markers| contains_any(norm, markers))
        .take(2)
        .count()
        >= 2
}

fn is_browser_operation_intent(norm: &str) -> bool {
    contains_any(
        norm,
        &[
            "点击",
            "點擊",
            "点一下",
            "点开",
            "押して",
            "クリック",
            "click",
            "填写",
            "填",
            "fill",
            "typeinto",
            "入力",
            "入力して",
            "submit",
            "送信",
            "提出して",
            "选择",
            "選択",
            "選んで",
            "select",
            "choose",
            "保存",
            "save",
            "決定",
            "scroll",
            "スクロール",
            "拖拽",
            "拖动",
            "ドラッグ",
            "drag",
            "mouse",
            "鼠标",
            "マウス",
        ],
    )
}

fn attached_browser_control_plan(norm: &str, turn_context: &AgentTurnContext) -> Option<Plan> {
    turn_context.browser_target.as_ref()?;
    if !turn_context.browser_click_labels.is_empty() {
        return Some(single_tool_plan("computer_screenshot", json!({})));
    }

    if contains_any(
        norm,
        &[
            "回首页",
            "回到首页",
            "返回首页",
            "去首页",
            "进入首页",
            "回主页",
            "回到主页",
            "返回主页",
            "トップページ",
            "ホーム",
            "home page",
            "homepage",
            "go home",
        ],
    ) {
        return Some(single_tool_plan("computer_screenshot", json!({})));
    }

    if contains_any(norm, &["返回", "后退", "戻って", "戻る", "back"]) {
        return Some(browser_nav_then_read_plan("browser_back"));
    }
    if contains_any(norm, &["前进", "進む", "forward"]) {
        return Some(browser_nav_then_read_plan("browser_forward"));
    }
    if contains_any(norm, &["刷新页面", "重载", "リロード", "reload page"]) {
        return Some(browser_nav_then_read_plan("browser_reload_page"));
    }

    if contains_any(norm, &["往下", "下に", "scroll down", "向下"]) {
        return Some(computer_scroll_then_observe_plan(-900));
    }
    if contains_any(norm, &["往上", "上に", "scroll up", "向上"]) {
        return Some(computer_scroll_then_observe_plan(900));
    }

    if is_browser_operation_intent(norm) {
        return Some(single_tool_plan("read_browser_page", json!({})));
    }

    None
}

fn browser_nav_then_read_plan(tool: &str) -> Plan {
    Plan {
        tools: vec![
            ToolCall {
                name: tool.to_string(),
                args: json!({}),
            },
            ToolCall {
                name: "read_browser_page".into(),
                args: json!({}),
            },
        ],
        image_only: false,
    }
}

fn computer_scroll_then_observe_plan(delta_y: i64) -> Plan {
    Plan {
        tools: vec![
            ToolCall {
                name: "computer_scroll".into(),
                args: json!({ "delta_y": delta_y }),
            },
            ToolCall {
                name: "computer_screenshot".into(),
                args: json!({}),
            },
            ToolCall {
                name: "read_browser_page".into(),
                args: json!({}),
            },
        ],
        image_only: false,
    }
}

fn extract_browser_click_text(norm: &str) -> Option<String> {
    let quoted = extract_between_any(
        norm,
        &[("“", "”"), ("\"", "\""), ("「", "」"), ("『", "』")],
    );
    if quoted.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        return quoted;
    }

    for marker in [
        "点击",
        "點擊",
        "点一下",
        "点开",
        "押して",
        "クリック",
        "click",
    ] {
        if let Some((_, tail)) = norm.split_once(marker) {
            let text = tail
                .trim_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '：' | ',' | '，'))
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ");
            if (2..=80).contains(&text.chars().count()) {
                return Some(text);
            }
        }
    }

    None
}

fn extract_between_any(s: &str, pairs: &[(&str, &str)]) -> Option<String> {
    for (open, close) in pairs {
        let Some((_, tail)) = s.split_once(open) else {
            continue;
        };
        let Some((inside, _)) = tail.split_once(close) else {
            continue;
        };
        let inside = inside.trim();
        if !inside.is_empty() {
            return Some(inside.to_string());
        }
    }
    None
}

#[cfg(test)]
fn campus_browser_plan(norm: &str) -> Option<Plan> {
    if is_browser_operation_intent(norm) {
        return None;
    }

    // Distinguish "open Luna (the site root)" from "see the Luna *detail / content
    // / materials*" — the latter is about the page the user is already looking at,
    // not the portal root. When the request names specific content, fall through
    // to the model so it reads the current page (read_browser_page) or uses a
    // detail tool, instead of hard-jumping to the site root.
    if contains_any(
        norm,
        &[
            "详情",
            "詳細",
            "詳细",
            "detail",
            "内容",
            "中身",
            "なかみ",
            "教材",
            "资料",
            "資料",
        ],
    ) {
        return None;
    }

    if !contains_any(
        norm,
        &[
            "打开",
            "打開",
            "看看",
            "看一下",
            "浏览",
            "開いて",
            "開く",
            "見て",
            "open",
            "browser",
        ],
    ) {
        return None;
    }

    if contains_any(norm, &["luna", "ルナ"]) {
        return Some(single_tool_plan(
            "open_browser_url",
            json!({ "url": crate::config::LUNA_BASE }),
        ));
    }
    if contains_any(norm, &["kwic"]) {
        return Some(single_tool_plan(
            "open_browser_url",
            json!({ "url": crate::config::KWIC_BASE }),
        ));
    }
    if contains_any(norm, &["kgcourse", "kgc"]) {
        return Some(single_tool_plan(
            "open_browser_url",
            json!({ "url": crate::config::KG_COURSE_BASE }),
        ));
    }

    None
}

fn single_tool_plan(name: &str, args: Value) -> Plan {
    Plan {
        tools: vec![ToolCall {
            name: name.into(),
            args,
        }],
        image_only: false,
    }
}

// ─────────────────────── Skip-Tool Detection ───────────────────────

fn should_skip_tools(history: &[crate::db::AgentMessageRow], user_text: &str) -> bool {
    let norm = normalize_planner_text(user_text);
    is_smalltalk_or_identity(&norm) || is_follow_up_with_context(history, &norm)
}

fn is_smalltalk_or_identity(norm: &str) -> bool {
    if norm.is_empty() {
        return true;
    }
    // Pure greetings / acknowledgements — never need a tool.
    const SMALLTALK: &[&str] = &[
        "こんにちは",
        "こんばんは",
        "おはよう",
        "ありがと",
        "ありがとう",
        "thanks",
        "thankyou",
        "你好",
        "您好",
        "谢谢",
        "嗨",
        "hello",
        "hi",
        "hey",
        "元気",
        "howareyou",
    ];
    // "Who are you / introduce yourself" style — answer comes from persona only.
    const IDENTITY: &[&str] = &[
        "あなたは誰",
        "君は誰",
        "是谁",
        "你是谁",
        "whoareyou",
        "自己紹介",
        "介绍一下自己",
    ];
    // Pure opinion / feeling questions about the assistant. Kept very short and
    // generic so utterances like "経済学が好き" with concrete subjects still
    // fall through to the planner.
    const OPINION: &[&str] = &["どう思う", "怎么看", "意见", "意見"];
    let short = norm.chars().count() <= 24;
    let very_short = norm.chars().count() <= 10;
    if short && contains_any(norm, SMALLTALK) {
        return true;
    }
    if short && contains_any(norm, IDENTITY) {
        return true;
    }
    if very_short && contains_any(norm, OPINION) {
        return true;
    }
    false
}

#[cfg(test)]
fn recent_downloaded_file_path(history: &[crate::db::AgentMessageRow]) -> Option<String> {
    history
        .iter()
        .rev()
        .filter(|row| row.role == "tool")
        .find_map(|row| {
            let name = row.tool_name.as_deref()?;
            if name != "list_downloaded_files" {
                return None;
            }
            let raw = row.tool_result_json.as_deref()?;
            let parsed: Value = serde_json::from_str(raw).ok()?;
            parsed
                .get("files")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
                .and_then(|file| file.get("path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

fn should_auto_read_live_note(user_text: &str, tool_name: &str) -> bool {
    if tool_name != "list_downloaded_files" {
        return false;
    }
    let norm = normalize_planner_text(user_text);
    contains_any(
        &norm,
        &[
            "讲义",
            "講義",
            "讲了什么",
            "講了什麼",
            "说了什么",
            "說了什麼",
            "上课内容",
            "上課內容",
            "这节课",
            "這節課",
            "授業内容",
            "講義内容",
            "ノート",
            "课堂笔记",
            "課堂筆記",
            "内容",
            "要点",
            "重點",
            "live",
        ],
    )
}

fn preferred_live_courses(user_text: &str, results: &[(String, Value)]) -> Vec<String> {
    let norm = normalize_planner_text(user_text);
    let wants_afternoon = contains_any(&norm, &["下午", "午後", "afternoon"]);
    let wants_morning = contains_any(&norm, &["上午", "午前", "morning"]);

    results
        .iter()
        .find_map(|(name, value)| {
            if name != "list_today_classes" {
                return None;
            }
            let classes = value.get("classes")?.as_array()?;
            let mut picked: Vec<(i64, String)> = classes
                .iter()
                .filter(|class| {
                    if class
                        .get("cancelled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    let period = class.get("period").and_then(|v| v.as_i64()).unwrap_or(0);
                    if wants_afternoon {
                        return period >= 3;
                    }
                    if wants_morning {
                        return period > 0 && period <= 2;
                    }
                    true
                })
                .filter_map(|class| {
                    let period = class.get("period").and_then(|v| v.as_i64()).unwrap_or(0);
                    let name = class.get("name").and_then(|v| v.as_str())?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    Some((period, name.to_string()))
                })
                .collect();

            if wants_afternoon {
                picked.sort_by_key(|(period, _)| *period);
            }

            Some(picked.into_iter().map(|(_, name)| name).collect::<Vec<_>>())
        })
        .unwrap_or_default()
}

fn pick_live_markdown_path(result: &Value, preferred_courses: &[String]) -> Option<String> {
    let files = result.get("files")?.as_array()?;
    let preferred_norms = preferred_courses
        .iter()
        .map(|name| normalize_planner_text(name))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();

    fn score(file: &Value, preferred_norms: &[String]) -> i64 {
        let filename = file
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let path = file
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let source = file
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let joined = normalize_planner_text(&format!("{} {}", filename, path));

        let mut score = 0_i64;
        if source == "live" {
            score += 5;
        }
        if filename.ends_with(".md") {
            score += 2;
        }
        if filename.contains("_live.md") || path.contains("_live.md") {
            score += 6;
        }
        if filename.contains("live") || path.contains("live") {
            score += 2;
        }
        for course in preferred_norms {
            if joined.contains(course) {
                score += 20;
            }
        }
        if let Some(downloaded_at) = file.get("downloaded_at").and_then(|v| v.as_i64()) {
            score += downloaded_at / 1_000_000_000;
        }
        score
    }

    files
        .iter()
        .filter_map(|file| {
            let path = file.get("path").and_then(|v| v.as_str())?;
            let filename = file
                .get("filename")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let path_lower = path.to_lowercase();
            if !filename.ends_with(".md") && !path_lower.ends_with(".md") {
                return None;
            }
            Some((score(file, &preferred_norms), path.to_string()))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, path)| path)
}

fn is_follow_up_with_context(history: &[crate::db::AgentMessageRow], norm: &str) -> bool {
    if !history.iter().rev().take(6).any(|row| row.role == "tool") {
        return false;
    }
    const DETAIL_MARKERS: &[&str] = &[
        "詳しく",
        "详细",
        "详细一点",
        "もう少し",
        "为什么",
        "為什麼",
        "怎么说",
        "什么意思",
        "哪个",
        "哪個",
        "whichone",
        "why",
        "moredetail",
        "continue",
        "続けて",
        "もっと",
        "具体的に",
        "ほかに",
        "他に",
        "还有",
        "另外",
        "第一",
        "第二",
        "第三",
        "最初",
        "最後",
        "pdf",
        "doc",
        "docx",
        "ファイル",
        "附件",
        "本文",
        "添付",
        // Calendar / action words — a short message that contains both an
        // acknowledgement and a directive (e.g. "了解、日历加一下") must still
        // trigger tool planning, not be silently swallowed.
        "日历",
        "カレンダー",
        "calendar",
        "加进",
        "加入",
        "追加",
        "登録",
        "削除",
        "删除",
        "编辑",
        "修改",
        "変更",
        "更新",
    ];
    if contains_any(norm, DETAIL_MARKERS) {
        return false;
    }
    const ACK_MARKERS: &[&str] = &[
        "ありがと",
        "ありがとう",
        "谢谢",
        "thanks",
        "thankyou",
        "ok",
        "わかった",
        "了解",
        "助かった",
        "收到",
        "明白",
        "なるほど",
        // Short CJK acknowledgements that never start an action sequence.
        // Note: "好", "行", "加", "要", "可以" are intentionally excluded because
        // they frequently serve as directives ("好，加进日历") that should still
        // trigger tool calls.
        "嗯",         // uh-huh / mm-hmm (Chinese)
        "そうですか", // I see / is that so (Japanese)
        "そうか",     // I see (Japanese)
    ];
    norm.chars().count() <= 24 && contains_any(norm, ACK_MARKERS)
}

// ─────────────────────── Plan Parsing ───────────────────────

fn parse_plan(raw: &str) -> Result<Plan, String> {
    let cleaned = agent_text::strip_think(raw);
    let trimmed = cleaned.trim();

    // Fast path: try parsing the entire string as JSON first (works with prefill).
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return plan_from_value(value);
    }

    if let Some(call) = parse_visible_tool_call(trimmed) {
        log::warn!(
            "[agent plan] recovered visible pseudo tool call from planner output: {}",
            call.name
        );
        return Ok(Plan {
            tools: vec![call],
            image_only: false,
        });
    }

    // Fallback: find the first JSON object in the string.
    if let Some(obj) = first_json_object(trimmed) {
        match serde_json::from_str::<Value>(obj) {
            Ok(value) => return plan_from_value(value),
            Err(e) => log::warn!("plan JSON parse error: {} (raw: {})", e, obj),
        }
    } else if trimmed.contains("\"tools\"") {
        // JSON mentions tools but is unbalanced — almost certainly truncated.
        log::warn!(
            "plan output looks truncated (no balanced object): {}",
            trimmed
        );
    }
    Err(format!(
        "planner returned invalid output: {}",
        truncate_for_log(trimmed, 240)
    ))
}

fn plan_from_value(value: Value) -> Result<Plan, String> {
    if !value.get("tools").is_some_and(Value::is_array) {
        return Err("planner JSON is missing a tools array".to_string());
    }
    serde_json::from_value(value).map_err(|e| format!("invalid planner JSON: {e}"))
}

fn first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut start: Option<usize> = None;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            match b {
                b'\\' => escape = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(st) = start {
                        return Some(&s[st..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

// ─────────────────────── Text Utilities ───────────────────────

fn normalize_planner_text(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !"[]()（）【】「」『』・,，.。:：!?！？_-".contains(*c))
        .collect()
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

#[cfg(test)]
fn extract_kgc_code(text: &str) -> Option<String> {
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_alphanumeric() {
            start.get_or_insert(idx);
        } else if let Some(st) = start.take() {
            let token = &text[st..idx];
            if looks_like_kgc_code(token) {
                return Some(token.to_uppercase());
            }
        }
    }
    if let Some(st) = start {
        let token = &text[st..];
        if looks_like_kgc_code(token) {
            return Some(token.to_uppercase());
        }
    }
    None
}

/// Real KGC course codes start with a small set of faculty-letter prefixes.
/// Adding the whitelist here prevents tokens like `PDF12345` or `MAC10000` —
/// which fit the structural pattern of letters+digits — from being
/// dispatched as syllabus lookups.
#[cfg(test)]
const KGC_PREFIX_WHITELIST: &[&str] = &[
    "AB", "AE", "AL", "AS", "BL", "BU", "CO", "CS", "DC", "EC", "ED", "EN", "FD", "GE", "GS", "HS",
    "HU", "IB", "IC", "IS", "JP", "LA", "LB", "LE", "LI", "LR", "LS", "MA", "MD", "ME", "MM", "MS",
    "NS", "PA", "PE", "PH", "PL", "PO", "PS", "RC", "RE", "SC", "SD", "SO", "SP", "ST", "TA", "TC",
    "TH", "TM", "TS", "UC",
];

#[cfg(test)]
fn looks_like_kgc_code(token: &str) -> bool {
    let letters_n = token
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    let digits_n = token
        .chars()
        .skip(letters_n)
        .take_while(|c| c.is_ascii_digit())
        .count();
    if !(letters_n >= 2 && digits_n >= 3 && letters_n + digits_n == token.len()) {
        return false;
    }
    // Real KGC codes are typically 2-3 letter prefix + 4-5 digits.
    if letters_n > 4 || digits_n > 6 {
        return false;
    }
    let prefix: String = token
        .chars()
        .take(2)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    KGC_PREFIX_WHITELIST.contains(&prefix.as_str())
}

fn local_text(text: &str, tokens: usize) -> String {
    crate::local_ai::trim_apple_text(text, tokens)
}

fn render_local_json(value: &Value, tokens: usize) -> String {
    let compact = crate::local_ai::compact_json_value(value, tokens);
    serde_json::to_string(&compact).unwrap_or_else(|_| "{}".into())
}
fn trim_to(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}…", truncated)
}

fn preview_of(v: &Value) -> String {
    let s = serde_json::to_string(&sanitize_answer_tool_result(v)).unwrap_or_default();
    let mut end = CFG.preview_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    if s.len() > CFG.preview_bytes {
        format!("{}…", &s[..end])
    } else {
        s
    }
}

// ─────────────────────── History Helpers ───────────────────────

fn slice_history(
    rows: &[crate::db::AgentMessageRow],
    window: usize,
) -> Vec<crate::db::AgentMessageRow> {
    if rows.is_empty() {
        return Vec::new();
    }
    let end = rows.len().saturating_sub(1);
    let start = end.saturating_sub(window);
    rows[start..end].to_vec()
}

fn maybe_autotitle(app: &AppHandle, db: &Database, conv_id: &str, user_text: &str) {
    let list = match db.agent_list_conversations() {
        Ok(l) => l,
        Err(_) => return,
    };
    let Some(row) = list.iter().find(|c| c.id == conv_id) else {
        return;
    };
    if !matches!(row.title.as_str(), "" | "新しい会話" | "エージェント") {
        return;
    }
    let title: String = user_text
        .chars()
        .filter(|c| !c.is_control())
        .take(24)
        .collect();
    let title = if title.trim().is_empty() {
        "新しい会話".to_string()
    } else {
        title
    };
    if db.agent_rename_conversation(conv_id, &title).is_ok() {
        let _ = app.emit("agent-conversations-changed", conv_id);
    }
}

// ─────────────────────── Tests ───────────────────────

#[cfg(test)]
mod tests {
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

    #[test]
    fn smalltalk_skips_tools() {
        assert!(should_skip_tools(&[], "你好"));
        assert!(should_skip_tools(&[], "あなたは誰？"));
        assert!(should_skip_tools(&[], "hello"));
    }

    #[test]
    fn detail_or_referential_follow_up_runs_tools_again() {
        // Even when recent tool context exists, follow-ups that ask for more
        // detail or refer ambiguously ("那个呢？") should re-plan rather than
        // silently reuse stale context — false positives there give wrong
        // answers. Only explicit acknowledgments skip tools; see
        // `follow_up_with_thanks_skips_tools` for that case.
        let history = vec![tool_row("list_today_classes")];
        assert!(!should_skip_tools(&history, "那个呢？"));
        assert!(!should_skip_tools(&history, "もう少し詳しく"));
    }

    #[test]
    fn deterministic_weather_plan() {
        let plan = heuristic_plan(&[], "明日の天気は？").expect("plan");
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "get_weather");
    }

    #[test]
    fn heuristic_opens_luna_with_real_browser_tool() {
        let plan = heuristic_plan(&[], "打开luna看看").expect("plan");
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "open_browser_url");
        assert_eq!(
            plan.tools[0].args.get("url").and_then(|v| v.as_str()),
            Some(crate::config::LUNA_BASE)
        );
    }

    #[test]
    fn heuristic_does_not_reopen_kwic_for_browser_clicks() {
        let plan = heuristic_plan(&[], "浏览器点击kwic的最新通知");
        assert!(
            plan.is_none(),
            "browser click intent should go through the full planner with current browser context"
        );
    }

    #[test]
    fn attached_browser_panel_home_request_starts_with_screenshot() {
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            browser_click_labels: Vec::new(),
            ..Default::default()
        };
        let plan = attached_browser_control_plan(&normalize_planner_text("回到首页"), &ctx)
            .expect("attached browser plan");
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "computer_screenshot");
    }

    #[test]
    fn attached_browser_panel_click_label_starts_with_screenshot() {
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            browser_click_labels: vec!["home".into()],
            ..Default::default()
        };
        let plan = attached_browser_control_plan(&normalize_planner_text("点啊"), &ctx)
            .expect("attached browser click plan");
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "computer_screenshot");
    }

    #[test]
    fn explicit_fill_continues_after_page_observation() {
        let plan = single_tool_plan("read_browser_page", json!({}));
        let results = vec![(
            "read_browser_page".into(),
            json!({"inputs":[{"label":"名前"}]}),
        )];
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            ..Default::default()
        };
        assert!(should_continue_after_browser_observation(
            &plan,
            &results,
            "名前にSelahと入力して",
            &ctx
        ));
        assert!(should_continue_after_browser_observation(
            &plan,
            &results,
            "fill the name field",
            &ctx
        ));
    }

    #[test]
    fn browser_observation_does_not_continue_after_action_or_for_read_only_request() {
        let observation = single_tool_plan("read_browser_page", json!({}));
        let clicked = vec![
            ("read_browser_page".into(), json!({"buttons":["次へ"]})),
            ("browser_click".into(), json!({"ok":true})),
        ];
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            ..Default::default()
        };
        assert!(!should_continue_after_browser_observation(
            &observation,
            &clicked,
            "次へをクリック",
            &ctx
        ));
        assert!(!should_continue_after_browser_observation(
            &observation,
            &[("read_browser_page".into(), json!({"text":"本文"}))],
            "このページを要約して",
            &ctx
        ));
    }

    #[test]
    fn failed_browser_action_can_reobserve_and_continue_safely() {
        let attempted = single_tool_plan("browser_click", json!({"text":"次へ"}));
        let results = vec![
            ("browser_click".into(), json!({"error":"not found"})),
            (
                "read_browser_page".into(),
                json!({"buttons":[{"text":"続ける"}]}),
            ),
        ];
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            ..Default::default()
        };
        assert!(should_continue_after_browser_observation(
            &attempted,
            &results,
            "次へをクリック",
            &ctx
        ));
        assert!(is_browser_mutation_tool("browser_click"));
        assert!(is_browser_mutation_tool("browser_fill"));
        assert!(!is_browser_mutation_tool("read_browser_page"));
        assert!(!is_browser_mutation_tool("browser_wait_for"));
    }

    #[test]
    fn lookup_followup_candidates_are_scoped_by_fresh_result_type() {
        let calendar = vec![(
            "list_google_calendar_events".into(),
            json!({"events":[{"event_id":"event-1","title":"試験"}]}),
        )];
        assert_eq!(
            allowed_lookup_followup_actions(&calendar),
            vec![
                "delete_google_calendar_event",
                "update_google_calendar_event"
            ]
        );
        assert!(should_continue_after_actionable_lookup(&calendar));

        let files = vec![(
            "list_downloaded_files".into(),
            json!({"files":[{"path":"/tmp/a.pdf"}]}),
        )];
        assert_eq!(
            allowed_lookup_followup_actions(&files),
            vec![
                "open_downloaded_file",
                "delete_downloaded_file",
                "read_downloaded_file"
            ]
        );

        let luna = vec![(
            "list_luna_todos".into(),
            json!({"todos":[{"title":"第7回課題","luna_id":"LUNA-42"}]}),
        )];
        assert_eq!(
            allowed_lookup_followup_actions(&luna),
            vec![
                "get_luna_activity_detail",
                "open_copilot_page",
                "open_luna_attachment",
                "download_luna_attachment",
                "download_course_material"
            ]
        );

        let notifications = vec![(
            "search_notifications".into(),
            json!({"notifications":[{"title":"履修登録のお知らせ"}]}),
        )];
        assert_eq!(
            allowed_lookup_followup_actions(&notifications),
            vec!["get_notification_detail", "open_copilot_page"]
        );

        let notification_detail = vec![(
            "get_notification_detail".into(),
            json!({"source":"KWIC","title":"履修登録のお知らせ"}),
        )];
        assert_eq!(
            allowed_lookup_followup_actions(&notification_detail),
            vec!["open_copilot_page"]
        );

        let luna_detail = vec![
            (
                "list_luna_todos".into(),
                json!({"todos":[{"title":"第7回課題","luna_id":"LUNA-42"}]}),
            ),
            (
                "get_luna_activity_detail".into(),
                json!({"matched_title":"第7回課題","activity_type":"report"}),
            ),
        ];
        assert!(
            !allowed_lookup_followup_actions(&luna_detail).contains(&"get_luna_activity_detail")
        );
    }

    #[test]
    fn planner_summaries_keep_dynamic_action_identifiers() {
        let calendar = summarize_plan_tool_result(
            "list_google_calendar_events",
            r#"{"events":[{"event_id":"event-123","title":"試験","date":"2026-06-15","start_time":"10:00","end_time":"11:00"}]}"#,
        );
        assert!(calendar.contains("event-123"));

        let files = summarize_plan_tool_result(
            "list_downloaded_files",
            r#"{"files":[{"path":"/tmp/lecture.pdf","filename":"lecture.pdf"}]}"#,
        );
        assert!(files.contains("/tmp/lecture.pdf"));

        let detail = summarize_plan_tool_result(
            "get_luna_activity_detail",
            r#"{"matched_title":"第7回課題","period":"2026-06-20","source":{"luna_id":"LUNA-42"},"attachments":[{"name":"instructions.pdf"},{"name":"answer.docx"}]}"#,
        );
        assert!(detail.contains("title=第7回課題"));
        assert!(detail.contains("luna_id=LUNA-42"));
        assert!(detail.contains("instructions.pdf / answer.docx"));

        let announcements = summarize_plan_tool_result(
            "list_luna_announcements",
            r#"{"announcements":[{"title":"第7回資料","course":"政治学","period":"2026-06-12","luna_id":"LUNA-42"}]}"#,
        );
        assert!(announcements.contains("title=第7回資料"));
        assert!(announcements.contains("luna_id=LUNA-42"));

        let notifications = summarize_plan_tool_result(
            "search_notifications",
            r#"{"notifications":[{"source":"KWIC","identifier":"notice-7","title":"履修登録"}]}"#,
        );
        assert!(notifications.contains("source=KWIC"));
        assert!(notifications.contains("identifier=notice-7"));

        let browser = summarize_plan_tool_result(
            "list_browser_windows",
            r#"{"windows":[{"target":"tab-1","type":"detail","title":"第7回課題","url":"index.html#surface=university-detail"}]}"#,
        );
        assert!(browser.contains("type=detail"));
        assert!(browser.contains("title=第7回課題"));
    }

    #[test]
    fn visible_plan_steps_are_specific_without_exposing_field_values() {
        let click = ToolCall {
            name: "browser_click".into(),
            args: json!({"text":"次へ"}),
        };
        assert_eq!(plan_step_detail(&click).as_deref(), Some("次へ"));

        let file = ToolCall {
            name: "read_downloaded_file".into(),
            args: json!({"path":"/Users/haru/Downloads/lecture.pdf"}),
        };
        assert_eq!(plan_step_detail(&file).as_deref(), Some("lecture.pdf"));

        let fill = ToolCall {
            name: "browser_fill".into(),
            args: json!({"label":"パスワード","value":"secret-value"}),
        };
        assert_eq!(plan_step_detail(&fill).as_deref(), Some("パスワード"));
        assert_ne!(plan_step_detail(&fill).as_deref(), Some("secret-value"));
    }

    #[test]
    fn browser_observation_can_infer_mouse_click_for_home() {
        let page = serde_json::json!({
            "links": [
                {
                    "text": "HOME",
                    "rect": { "centerX": 42, "centerY": 18 }
                }
            ],
            "interactive_elements": {
                "buttons": [],
                "inputs": []
            }
        });
        let args =
            infer_mouse_click_from_observation("回到首页", &page, &AgentTurnContext::default())
                .expect("mouse args");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(18));
    }

    #[test]
    fn browser_home_request_can_fall_back_to_top_left_logo_area() {
        let page = serde_json::json!({
            "viewport": { "width": 1200, "height": 800 },
            "links": [
                {
                    "text": "公益財団法人 ひょうご環境創造協会",
                    "rect": { "centerX": 96, "centerY": 42 }
                },
                {
                    "text": "お問い合わせ",
                    "rect": { "centerX": 980, "centerY": 48 }
                }
            ],
            "interactive_elements": {
                "buttons": [],
                "inputs": []
            }
        });
        let args =
            infer_mouse_click_from_observation("回到主页", &page, &AgentTurnContext::default())
                .expect("mouse args");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(96));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(42));
    }

    #[test]
    fn browser_screenshot_can_infer_top_left_home_click() {
        let screenshot = serde_json::json!({
            "coordinate_space": "screenshot",
            "screen_rect": { "x": 200, "y": 80, "width": 1200, "height": 800 },
            "image": { "mime": "image/png", "data_base64": "" }
        });
        let args = infer_mouse_click_from_screenshot(
            "回到主页",
            &screenshot,
            &AgentTurnContext::default(),
        )
        .expect("mouse args");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(144));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(64));
        assert_eq!(
            args.get("coordinate_space").and_then(|v| v.as_str()),
            Some("screenshot")
        );
    }

    #[test]
    fn short_click_confirmation_inherits_recent_home_intent() {
        let history = vec![
            crate::db::AgentMessageRow {
                id: 1,
                conv_id: "c".into(),
                role: "user".into(),
                content: "你不会点击logo回到主页吗".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 0,
            },
            crate::db::AgentMessageRow {
                id: 2,
                conv_id: "c".into(),
                role: "user".into(),
                content: "点啊".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 1,
            },
        ];
        let labels = browser_click_labels_for_turn(&history, "点啊");
        assert!(labels.iter().any(|label| label == "home"));
        assert!(labels.iter().any(|label| label == "logo"));
    }

    #[test]
    fn short_click_confirmation_inherits_recent_suggested_tab_label() {
        let history = vec![
            crate::db::AgentMessageRow {
                id: 1,
                conv_id: "c".into(),
                role: "assistant".into(),
                content:
                    "如果是想寻找志愿者相关的信息，我可以帮你点击上方的“ボランティア集まれ”按钮。"
                        .into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 0,
            },
            crate::db::AgentMessageRow {
                id: 2,
                conv_id: "c".into(),
                role: "user".into(),
                content: "点击".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 1,
            },
        ];
        let labels = browser_click_labels_for_turn(&history, "点击");
        assert!(labels.iter().any(|label| label == "ボランティア集まれ"));
    }

    #[test]
    fn observation_can_click_recent_suggested_tab_label() {
        let page = serde_json::json!({
            "links": [
                {
                    "text": "ボランティア集まれ",
                    "url": "https://jof-camp.com/new/volunteer/join-leader/",
                    "rect": { "centerX": 640, "centerY": 96 }
                }
            ],
            "interactive_elements": {
                "buttons": [],
                "inputs": []
            }
        });
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            browser_click_labels: vec!["ボランティア集まれ".into()],
            ..Default::default()
        };
        let args = infer_mouse_click_from_observation("点击", &page, &ctx).expect("mouse args");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(640));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(96));
    }

    #[test]
    fn generic_browse_tabs_request_clicks_safe_top_navigation_candidate() {
        let page = serde_json::json!({
            "url": "https://jof-camp.com/new/",
            "viewport": { "width": 1200, "height": 800 },
            "links": [
                {
                    "text": "HOME",
                    "url": "https://jof-camp.com/new/",
                    "rect": { "centerX": 46, "centerY": 92 }
                },
                {
                    "text": "募集中のキャンプ",
                    "url": "https://jof-camp.com/new/camp/",
                    "rect": { "centerX": 280, "centerY": 92 }
                },
                {
                    "text": "お問い合わせ",
                    "url": "https://jof-camp.com/new/contact/",
                    "rect": { "centerX": 960, "centerY": 92 }
                }
            ],
            "interactive_elements": {
                "buttons": [],
                "inputs": []
            }
        });
        let args =
            infer_tab_browse_click_from_observation("点击标签看看全部", &page).expect("tab click");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(280));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(92));
    }

    #[test]
    fn generic_tab_tail_is_not_treated_as_literal_click_label() {
        assert!(requested_click_labels(&normalize_planner_text("点击标签看看全部")).is_none());
    }

    #[test]
    fn click_label_strips_generic_button_suffix() {
        let labels =
            requested_click_labels(&normalize_planner_text("点击募集中的按钮")).expect("labels");
        assert!(labels.iter().any(|label| label == "募集中"));
    }

    #[test]
    fn numeric_selection_inherits_recent_numbered_browser_option() {
        let history = vec![
            crate::db::AgentMessageRow {
                id: 1,
                conv_id: "c".into(),
                role: "assistant".into(),
                content: "1. 页面顶部导航栏左侧的 **「募集中のキャンプ（募集中营地）」** 按钮\n2. 页面下方的 **「募集中」** 绿色图标链接".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 0,
            },
            crate::db::AgentMessageRow {
                id: 2,
                conv_id: "c".into(),
                role: "user".into(),
                content: "1".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 1,
            },
        ];
        let labels = browser_click_labels_for_turn(&history, "1");
        assert!(labels.iter().any(|label| label == "募集中のキャンプ"));
        assert!(labels.iter().all(|label| label != "募集中"));
    }

    #[test]
    fn retry_inherits_recent_explicit_click_target() {
        let history = vec![
            crate::db::AgentMessageRow {
                id: 1,
                conv_id: "c".into(),
                role: "assistant".into(),
                content: "我这就为你点击左上角的「募集中のキャンプ（募集中营地）」按钮。".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 0,
            },
            crate::db::AgentMessageRow {
                id: 2,
                conv_id: "c".into(),
                role: "user".into(),
                content: "重试".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 1,
            },
        ];
        let labels = browser_click_labels_for_turn(&history, "重试");
        assert!(labels.iter().any(|label| label == "募集中のキャンプ"));
    }

    #[test]
    fn ambiguous_short_boshuuchuu_click_prefers_top_navigation_over_pdf() {
        let page = serde_json::json!({
            "url": "https://jof-camp.com/new/",
            "links": [
                {
                    "text": "募集中",
                    "url": "https://jof-camp.com/new/files/spring.pdf",
                    "rect": { "centerX": 520, "centerY": 560 }
                },
                {
                    "text": "募集中のキャンプ",
                    "url": "https://jof-camp.com/new/jof_camp/",
                    "rect": { "centerX": 280, "centerY": 92 }
                }
            ],
            "interactive_elements": {
                "buttons": [],
                "inputs": []
            }
        });
        let args = infer_mouse_click_from_observation(
            "点击募集中的按钮",
            &page,
            &AgentTurnContext::default(),
        )
        .expect("mouse args");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(280));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(92));
    }

    #[test]
    fn observation_click_matches_cjk_equivalent_visible_label() {
        let page = serde_json::json!({
            "url": "https://kwic.kwansei.ac.jp/portal/",
            "links": [
                {
                    "text": "語学資料",
                    "url": "https://kwic.kwansei.ac.jp/portal/lang",
                    "rect": { "centerX": 92, "centerY": 44 }
                },
                {
                    "text": "履修登録",
                    "url": "https://kwic.kwansei.ac.jp/portal/registration",
                    "rect": { "centerX": 220, "centerY": 44 }
                }
            ],
            "interactive_elements": {
                "buttons": [],
                "inputs": []
            }
        });
        let args = infer_mouse_click_from_observation(
            "点击语学资料",
            &page,
            &AgentTurnContext {
                browser_target: Some("kwic-detail-0-ct".into()),
                browser_click_labels: Vec::new(),
                ..Default::default()
            },
        )
        .expect("mouse args");
        assert_eq!(args.get("x").and_then(|v| v.as_i64()), Some(92));
        assert_eq!(args.get("y").and_then(|v| v.as_i64()), Some(44));
    }

    #[test]
    fn browser_click_success_can_finish_without_remote_answer() {
        let answer = local_browser_action_answer(
            "点击可见链接",
            &[(
                "computer_mouse_click".into(),
                serde_json::json!({
                    "current_url": "https://example.test/next"
                }),
            )],
            &AgentTurnContext {
                browser_target: Some("kwic-detail-0-ct".into()),
                browser_click_labels: Vec::new(),
                ..Default::default()
            },
        )
        .expect("local browser action answer");
        assert!(answer.contains("已点击"));
        assert!(answer.contains("https://example.test/next"));
    }

    #[test]
    fn heuristic_grades() {
        let plan = heuristic_plan(&[], "成績どうだった？").expect("plan");
        assert_eq!(plan.tools[0].name, "get_grades");
    }

    #[test]
    fn heuristic_mail() {
        let plan = heuristic_plan(&[], "メール見せて").expect("plan");
        assert_eq!(plan.tools[0].name, "list_recent_mail");
    }

    #[test]
    fn heuristic_tasks() {
        let plan = heuristic_plan(&[], "未提出の課題ある？").expect("plan");
        assert_eq!(plan.tools[0].name, "list_luna_todos");
    }

    #[test]
    fn general_knowledge_falls_through() {
        // "帮我查一下地政学的相关知识" should NOT match any heuristic.
        assert!(heuristic_plan(&[], "帮我查一下地政学的相关知识").is_none());
    }

    #[test]
    fn course_name_falls_through_to_model() {
        // Course-specific queries should NOT be caught by heuristics —
        // the model needs to translate and pick the right tool.
        assert!(heuristic_plan(&[], "我下周要上国际关系历史基础").is_none());
    }

    #[test]
    fn kgc_code_extraction() {
        assert_eq!(extract_kgc_code("AB12345 の詳細"), Some("AB12345".into()));
        assert_eq!(extract_kgc_code("hello"), None);
    }

    #[test]
    fn parse_plan_from_json() {
        let plan = parse_plan("{\"tools\":[{\"name\":\"get_weather\",\"args\":{}}]}").unwrap();
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "get_weather");
    }

    #[test]
    fn parse_plan_rejects_garbage_for_retry() {
        assert!(parse_plan("not json at all").is_err());
        assert!(parse_plan(r#"{"tools":[{"name":"get_weather""#).is_err());
        assert!(parse_plan(r#"{"answer":"I will check"}"#).is_err());
    }

    #[test]
    fn trim_to_respects_limit() {
        assert_eq!(trim_to("hello", 10), "hello");
        assert_eq!(trim_to("hello world", 5), "hello…");
    }

    #[test]
    fn heuristic_tomorrow_classes() {
        let plan = heuristic_plan(&[], "明日の授業は？").expect("plan");
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "list_week_classes");
    }

    #[test]
    fn heuristic_tomorrow_chinese() {
        let plan = heuristic_plan(&[], "明天有什么课").expect("plan");
        assert_eq!(plan.tools[0].name, "list_week_classes");
    }

    #[test]
    fn heuristic_notifications() {
        let plan = heuristic_plan(&[], "お知らせある？").expect("plan");
        assert_eq!(plan.tools[0].name, "list_recent_notifications");
    }

    #[test]
    fn heuristic_registration() {
        let plan = heuristic_plan(&[], "履修科目一覧見せて").expect("plan");
        assert_eq!(plan.tools[0].name, "get_registration");
    }

    #[test]
    fn follow_up_with_thanks_skips_tools() {
        let history = vec![tool_row("get_grades")];
        assert!(should_skip_tools(&history, "ありがとう"));
        assert!(should_skip_tools(&history, "了解"));
    }

    #[test]
    fn multi_tool_query_falls_to_model() {
        // Queries requiring multiple tools or ambiguous intent should NOT match a single heuristic.
        assert!(heuristic_plan(&[], "来週の予定を全部まとめて教えて、準備するものも").is_none());
        assert!(heuristic_plan(&[], "看看邮件和课题").is_none());
        assert!(heuristic_plan(&[], "LunaとKWICを開いて").is_none());
    }

    #[test]
    fn production_preplan_leaves_business_intent_to_model() {
        let ctx = AgentTurnContext::default();
        assert!(deterministic_preplan(&[], "打开Luna看看", &ctx).is_none());
        assert!(deterministic_preplan(&[], "看看邮件", &ctx).is_none());
        assert!(deterministic_preplan(&[], "打开相关Copilot页面", &ctx).is_none());
        assert!(deterministic_preplan(&[], "明天有什么课", &ctx).is_none());
    }

    #[test]
    fn planner_failure_reads_attached_page_instead_of_doing_nothing() {
        let ctx = AgentTurnContext {
            browser_target: Some("detail-a-ct".into()),
            ..Default::default()
        };
        let plan = planner_failure_fallback(&[], "这个页面有什么", &ctx);
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "read_browser_page");
        assert_eq!(
            plan.tools[0]
                .args
                .get("target")
                .and_then(|value| value.as_str()),
            Some("detail-a-ct")
        );
    }

    #[test]
    fn empty_plan_retries_for_data_requests_but_not_recent_summaries() {
        assert!(should_retry_empty_plan(
            &[],
            "下周的课程和课题怎么样",
            &AgentTurnContext::default()
        ));
        assert!(!should_retry_empty_plan(
            &[tool_row("list_luna_todos")],
            "总结一下",
            &AgentTurnContext::default()
        ));
        assert!(!should_retry_empty_plan(
            &[],
            "帮我解释一下这个概念",
            &AgentTurnContext::default()
        ));
    }

    #[test]
    fn finalized_plan_allows_six_step_chain() {
        let plan = Plan {
            tools: [
                "get_weather",
                "list_recent_mail",
                "list_luna_todos",
                "list_today_classes",
                "list_recent_notifications",
                "get_grades",
                "get_registration",
            ]
            .into_iter()
            .map(|name| ToolCall {
                name: name.into(),
                args: json!({}),
            })
            .collect(),
            image_only: false,
        };
        let finalized =
            finalize_plan_with_diagnostics(plan, &[], "全部まとめて", &AgentTurnContext::default());
        assert_eq!(finalized.plan.tools.len(), 6);
    }

    #[test]
    fn parse_plan_with_prefill() {
        // Simulates prefilled output: {"tools":[ + model continuation
        let raw = r#"{"tools":[{"name":"get_grades","args":{}}]}"#;
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "get_grades");
    }

    #[test]
    fn parse_plan_prefill_empty_array() {
        // Model outputs ]} after prefill {"tools":[
        let raw = r#"{"tools":[]}"#;
        let plan = parse_plan(raw).unwrap();
        assert!(plan.tools.is_empty());
    }

    #[test]
    fn parse_plan_prefill_multi_tool() {
        let raw =
            r#"{"tools":[{"name":"get_grades","args":{}},{"name":"list_luna_todos","args":{}}]}"#;
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.tools.len(), 2);
        assert_eq!(plan.tools[0].name, "get_grades");
        assert_eq!(plan.tools[1].name, "list_luna_todos");
    }

    #[test]
    fn parse_plan_with_trailing_text() {
        // Model might output extra text after JSON
        let raw = r#"{"tools":[{"name":"get_weather","args":{}}]} I chose weather because..."#;
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "get_weather");
    }

    #[test]
    fn sanitize_tool_results_neutralizes_pseudo_calls() {
        let value = serde_json::json!({
            "body": "call:kg_canvas:download_luna_file({}) <call:tool /> MALFORMED_FUNCTION_CALL",
            "download_action": "/hidden",
        });
        let sanitized = sanitize_answer_tool_result(&value);
        let body = sanitized.get("body").and_then(|v| v.as_str()).unwrap();
        assert!(body.contains("call：kg_canvas"));
        assert!(!body.contains("call:kg_canvas"));
        assert!(sanitized.get("download_action").is_none());
    }

    #[test]
    fn canonical_tool_aliases_are_accepted() {
        assert_eq!(
            agent_tools::canonical_tool_name("browser_reload"),
            Some("browser_reload_page")
        );
        assert_eq!(
            agent_tools::canonical_tool_name("read_file"),
            Some("read_downloaded_file")
        );
        assert_eq!(
            agent_tools::canonical_tool_name("view_file"),
            Some("read_downloaded_file")
        );
        assert_eq!(
            agent_tools::canonical_tool_name("kg_canvas:download_luna_file"),
            Some("download_course_material")
        );
        assert_eq!(
            agent_tools::canonical_tool_name("download_luna_file"),
            Some("download_course_material")
        );
        assert_eq!(
            agent_tools::canonical_tool_name("download_material_file"),
            Some("download_course_material")
        );
        assert_eq!(
            agent_tools::canonical_tool_name("fetch_lms_course_resources"),
            Some("list_luna_announcements")
        );
        assert_eq!(
            agent_tools::exact_tool_name("open_browser_url"),
            Some("open_browser_url")
        );
        assert!(agent_tools::exact_tool_name("launch_browser").is_none());
        assert!(agent_tools::canonical_tool_name("launch_browser").is_none());
        assert!(agent_tools::is_known_tool("read_file"));
    }

    #[test]
    fn lms_resource_alias_preserves_course_keyword() {
        let args = serde_json::json!({ "course_name": "政治学基礎 ２" });
        let sanitized = agent_tools::sanitize_tool_args("fetch_lms_course_resources", &args)
            .expect("sanitized");
        assert_eq!(
            sanitized.get("keyword").and_then(|v| v.as_str()),
            Some("政治学基礎 ２")
        );
    }

    #[test]
    fn parses_visible_task_call_for_real_execution() {
        let answer = "‹task_call:download_course_material(luna_id=\"2026341390020201\",filename=\"2026年度春中間試験の実施要項.pdf\")›";
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "download_course_material");
        assert_eq!(
            call.args.get("luna_id").and_then(|v| v.as_str()),
            Some("2026341390020201")
        );
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("2026年度春中間試験の実施要項.pdf")
        );
    }

    #[test]
    fn parses_visible_json_style_tool_call() {
        let answer = r#"task_call:download_course_material{"luna_id":"2026341390020201","filename":"midterm.pdf"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "download_course_material");
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("midterm.pdf")
        );
    }

    #[test]
    fn parses_visible_download_luna_file_alias_for_real_execution() {
        let answer =
            r#"call:download_luna_file{"course_name":"政治学基礎 ２","filename":"midterm.pdf"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "download_course_material");
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("midterm.pdf")
        );
    }

    #[test]
    fn parses_visible_download_luna_file_js_style_args() {
        let answer = r#"call:download_luna_file {luna_id: "2026341390020201", file_name: "2026年度春中間試験の実施要項.pdf"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "download_course_material");
        assert_eq!(
            call.args.get("luna_id").and_then(|v| v.as_str()),
            Some("2026341390020201")
        );
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("2026年度春中間試験の実施要項.pdf")
        );
    }

    #[test]
    fn parses_glued_download_material_file_call() {
        let answer = "call:download_material_fileluna_id=2026341390020201file_name=2026年度春中間試験の実施要項.pdf";
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "download_course_material");
        assert_eq!(
            call.args.get("luna_id").and_then(|v| v.as_str()),
            Some("2026341390020201")
        );
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("2026年度春中間試験の実施要項.pdf")
        );
    }

    #[test]
    fn parses_gemini_finish_message_call_for_real_execution() {
        let answer = r#"call:read_downloaded_file {"path":"/Users/haru/Documents/Selah/政治学基礎 ２/20260525_政治学基礎　２_live.md"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "read_downloaded_file");
        assert_eq!(
            call.args.get("path").and_then(|v| v.as_str()),
            Some("/Users/haru/Documents/Selah/政治学基礎 ２/20260525_政治学基礎　２_live.md")
        );
    }

    #[test]
    fn parses_view_file_alias_for_real_execution() {
        let answer = r#"call:view_file {"path":"/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "read_downloaded_file");
        assert_eq!(
            call.args.get("path").and_then(|v| v.as_str()),
            Some(
                "/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md"
            )
        );
    }

    #[test]
    fn detects_unknown_leading_pseudo_call_without_leaking_it() {
        let answer = r#"call:imaginary_file_tool {"path":"/tmp/a.md"}"#;
        assert!(parse_visible_tool_call(answer).is_none());
        assert!(has_any_pseudo_tool_call(answer));
        let raw = parse_any_raw_tool_call(answer).expect("raw pseudo call");
        assert_eq!(raw.name, "imaginary_file_tool");
        assert_eq!(
            raw.args.get("path").and_then(|v| v.as_str()),
            Some("/tmp/a.md")
        );
    }

    #[test]
    fn parses_neutralized_fullwidth_call_for_real_execution() {
        let answer = r#"call：read_file〔path: "/Users/haru/Documents/Selah/政治学基礎 ２/2026年度春中間試験の実施要項.pdf"〕"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "read_downloaded_file");
        assert_eq!(
            call.args.get("path").and_then(|v| v.as_str()),
            Some("/Users/haru/Documents/Selah/政治学基礎 ２/2026年度春中間試験の実施要項.pdf")
        );
    }

    #[test]
    fn parses_fullwidth_arg_delimiter_in_neutralized_call() {
        let answer = r#"call：read_file〔path： "/tmp/midterm.pdf"〕"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "read_downloaded_file");
        assert_eq!(
            call.args.get("path").and_then(|v| v.as_str()),
            Some("/tmp/midterm.pdf")
        );
    }

    #[test]
    fn parses_read_downloaded_file_filename_only_call() {
        let answer = r#"call:read_downloaded_file {"filename":"20260525_政治学基礎　２_live.md","course_name":"政治学基礎 ２"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "read_downloaded_file");
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("20260525_政治学基礎　２_live.md")
        );
        assert_eq!(
            call.args.get("course_name").and_then(|v| v.as_str()),
            Some("政治学基礎 ２")
        );
    }

    #[test]
    fn parses_fullwidth_bracket_course_context_call() {
        let answer = r#"call:get_course_context〔kgc_code: "34139002"〕"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "get_course_context");
        assert_eq!(
            call.args.get("query").and_then(|v| v.as_str()),
            Some("34139002")
        );
    }

    #[test]
    fn parses_call_space_course_context_call() {
        let answer = r#"call get_course_context {luna_id: "2026341390020201"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "get_course_context");
        assert_eq!(
            call.args.get("query").and_then(|v| v.as_str()),
            Some("2026341390020201")
        );
    }

    #[test]
    fn parses_activity_title_alias_for_luna_detail_call() {
        let answer = r#"call:get_luna_activity_detail{activity_title:"第7回復習課題（5/29 23:59締め切り）",luna_id:"2026341390020201"}"#;
        let call = parse_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "get_luna_activity_detail");
        assert_eq!(
            call.args.get("title").and_then(|v| v.as_str()),
            Some("第7回復習課題（5/29 23:59締め切り）")
        );
        assert_eq!(
            call.args.get("luna_id").and_then(|v| v.as_str()),
            Some("2026341390020201")
        );
    }

    #[test]
    fn sanitizer_accepts_common_tool_arg_aliases() {
        let course_args = serde_json::json!({ "course_code": "34139002" });
        let course = agent_tools::sanitize_tool_args("get_course_context", &course_args)
            .expect("course context args");
        assert_eq!(
            course.get("query").and_then(|v| v.as_str()),
            Some("34139002")
        );

        let detail_args = serde_json::json!({ "activityTitle": "中間試験", "type": "material" });
        let detail = agent_tools::sanitize_tool_args("get_luna_activity_detail", &detail_args)
            .expect("luna detail args");
        assert_eq!(
            detail.get("title").and_then(|v| v.as_str()),
            Some("中間試験")
        );
        assert_eq!(
            detail.get("activity_type").and_then(|v| v.as_str()),
            Some("material")
        );

        let material_args =
            serde_json::json!({ "attachment_name": "2026年度春中間試験の実施要項.pdf" });
        let material = agent_tools::sanitize_tool_args("download_course_material", &material_args)
            .expect("download material args");
        assert_eq!(
            material.get("filename").and_then(|v| v.as_str()),
            Some("2026年度春中間試験の実施要項.pdf")
        );
    }

    #[test]
    fn parse_plan_recovers_visible_tool_call() {
        let plan = parse_plan(r#"call:read_downloaded_file {"path":"/tmp/a.md"}"#).expect("plan");
        assert!(!plan.image_only);
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "read_downloaded_file");
        assert_eq!(
            plan.tools[0].args.get("path").and_then(|v| v.as_str()),
            Some("/tmp/a.md")
        );
    }

    #[test]
    fn visible_tool_call_parser_requires_leading_call() {
        let answer = "これは説明です。task_call:download_course_material(filename=\"midterm.pdf\")";
        assert!(parse_visible_tool_call(answer).is_none());
    }

    #[test]
    fn any_visible_tool_call_parser_handles_nonleading_call() {
        let answer = r#"確認します。 call:view_file {"path":"/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md"}"#;
        assert!(parse_visible_tool_call(answer).is_none());
        let call = parse_any_visible_tool_call(answer).expect("tool call");
        assert_eq!(call.name, "read_downloaded_file");
        assert_eq!(
            call.args.get("path").and_then(|v| v.as_str()),
            Some(
                "/Users/haru/Documents/Selah/キリスト教学Ａ １/20260519_キリスト教学Ａ　１_live.md"
            )
        );
        assert!(has_any_pseudo_tool_call(answer));
    }

    #[test]
    fn pseudo_call_scan_ignores_normal_words() {
        assert!(find_pseudo_tool_call_start("callback: done").is_none());
        assert!(find_pseudo_tool_call_start("recall the file later").is_none());
        assert!(find_pseudo_tool_call_start(
            "確認: call:get_course_context〔kgc_code: \"34139002\"〕"
        )
        .is_some());
    }

    #[test]
    fn safe_visible_emit_len_holds_tail_for_split_detection() {
        assert_eq!(safe_visible_emit_len("短い call", 16), 0);
        let long = "これは普通の説明です。あとで call";
        let emit_len = safe_visible_emit_len(long, 8);
        assert!(emit_len > 0);
        assert!(long[emit_len..].contains("call"));
    }

    #[test]
    fn visible_stream_start_detects_split_pseudo_call() {
        assert!(matches!(
            classify_visible_stream_start("‹task_"),
            VisibleStart::MaybePseudoCall
        ));
        assert!(matches!(
            classify_visible_stream_start("‹task_call:download_course_material("),
            VisibleStart::PseudoCall
        ));
        assert!(matches!(
            classify_visible_stream_start("call "),
            VisibleStart::PseudoCall
        ));
        assert!(matches!(
            classify_visible_stream_start("call：read_file〔"),
            VisibleStart::PseudoCall
        ));
        assert!(matches!(
            classify_visible_stream_start("call get_course_context {"),
            VisibleStart::PseudoCall
        ));
        assert!(matches!(
            classify_visible_stream_start("我看了一下資料"),
            VisibleStart::Normal
        ));
    }

    #[test]
    fn estimate_tokens_sanity() {
        // Short ASCII text
        assert!(estimate_tokens("hello") > 0);
        // CJK text (3 bytes per char)
        let cjk = "こんにちは"; // 15 bytes
        assert!(estimate_tokens(cjk) >= 3);
        // Empty
        assert_eq!(estimate_tokens(""), 1);
    }

    #[test]
    fn heuristic_student_profile() {
        let plan = heuristic_plan(&[], "学籍番号教えて").expect("plan");
        assert_eq!(plan.tools[0].name, "get_student_profile");
    }

    #[test]
    fn heuristic_today_brief() {
        let plan = heuristic_plan(&[], "今天有什么安排").expect("plan");
        assert_eq!(plan.tools[0].name, "get_today_brief");
    }

    #[test]
    fn kgc_code_whitelist_rejects_random_token() {
        // PDF12345 fits the structural pattern but isn't a real KGC prefix.
        assert_eq!(extract_kgc_code("PDF12345 syllabus"), None);
        // AB12345 should still be picked up.
        assert_eq!(extract_kgc_code("AB12345 syllabus"), Some("AB12345".into()));
    }

    #[test]
    fn opinion_short_skips_smalltalk_but_long_does_not() {
        assert!(should_skip_tools(&[], "どう思う？"));
        assert!(!should_skip_tools(
            &[],
            "経済学が好きだから経済学の授業教えて"
        ));
    }

    #[test]
    fn dispatch_known_includes_new_tools() {
        for name in [
            "search_mail",
            "list_luna_announcements",
            "delete_downloaded_file",
            "download_url",
            "browser_close",
            "get_today_brief",
            "get_notification_detail",
        ] {
            assert!(
                agent_tools::is_known_tool(name),
                "tool {} missing from registry",
                name
            );
        }
    }

    #[test]
    fn sanitize_get_notification_detail_args() {
        let args = serde_json::json!({"title": "  休講のお知らせ  "});
        let cleaned = agent_tools::sanitize_tool_args("get_notification_detail", &args).unwrap();
        assert_eq!(
            cleaned.get("title").and_then(|v| v.as_str()),
            Some("休講のお知らせ")
        );

        let empty = serde_json::json!({});
        assert!(agent_tools::sanitize_tool_args("get_notification_detail", &empty).is_none());
    }

    #[test]
    fn all_registered_tools_have_dispatch_arms() {
        let registered =
            agent_tools::registered_tool_names().collect::<std::collections::BTreeSet<_>>();
        let dispatched = agent_tools::dispatched_tool_names()
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            registered, dispatched,
            "TOOL_SPECS and dispatch arms drifted apart"
        );
    }

    #[test]
    fn finalize_plan_reports_rejected_tools_for_repair() {
        let plan = Plan {
            tools: vec![
                ToolCall {
                    name: "launch_browser".into(),
                    args: serde_json::json!({ "url": "https://example.com" }),
                },
                ToolCall {
                    name: "browser_click".into(),
                    args: serde_json::json!({}),
                },
                ToolCall {
                    name: "open_browser_url".into(),
                    args: serde_json::json!({ "url": "https://example.com" }),
                },
                ToolCall {
                    name: "read_downloaded_file".into(),
                    args: serde_json::json!({ "path": "<PATH_FROM_LIST>" }),
                },
            ],
            image_only: false,
        };
        let finalized = finalize_plan_with_diagnostics(
            plan,
            &[],
            "打开 example.com",
            &AgentTurnContext::default(),
        );
        assert_eq!(finalized.unknown_tools, vec!["launch_browser"]);
        assert_eq!(
            finalized.invalid_args,
            vec!["browser_click", "read_downloaded_file"]
        );
        assert_eq!(finalized.plan.tools.len(), 1);
        assert_eq!(finalized.plan.tools[0].name, "open_browser_url");
        assert!(plan_repair_note(&finalized).contains("launch_browser"));
    }

    #[test]
    fn placeholder_detection_does_not_reject_literal_markup() {
        assert!(contains_unresolved_plan_placeholder(
            &json!({"path":"<PATH_FROM_LIST>"})
        ));
        assert!(contains_unresolved_plan_placeholder(
            &json!({"event_id":"<event_id_from_list>"})
        ));
        assert!(!contains_unresolved_plan_placeholder(
            &json!({"value":"<div>"})
        ));
        assert!(!contains_unresolved_plan_placeholder(
            &json!({"value":"<日本語>"})
        ));
    }

    #[test]
    fn finalize_plan_locks_browser_target_for_attached_panel() {
        let plan = Plan {
            tools: vec![
                ToolCall {
                    name: "read_browser_page".into(),
                    args: serde_json::json!({}),
                },
                ToolCall {
                    name: "browser_click".into(),
                    args: serde_json::json!({
                        "target": "ext-b-ct",
                        "text": "詳細",
                    }),
                },
                ToolCall {
                    name: "list_browser_windows".into(),
                    args: serde_json::json!({}),
                },
            ],
            image_only: false,
        };
        let ctx = AgentTurnContext {
            browser_target: Some("ext-a-ct".into()),
            browser_click_labels: Vec::new(),
            ..Default::default()
        };
        let finalized = finalize_plan_with_diagnostics(plan, &[], "这个页面看看", &ctx);
        assert_eq!(finalized.plan.tools.len(), 3);
        assert_eq!(
            finalized.plan.tools[0]
                .args
                .get("target")
                .and_then(|v| v.as_str()),
            Some("ext-a-ct")
        );
        assert_eq!(
            finalized.plan.tools[1]
                .args
                .get("target")
                .and_then(|v| v.as_str()),
            Some("ext-a-ct")
        );
        assert!(finalized.plan.tools[2].args.get("target").is_none());
    }

    #[test]
    fn build_plan_messages_structure() {
        let history = vec![
            crate::db::AgentMessageRow {
                id: 1,
                conv_id: "c".into(),
                role: "user".into(),
                content: "天気は？".into(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 0,
            },
            tool_row("get_weather"),
        ];
        let msgs = build_plan_messages(None, &history, "明日は？", true);
        // system + 1 user history + 1 tool history + current user = 4
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs.last().unwrap().role, "user");
        assert_eq!(msgs.last().unwrap().content, "明日は？");
    }

    #[test]
    fn build_answer_messages_includes_tool_results() {
        let tool_results = vec![("get_weather".to_string(), serde_json::json!({"temp": 22}))];
        let msgs = build_answer_messages(
            None,
            &[],
            "天気は？",
            &[],
            &tool_results,
            None,
            &AgentTurnContext::default(),
            false,
            false,
        );
        assert_eq!(msgs.len(), 2); // system + user
        assert!(msgs[0].content.contains("tool_results"));
        assert!(msgs[0].content.contains("get_weather"));
        assert!(msgs[0].content.contains("TOOL EXECUTION BOUNDARY"));
        assert!(msgs[0].content.contains("AVAILABLE TOOLS REFERENCE"));
        assert!(msgs[0].content.contains("open_browser_url(url: string)"));
    }
    #[test]
    fn local_answer_omits_tool_catalog_but_keeps_results() {
        let tool_results = vec![("get_weather".to_string(), serde_json::json!({"temp": 22}))];
        let msgs = build_answer_messages(
            None,
            &[],
            "天気は？",
            &[],
            &tool_results,
            None,
            &AgentTurnContext::default(),
            false,
            true,
        );
        assert!(msgs[0].content.contains("get_weather"));
        assert!(!msgs[0].content.contains("open_browser_url(url: string)"));
        assert_eq!(msgs.last().unwrap().content, "天気は？");
    }

    #[test]
    fn local_answer_compacts_tool_json_instead_of_slicing_it() {
        let todos: Vec<_> = (0..40)
            .map(|index| {
                serde_json::json!({
                    "title": format!("課題{index}{}", "あ".repeat(40)),
                    "course": "科目",
                    "deadline": "2026-04-20"
                })
            })
            .collect();
        let tool_results = vec![(
            "list_luna_todos".to_string(),
            serde_json::json!({"todos": todos}),
        )];
        let msgs = build_answer_messages(
            None,
            &[],
            "課題は？",
            &[],
            &tool_results,
            None,
            &AgentTurnContext::default(),
            false,
            true,
        );
        let system = &msgs[0].content;
        let marker = "[list_luna_todos]";
        let start = system.find(marker).expect("tool label");
        let rest = system[start + marker.len()..].trim_start();
        let line = rest.lines().next().expect("json line").trim();
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("tool JSON was sliced: {error}: {line}");
        });
        assert!(parsed
            .get("todos")
            .and_then(|value| value.as_array())
            .is_some());
    }

    #[test]
    fn local_plan_fits_apple_window() {
        let msgs = build_plan_messages_with_note(
            None,
            &[],
            "明日の予定は？",
            false,
            None,
            &AgentTurnContext::default(),
            false,
            true,
        );
        let system = &msgs[0].content;
        assert!(system.contains("list_today_classes()"));
        assert!(!system.contains("FAST SELECTION MAP"));
        assert!(
            crate::local_ai::estimate_apple_tokens(system)
                < crate::local_ai::APPLE_PROMPT_TOKEN_BUDGET
        );
    }

    #[test]
    fn build_answer_messages_budget_limits_history() {
        // Each message is trimmed to 1200 chars (~400 tokens).
        // 200 messages × ~410 tokens = ~82000 > budget of 50000.
        let long_msg = "あ".repeat(20000);
        let history: Vec<crate::db::AgentMessageRow> = (0..200)
            .map(|i| crate::db::AgentMessageRow {
                id: i,
                conv_id: "c".into(),
                role: if i % 2 == 0 {
                    "user".into()
                } else {
                    "assistant".into()
                },
                content: long_msg.clone(),
                images_json: None,
                tool_name: None,
                tool_result_json: None,
                created_at: 0,
            })
            .collect();
        let msgs = build_answer_messages(
            None,
            &history,
            "test",
            &[],
            &[],
            None,
            &AgentTurnContext::default(),
            false,
            false,
        );
        // Budget should prevent ALL 200 history messages from being included.
        assert!(
            msgs.len() < 200,
            "expected truncation, got {} messages",
            msgs.len()
        );
        assert_eq!(msgs.last().unwrap().content, "test");
    }
}
