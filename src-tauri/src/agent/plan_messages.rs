//! Planner prompt assembly.
//!
//! Turns history, the attached browser pane, and recent screenshots into the
//! ChatML messages sent to the planning model.

use super::*;

/// Build the ChatML message list for the planner.  Pure function — does not
/// touch the model or database, so it can be unit-tested.
#[cfg(test)]
pub(super) fn build_plan_messages(
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
pub(super) fn recent_screenshot_images(
    history: &[crate::db::AgentMessageRow],
    limit: usize,
) -> Vec<ImagePart> {
    let mut out = Vec::new();
    if limit == 0 {
        return out;
    }
    for row in history.iter().rev() {
        if row.role != "tool" {
            continue;
        }
        let Some(json) = row.tool_result_json.as_deref() else {
            continue;
        };
        if let Some(image) = tool_result::screenshot_image(json) {
            out.push(image);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

pub(super) fn build_plan_messages_with_note(
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

pub(super) fn append_browser_context(
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
