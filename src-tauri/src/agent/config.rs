// ─────────────────────── Date/Time Context ───────────────────────

/// Builds a one-line date/time context string in JST.
/// Used by both the planner and answer phases so the model understands
/// relative time references (今日, 明日, 来週, etc.).
pub fn datetime_context() -> String {
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
pub fn tomorrow_week_offset() -> i32 {
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
pub struct AgentConfig {
    /// Max historical messages (excluding the new user turn) in Phase 2.
    pub(super) history_window: usize,
    /// Max tools executed per turn.
    pub(super) max_tools: usize,
    /// Temperature for Phase 1 (planning) — low for determinism.
    pub(super) plan_temperature: f32,
    /// Max tokens for Phase 1 output.
    pub(super) plan_max_tokens: u32,
    /// Phase 1 think budget percentage.
    pub(super) plan_think_budget_pct: u32,
    /// Number of recent history turns fed into Phase 1.
    pub(super) plan_history_turns: usize,
    /// Max chars for a persisted tool result summary in the planning prompt.
    pub(super) plan_tool_result_chars: usize,
    /// Prefill injected into the assistant turn for Phase 1.
    pub(super) plan_prefill: &'static str,
    /// Think budget percentage for Phase 2.
    pub(super) answer_think_budget_pct: u32,
    /// Rough prompt token budget (chars / 3).
    pub(super) prompt_token_budget: usize,
    /// Max chars for a single tool result in the answer prompt.
    pub(super) tool_result_chars: usize,
    /// Max chars for recent (prior-turn) tool results in the answer prompt.
    pub(super) recent_tool_result_chars: usize,
    /// Recent persisted tool results exposed as follow-up context.
    pub(super) recent_tool_context: usize,
    /// Bytes shown in the tool_result event preview.
    pub(super) preview_bytes: usize,
    /// Hard timeout for a single tool execution.
    pub(super) tool_timeout_secs: u64,
    /// Extended timeout for slow refresh-style tools.
    pub(super) slow_tool_timeout_secs: u64,
    /// Hard timeout for answer generation so the UI cannot stay in thinking forever.
    pub(super) answer_timeout_secs: u64,
    /// How many times Phase 2 may retry after emitting an invalid pseudo-tool call.
    pub(super) max_answer_repairs: usize,
    /// How many times Phase 1 may retry after selecting unknown/invalid tools.
    pub(super) max_plan_repairs: usize,
    /// Max adaptive plan→execute→observe steps per turn (incl. the first plan).
    /// Enables "act, observe, re-plan" instead of failing on the first problem.
    pub(super) max_agent_steps: usize,
}

/// Tools that are known to take much longer than `tool_timeout_secs` because
/// they hit the network across many courses. Returning a timeout for them
/// while the work continues in the background creates "failed but actually
/// succeeded" inconsistencies, so they get their own ceiling.
const SLOW_TOOLS: &[&str] = &["refresh_data", "download_url"];

pub fn timeout_for(tool: &str) -> std::time::Duration {
    let secs = if SLOW_TOOLS.contains(&tool) {
        CFG.slow_tool_timeout_secs
    } else {
        CFG.tool_timeout_secs
    };
    std::time::Duration::from_secs(secs)
}

pub const CFG: AgentConfig = AgentConfig {
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
