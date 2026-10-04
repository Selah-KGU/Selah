use super::super::{APPLE_CONTEXT_OVERHEAD_TOKENS, APPLE_CONTEXT_WINDOW_TOKENS};

/// Apple Intelligence counts about one token per CJK character and ~3 ASCII characters.
pub(crate) fn estimate_apple_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    other + ascii.div_ceil(3) + 1
}

/// requested == 0 means "use the room left in the window", never unlimited.
pub(crate) fn apple_response_limit(requested: u32, input_tokens: usize) -> u32 {
    let room = APPLE_CONTEXT_WINDOW_TOKENS
        .saturating_sub(APPLE_CONTEXT_OVERHEAD_TOKENS)
        .saturating_sub(input_tokens)
        .clamp(
            192,
            APPLE_CONTEXT_WINDOW_TOKENS - APPLE_CONTEXT_OVERHEAD_TOKENS,
        ) as u32;
    if requested == 0 {
        room
    } else {
        requested.min(room).max(32)
    }
}
