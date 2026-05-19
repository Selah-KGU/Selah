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
    /// Extended timeout for slow refresh-style tools.
    slow_tool_timeout_secs: u64,
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
    slow_tool_timeout_secs: 120,
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
    choose_plan(provider, history, user_text, conv_id).await
}

async fn choose_plan(
    provider: &AgentProvider,
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    conv_id: &str,
) -> Plan {
    // Fast path: heuristic covers unambiguous keywords.
    if let Some(plan) = heuristic_plan(history, user_text) {
        return finalize_plan(plan, history, user_text);
    }
    // Slow path: ask model.
    match run_plan_inference(provider, history, user_text, conv_id).await {
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
    conv_id: &str,
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
        "browser_click"
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
            let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let deadline = parsed
                .get("deadline")
                .or_else(|| parsed.get("period"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body_preview = parsed
                .get("body")
                .or_else(|| parsed.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(80).collect::<String>())
                .unwrap_or_default();
            let attachment_count = parsed
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "activity[title={}, deadline={}, attachments={}, body_preview={}]",
                title, deadline, attachment_count, body_preview
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
                                "announce[course={}, title={}]",
                                a.get("course").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("title").and_then(|v| v.as_str()).unwrap_or(""),
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
        let timeout = timeout_for(&call.name);
        let result =
            match tokio::time::timeout(timeout, agent_tools::dispatch(app, &call.name, &call.args))
                .await
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

    let current_names: HashSet<&str> = tool_results.iter().map(|(n, _)| n.as_str()).collect();
    let recent: Vec<(String, String)> = recent_tool_results(history, CFG.recent_tool_context)
        .into_iter()
        .filter(|(name, _)| !current_names.contains(name.as_str()))
        .collect();
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
const KGC_PREFIX_WHITELIST: &[&str] = &[
    "AB", "AE", "AL", "AS", "BL", "BU", "CO", "CS", "DC", "EC", "ED", "EN", "FD", "GE", "GS", "HS",
    "HU", "IB", "IC", "IS", "JP", "LA", "LB", "LE", "LI", "LR", "LS", "MA", "MD", "ME", "MM", "MS",
    "NS", "PA", "PE", "PH", "PL", "PO", "PS", "RC", "RE", "SC", "SD", "SO", "SP", "ST", "TA", "TC",
    "TH", "TM", "TS", "UC",
];

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
        // Smoke check: every name we expose in TOOL_SPECS must come back is_known.
        // Catches the inverse of the panic we removed — a tool listed but not
        // wired (or vice versa).
        for name in [
            "list_today_classes",
            "list_week_classes",
            "search_courses",
            "get_course_context",
            "get_course_detail",
            "get_cancellations",
            "get_makeup_classes",
            "get_room_changes",
            "get_exam_timetable",
            "list_luna_todos",
            "get_grades",
            "get_registration",
            "list_syllabus_favorites",
            "list_recent_notifications",
            "search_notifications",
            "get_notification_detail",
            "list_recent_mail",
            "read_mail",
            "search_mail",
            "list_luna_announcements",
            "get_mail_profile",
            "get_student_profile",
            "get_weather",
            "get_weekly_summary",
            "get_todo_guide",
            "get_upcoming_deadlines",
            "get_luna_activity_detail",
            "refresh_data",
            "list_downloaded_files",
            "read_downloaded_file",
            "inspect_file",
            "write_downloaded_text_file",
            "open_downloaded_file",
            "delete_downloaded_file",
            "download_url",
            "open_luna_attachment",
            "download_luna_attachment",
            "list_browser_windows",
            "open_browser_url",
            "read_browser_page",
            "browser_back",
            "browser_forward",
            "browser_reload_page",
            "browser_click",
            "browser_fill",
            "browser_select_option",
            "browser_press",
            "browser_scroll",
            "browser_wait_for",
            "browser_close",
            "get_today_brief",
        ] {
            assert!(
                agent_tools::is_known_tool(name),
                "tool {} missing from TOOL_SPECS",
                name
            );
        }
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
