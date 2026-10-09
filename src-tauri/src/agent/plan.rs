use super::*;

// ─────────────────────── Phase 1: Planning ───────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Plan {
    #[serde(default)]
    pub(super) tools: Vec<ToolCall>,
    #[serde(default)]
    pub(super) image_only: bool,
}

pub async fn plan_phase(
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

pub fn deterministic_preplan(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
    turn_context: &AgentTurnContext,
) -> Option<Plan> {
    if should_skip_tools(history, user_text) {
        return Some(Plan::default());
    }
    attached_browser_control_plan(&normalize_planner_text(user_text), turn_context)
}

pub fn planner_failure_fallback(
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

pub fn should_retry_empty_plan(
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
pub async fn plan_next_step(
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
pub fn agent_loop_should_continue(
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
        "[agent plan] user_text={:?} context_tool_rows={}",
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

// ─────────────────────── Plan Parsing ───────────────────────

pub fn parse_plan(raw: &str) -> Result<Plan, String> {
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
