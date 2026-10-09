//! Streaming answer phase.
//!
//! Builds the answer prompt, holds back pseudo tool-call syntax, and repairs
//! a visible tool call before the reply is persisted.

use super::*;

// ─────────────────────── Phase 2: Answer ───────────────────────

#[allow(clippy::too_many_arguments)] // Phase inputs are intentionally explicit for call-site auditability.
pub(super) async fn answer_phase(
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
pub(super) async fn answer_phase_with_repair(
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

pub(super) fn pseudo_tool_repair_note(raw: Option<&RawToolCall>, answer: &str) -> String {
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

pub(super) fn pseudo_tool_repair_failed_message(
    user_text: &str,
    _raw: Option<&RawToolCall>,
) -> String {
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
    owner: Option<std::sync::Arc<crate::agent_turn_scope::Turn>>,
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
        let owner = crate::agent_turn_scope::current(&conv_id);
        Self {
            owner,
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
        emit_owned(&self.app, &self.conv_id, &ev, self.owner.as_deref());
    }
}

pub(super) fn safe_visible_emit_len(s: &str, hold_chars: usize) -> usize {
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
pub(super) enum VisibleStart {
    Normal,
    MaybePseudoCall,
    PseudoCall,
}

#[cfg(test)]
pub(super) fn classify_visible_stream_start(buffer: &str) -> VisibleStart {
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
pub(super) fn build_answer_messages(
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
        local_text(
            user_text,
            if turn_context.has_documents {
                1800
            } else {
                400
            },
        )
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
            let rendered = if local {
                render_local_json(&sanitize_answer_tool_result(value), 180)
            } else {
                tool_result::json_prefix(value, tool_result_chars)
            };
            system.push_str(&format!("[{}] {}\n", name, rendered));
        }
        system.push_str("</tool_results>\n");
    }

    let current_names: HashSet<&str> = tool_results.iter().map(|(n, _)| n.as_str()).collect();
    let recent_limit = if local { 1 } else { CFG.recent_tool_context };
    let recent: Vec<(&str, &str)> = recent_tool_results(history, recent_limit)
        .into_iter()
        .filter(|(name, _)| !current_names.contains(name))
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
                match serde_json::from_str::<Value>(json) {
                    Ok(value) => tool_result::json_prefix(&value, recent_chars),
                    Err(_) => {
                        // Preserve the existing literal fallback for malformed
                        // history. Its input is already bounded before encoding.
                        let text = Value::String(trim_to(json, recent_chars));
                        let encoded = serde_json::to_string(&text).unwrap_or_else(|_| "{}".into());
                        trim_to(&encoded, recent_chars)
                    }
                }
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
pub(super) fn estimate_tokens(text: &str) -> usize {
    text.len() / 3 + 1
}

fn recent_tool_results(history: &[crate::db::AgentMessageRow], limit: usize) -> Vec<(&str, &str)> {
    history
        .iter()
        .rev()
        .filter_map(|row| {
            if row.role != "tool" {
                return None;
            }
            Some((row.tool_name.as_deref()?, row.tool_result_json.as_deref()?))
        })
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub(super) fn sanitize_answer_tool_result(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if tool_result::hidden_field(key) {
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
