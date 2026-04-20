//! Local-only agent loop (Selah persona).
//!
//! Two-phase design:
//!   Phase 1 — Planning: asks the model to pick tools (JSON, non-streaming).
//!   Phase 2 — Answering: streams the final reply with persona + tool results.
//!
//! Small 2B/4B models are unreliable at multi-turn ReAct, so we constrain
//! them to a single planning step per turn.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, Manager};

use crate::agent_error::AgentError;
use crate::agent_prompts;
use crate::agent_provider::AgentProvider;
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
}

const CFG: AgentConfig = AgentConfig {
    history_window: 6,
    max_tools: 4,
    plan_temperature: 0.1,
    // Give reasoning models full headroom — thinking produces better tool choices.
    plan_max_tokens: 8192,
    plan_think_budget_pct: 60,
    plan_history_turns: 4,
    plan_prefill: "{\"tools\":[",
    answer_think_budget_pct: 75,
    prompt_token_budget: 120_000,
    tool_result_chars: 7000,
    recent_tool_result_chars: 4000,
    recent_tool_context: 3,
    preview_bytes: 180,
    tool_timeout_secs: 35,
};

// ─────────────────────── Stream Events ───────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent<'a> {
    Phase {
        stage: &'a str,
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

// ─────────────────────── Public Entry Point ───────────────────────

/// Called from the Tauri command layer.
pub async fn agent_send(
    app: AppHandle,
    conv_id: String,
    user_text: String,
    user_images: Vec<ImagePart>,
) -> Result<(), String> {
    let result = run_turn(&app, &conv_id, user_text, user_images).await;
    match &result {
        Ok(()) => emit(&app, &conv_id, &StreamEvent::Done),
        Err(e) => {
            let msg = e.to_string();
            emit(&app, &conv_id, &StreamEvent::Error { message: &msg });
        }
    }
    result.map_err(|e| e.to_string())
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
) -> Result<(), AgentError> {
    let provider = AgentProvider::resolve()?;
    let db = app.state::<Database>();

    // 1. Persist user message.
    persist_user_message(&db, conv_id, &user_text, &user_images)?;

    // 2. Load conversation history.
    let history = db.agent_load_messages(conv_id).unwrap_or_default();
    let history_slice = slice_history(&history, CFG.history_window);

    // 3. Phase 1 — plan (skip for image-only turns).
    let plan = plan_phase(
        app,
        conv_id,
        &provider,
        &history_slice,
        &user_text,
        &user_images,
    )
    .await;

    // 4. Execute tools.
    let tool_results = execute_tools(app, conv_id, &db, &plan, &user_text).await;

    // 5. Phase 2 — stream answer.
    let answer = answer_phase(
        app,
        conv_id,
        &provider,
        &history_slice,
        &user_text,
        &user_images,
        &tool_results,
    )
    .await?;

    // 6. Persist assistant response.
    db.agent_append_message(conv_id, "assistant", &answer, None, None, None)
        .map_err(AgentError::db)?;

    Ok(())
}

fn persist_user_message(
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
    maybe_autotitle(db, conv_id, user_text);
    Ok(())
}

// ─────────────────────── Phase 1: Planning ───────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
struct Plan {
    #[serde(default)]
    tools: Vec<ToolCall>,
    #[serde(default)]
    #[allow(dead_code)]
    image_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    args: Value,
}

async fn plan_phase(
    app: &AppHandle,
    conv_id: &str,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
) -> Plan {
    if !user_images.is_empty() {
        return Plan {
            tools: vec![],
            image_only: true,
        };
    }
    emit(app, conv_id, &StreamEvent::Phase { stage: "planning" });
    choose_plan(provider, history, user_text).await
}

async fn choose_plan(
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
) -> Plan {
    // Fast path: heuristic covers unambiguous keywords.
    if let Some(plan) = heuristic_plan(history, user_text) {
        return finalize_plan(plan, history, user_text);
    }
    // Slow path: ask model.
    match run_plan_inference(provider, history, user_text).await {
        Ok(plan) => finalize_plan(plan, history, user_text),
        Err(e) => {
            log::warn!("agent plan phase failed: {} — proceeding with no tools", e);
            Plan::default()
        }
    }
}

async fn run_plan_inference(
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
) -> Result<Plan, AgentError> {
    let supports_prefill = provider.supports_prefill();
    log::debug!(
        "[agent plan] user_text={:?} history_tool_turns={}",
        truncate_for_log(user_text, 200),
        history.iter().filter(|r| r.role == "tool").count()
    );
    let msgs = build_plan_messages(history, user_text, supports_prefill);
    let prefill = if supports_prefill {
        CFG.plan_prefill
    } else {
        ""
    };

    let raw = provider
        .plan(
            msgs,
            CFG.plan_max_tokens,
            CFG.plan_temperature,
            prefill,
            CFG.plan_think_budget_pct,
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
fn build_plan_messages(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    supports_prefill: bool,
) -> Vec<ChatMessage> {
    let mut msgs = vec![ChatMessage {
        role: "system".into(),
        content: agent_prompts::plan_system_prompt(&datetime_context(), supports_prefill),
        images: Vec::new(),
    }];

    for row in history
        .iter()
        .rev()
        .take(CFG.plan_history_turns)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        match row.role.as_str() {
            "user" | "assistant" => msgs.push(ChatMessage {
                role: row.role.clone(),
                content: trim_to(&row.content, 400),
                images: Vec::new(),
            }),
            "tool" => {
                if let (Some(name), Some(json)) =
                    (row.tool_name.as_deref(), row.tool_result_json.as_deref())
                {
                    msgs.push(ChatMessage {
                        role: "assistant".into(),
                        content: format!(
                            "[tool result: {}] {}",
                            name,
                            summarize_plan_tool_result(name, json)
                        ),
                        images: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }

    msgs.push(ChatMessage {
        role: "user".into(),
        content: user_text.to_string(),
        images: Vec::new(),
    });

    msgs
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
                        "todo[title={}, course={}]",
                        t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("course").and_then(|v| v.as_str()).unwrap_or("")
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
                            "browser[target={}, url={}]",
                            w.get("target").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("url").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "search_notifications" | "list_recent_notifications" => parsed
            .get("notifications")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|n| {
                        format!(
                            "notification[title={}]",
                            n.get("title").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        _ => None,
    };
    trim_to(summary.as_deref().unwrap_or(json), 260)
}

fn finalize_plan(plan: Plan, history: &[crate::db::AgentMessageRow], user_text: &str) -> Plan {
    if should_skip_tools(history, user_text) {
        log::debug!(
            "[agent plan] skip_tools=true (smalltalk/followup), dropping {} tool(s)",
            plan.tools.len()
        );
        return Plan::default();
    }
    let mut seen = HashSet::new();
    let tools: Vec<ToolCall> = plan
        .tools
        .into_iter()
        .filter_map(|call| {
            let sanitized = agent_tools::sanitize_tool_args(&call.name, &call.args);
            if sanitized.is_none() {
                log::warn!(
                    "[agent plan] tool dropped by sanitize: name={} args={}",
                    call.name,
                    call.args
                );
            }
            let args = sanitized?;
            let key = format!(
                "{}:{}",
                call.name,
                serde_json::to_string(&args).unwrap_or_default()
            );
            if !seen.insert(key) {
                return None;
            }
            Some(ToolCall {
                name: call.name,
                args,
            })
        })
        .take(CFG.max_tools)
        .collect();
    Plan {
        tools,
        image_only: plan.image_only,
    }
}

// ─────────────────────── Tool Execution ───────────────────────

async fn execute_tools(
    app: &AppHandle,
    conv_id: &str,
    db: &Database,
    plan: &Plan,
    user_text: &str,
) -> Vec<(String, Value)> {
    let mut results = Vec::new();
    let mut auto_read_done = false;
    let plan_already_reads_file = plan
        .tools
        .iter()
        .any(|call| call.name == "read_downloaded_file");
    for call in plan.tools.iter().take(CFG.max_tools) {
        emit(app, conv_id, &StreamEvent::ToolCall { name: &call.name });
        let started = std::time::Instant::now();
        log::debug!(
            "[agent tool] start name={} args={}",
            call.name,
            serde_json::to_string(&call.args).unwrap_or_default()
        );
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(CFG.tool_timeout_secs),
            agent_tools::dispatch(app, &call.name, &call.args),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => json!({
                "error": format!("tool timed out after {}s", CFG.tool_timeout_secs),
            }),
        };
        let ok = result.get("error").is_none();
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
                let auto_result = match tokio::time::timeout(
                    std::time::Duration::from_secs(CFG.tool_timeout_secs),
                    agent_tools::dispatch(app, "read_downloaded_file", &auto_args),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => json!({
                        "error": format!("tool timed out after {}s", CFG.tool_timeout_secs),
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
                let auto_json =
                    serde_json::to_string(&auto_result).unwrap_or_else(|_| "{}".into());
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
            }
        }
    }
    results
}

// ─────────────────────── Phase 2: Answer ───────────────────────

async fn answer_phase(
    app: &AppHandle,
    conv_id: &str,
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    tool_results: &[(String, Value)],
) -> Result<String, AgentError> {
    emit(app, conv_id, &StreamEvent::Phase { stage: "answering" });

    let messages = build_answer_messages(history, user_text, user_images, tool_results);
    log::debug!(
        "[agent answer] start conv_id={} messages={} tool_results={}",
        conv_id,
        messages.len(),
        tool_results.len()
    );

    let app_for_cb = app.clone();
    let conv_for_cb = conv_id.to_string();
    let gen_id = conv_id.to_string();
    let visible_chars = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let visible_chars_for_cb = visible_chars.clone();

    let answer = provider
        .answer(
            messages,
            &gen_id,
            CFG.answer_think_budget_pct,
            move |chunk: &str, is_think: bool| {
                if !is_think && !chunk.is_empty() {
                    visible_chars_for_cb
                        .fetch_add(chunk.chars().count(), std::sync::atomic::Ordering::Relaxed);
                }
                let topic = format!("agent_stream:{}", conv_for_cb);
                let ev = if is_think {
                    StreamEvent::Think { text: chunk }
                } else {
                    StreamEvent::Token { text: chunk }
                };
                let _ = app_for_cb.emit(&topic, &ev);
            },
        )
        .await?;
    if visible_chars.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        let cleaned = strip_think(&answer).trim().to_string();
        if !cleaned.is_empty() {
            log::warn!(
                "[agent answer] no visible token was streamed; emitting cleaned final answer chars={}",
                cleaned.len()
            );
            emit(app, conv_id, &StreamEvent::Token { text: &cleaned });
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

fn build_answer_messages(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    user_images: &[ImagePart],
    tool_results: &[(String, Value)],
) -> Vec<ChatMessage> {
    let mut budget = CFG.prompt_token_budget;

    // ── System prompt: persona + date + tool results ──
    let mut system = String::from(agent_prompts::PERSONA_PROMPT);
    system.push_str(&format!(
        "\n\n=== CURRENT DATE/TIME ===\n{}\n",
        datetime_context()
    ));

    if !tool_results.is_empty() {
        system.push_str("\n\n<tool_results>\n");
        for (name, value) in tool_results {
            let json_str = serde_json::to_string(&sanitize_answer_tool_result(value))
                .unwrap_or_else(|_| "{}".into());
            system.push_str(&format!(
                "[{}] {}\n",
                name,
                trim_to(&json_str, CFG.tool_result_chars)
            ));
        }
        system.push_str("</tool_results>\n");
    }

    let recent = recent_tool_results(history, CFG.recent_tool_context);
    if !recent.is_empty() {
        system.push_str("\n<recent_tool_results>\n");
        for (name, json) in &recent {
            let sanitized = serde_json::from_str::<Value>(json)
                .map(|v| sanitize_answer_tool_result(&v))
                .unwrap_or_else(|_| Value::String(trim_to(json, CFG.recent_tool_result_chars)));
            let safe_json = serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".into());
            system.push_str(&format!(
                "[{}] {}\n",
                name,
                trim_to(&safe_json, CFG.recent_tool_result_chars)
            ));
        }
        system.push_str("</recent_tool_results>\n");
    }

    if !user_images.is_empty() {
        system.push_str(
            "\n[IMAGE NOTICE] The user sent an image, but the current model cannot see images.\n\
             Briefly say you cannot view images yet and ask for a text description.\n\
             Do not guess image contents. Do not add unrelated topics.\n",
        );
    }

    budget = budget.saturating_sub(estimate_tokens(&system));
    budget = budget.saturating_sub(estimate_tokens(user_text));

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
        let content = trim_to(&row.content, 1200);
        let cost = estimate_tokens(&content) + 10; // overhead for role/tags
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

    msgs.push(ChatMessage {
        role: "user".into(),
        content: user_text.to_string(),
        images: user_images.to_vec(),
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
        _ => value.clone(),
    }
}

// ─────────────────────── Heuristic Planner ───────────────────────
//
// Table-driven keyword matching for unambiguous intents.  Falls through to the
// model when no rule matches.  This avoids a model round-trip for the most
// common queries and is cheaper than 20+ if-else branches.

struct HeuristicRule {
    keywords: &'static [&'static str],
    /// Extra keywords that must ALSO match (empty = no extra requirement).
    requires: &'static [&'static str],
    tool: &'static str,
    args: fn() -> Value,
}

const HEURISTIC_RULES: &[HeuristicRule] = &[
    HeuristicRule {
        keywords: &["天気", "weather", "天气"],
        requires: &[],
        tool: "get_weather",
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
        keywords: &["pdf", "docx", "ファイル", "附件", "添付", "ダウンロード"],
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
];

fn heuristic_plan(history: &[crate::db::AgentMessageRow], user_text: &str) -> Option<Plan> {
    if should_skip_tools(history, user_text) {
        return Some(Plan::default());
    }

    let norm = normalize_planner_text(user_text);

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
    const IDENTITY: &[&str] = &[
        "あなたは誰",
        "君は誰",
        "是谁",
        "你是谁",
        "whoareyou",
        "自己紹介",
        "介绍一下自己",
        "好き",
        "like",
        "喜歡",
        "喜欢",
        "意见",
        "意見",
        "怎么看",
        "どう思う",
    ];
    let short = norm.chars().count() <= 24;
    (short && contains_any(norm, SMALLTALK)) || (short && contains_any(norm, IDENTITY))
}

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
    ];
    norm.chars().count() <= 24 && contains_any(norm, ACK_MARKERS)
}

// ─────────────────────── Plan Parsing ───────────────────────

fn parse_plan(raw: &str) -> Result<Plan, String> {
    let cleaned = strip_think(raw);
    let trimmed = cleaned.trim();

    // Fast path: try parsing the entire string as JSON first (works with prefill).
    if let Ok(plan) = serde_json::from_str::<Plan>(trimmed) {
        return Ok(plan);
    }

    // Fallback: find the first JSON object in the string.
    if let Some(obj) = first_json_object(trimmed) {
        match serde_json::from_str::<Plan>(obj) {
            Ok(p) => return Ok(p),
            Err(e) => log::warn!("plan JSON parse error: {} (raw: {})", e, obj),
        }
    } else if trimmed.contains("\"tools\"") {
        // JSON mentions tools but is unbalanced — almost certainly truncated.
        log::warn!(
            "plan output looks truncated (no balanced object): {}",
            trimmed
        );
    }
    Ok(Plan::default())
}

fn strip_think(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end_rel) => rest = &rest[start + end_rel + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
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
                if depth > 0 {
                    depth -= 1;
                }
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

fn looks_like_kgc_code(token: &str) -> bool {
    let letters = token
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    let digits = token
        .chars()
        .skip(letters)
        .take_while(|c| c.is_ascii_digit())
        .count();
    letters >= 2 && digits >= 3 && letters + digits == token.len()
}

fn trim_to(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}…", truncated)
}

fn preview_of(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
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

fn maybe_autotitle(db: &Database, conv_id: &str, user_text: &str) {
    let list = match db.agent_list_conversations() {
        Ok(l) => l,
        Err(_) => return,
    };
    let Some(row) = list.iter().find(|c| c.id == conv_id) else {
        return;
    };
    if row.title != "新しい会話" && !row.title.is_empty() {
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
    let _ = db.agent_rename_conversation(conv_id, &title);
}

// ─────────────────────── Tests ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
    fn follow_up_reuses_recent_tool_context() {
        let history = vec![tool_row("list_today_classes")];
        assert!(should_skip_tools(&history, "那个呢？"));
        assert!(should_skip_tools(&history, "もう少し詳しく"));
    }

    #[test]
    fn deterministic_weather_plan() {
        let plan = heuristic_plan(&[], "明日の天気は？").expect("plan");
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "get_weather");
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
    fn strip_think_blocks() {
        assert_eq!(
            strip_think("<think>reasoning</think>{\"tools\":[]}"),
            "{\"tools\":[]}"
        );
        assert_eq!(strip_think("no tags here"), "no tags here");
    }

    #[test]
    fn parse_plan_from_json() {
        let plan = parse_plan("{\"tools\":[{\"name\":\"get_weather\",\"args\":{}}]}").unwrap();
        assert_eq!(plan.tools.len(), 1);
        assert_eq!(plan.tools[0].name, "get_weather");
    }

    #[test]
    fn parse_plan_empty_on_garbage() {
        let plan = parse_plan("not json at all").unwrap();
        assert!(plan.tools.is_empty());
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
        let msgs = build_plan_messages(&history, "明日は？", true);
        // system + 1 user history + 1 tool history + current user = 4
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs.last().unwrap().role, "user");
        assert_eq!(msgs.last().unwrap().content, "明日は？");
    }

    #[test]
    fn build_answer_messages_includes_tool_results() {
        let tool_results = vec![("get_weather".to_string(), serde_json::json!({"temp": 22}))];
        let msgs = build_answer_messages(&[], "天気は？", &[], &tool_results);
        assert_eq!(msgs.len(), 2); // system + user
        assert!(msgs[0].content.contains("tool_results"));
        assert!(msgs[0].content.contains("get_weather"));
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
        let msgs = build_answer_messages(&history, "test", &[], &[]);
        // Budget should prevent ALL 200 history messages from being included.
        assert!(
            msgs.len() < 200,
            "expected truncation, got {} messages",
            msgs.len()
        );
        assert_eq!(msgs.last().unwrap().content, "test");
    }
}
