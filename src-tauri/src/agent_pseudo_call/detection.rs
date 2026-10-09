use crate::agent_text;

#[cfg(test)]
#[path = "detection/tests.rs"]
mod tests;

pub(crate) fn has_any(answer: &str) -> bool {
    let visible = agent_text::visible_without_thinking(answer);
    find_start(&visible).is_some()
}

/// Preserve the earliest eligible boundary through whitespace and wrappers.
/// Each character is visited once; marker checks borrow only a fixed prefix.
pub(crate) fn find_start(text: &str) -> Option<usize> {
    let mut candidate_start = None;
    let mut boundary = true;
    for (idx, ch) in text.char_indices() {
        if boundary {
            candidate_start.get_or_insert(idx);
        }
        if ch.is_whitespace() || matches!(ch, '`' | '<' | '‹' | '〈') {
            boundary = true;
            continue;
        }
        if let Some(start) = candidate_start {
            if agent_text::has_pseudo_marker_prefix(&text[idx..]) {
                return Some(start);
            }
        }
        candidate_start = None;
        boundary = matches!(ch, '(' | '[' | '{' | '"' | '\'' | '「' | '『');
    }
    None
}
