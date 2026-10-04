use super::super::{
    APPLE_CONTEXT_OVERHEAD_TOKENS, APPLE_CONTEXT_WINDOW_TOKENS, APPLE_JSON_RESPONSE_RESERVE_TOKENS,
    APPLE_RESPONSE_RESERVE_TOKENS,
};
use super::tokens::{apple_response_limit, estimate_apple_tokens};
use super::trim::{trim_apple_text, trim_apple_turns};

pub(crate) struct FittedAppleRequest {
    pub instructions: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub input_tokens: usize,
    pub trimmed: bool,
}

/// Fit instructions and the conversation into the Apple window before the bridge runs.
/// Every local caller, including timetable and Live, goes through this.
pub(crate) fn fit_apple_request(
    instructions: &str,
    prompt: &str,
    requested_max_tokens: u32,
) -> FittedAppleRequest {
    let budget = prompt_budget_for(&instructions, &prompt);
    let mut instructions = instructions.trim().to_string();
    let mut prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        prompt = "応答してください。".to_string();
    }
    let before = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
    if before > budget {
        let prompt_floor = 512usize.min(budget.saturating_sub(256));
        let prompt_budget = (budget / 2).clamp(prompt_floor, budget.saturating_sub(256));
        prompt = trim_apple_turns(&prompt, prompt_budget);
        let instruction_budget = budget
            .saturating_sub(estimate_apple_tokens(&prompt))
            .max(128);
        if estimate_apple_tokens(&instructions) > instruction_budget {
            instructions = trim_apple_text(&instructions, instruction_budget);
        }
        let used = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
        if used > budget {
            let tighter = budget
                .saturating_sub(estimate_apple_tokens(&instructions))
                .max(64);
            prompt = trim_apple_turns(&prompt, tighter);
        }
    }
    let mut input_tokens = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
    if input_tokens > budget {
        prompt = trim_apple_turns(&prompt, budget / 2);
        let instruction_budget = budget.saturating_sub(estimate_apple_tokens(&prompt));
        instructions = trim_apple_text(&instructions, instruction_budget);
        input_tokens = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
    }
    FittedAppleRequest {
        instructions,
        prompt,
        max_tokens: apple_response_limit(requested_max_tokens, input_tokens),
        input_tokens,
        trimmed: before > budget,
    }
}

fn json_reply_requested(instructions: &str, prompt: &str) -> bool {
    instructions.contains("JSONのみ")
        || instructions.contains("JSONだけ")
        || instructions.contains("Output one JSON")
        || instructions.contains("出力は必ず")
        || prompt.contains("JSONのみ")
        || prompt.contains("JSONだけ")
}

fn prompt_budget_for(instructions: &str, prompt: &str) -> usize {
    let reserve = if json_reply_requested(instructions, prompt) {
        APPLE_JSON_RESPONSE_RESERVE_TOKENS
    } else {
        APPLE_RESPONSE_RESERVE_TOKENS
    };
    APPLE_CONTEXT_WINDOW_TOKENS
        .saturating_sub(APPLE_CONTEXT_OVERHEAD_TOKENS)
        .saturating_sub(reserve)
        .max(512)
}
