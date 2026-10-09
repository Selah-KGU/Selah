// Frozen pre-scan detectors for differential tests and isolated benchmarks.
use crate::agent_text;

fn strip_think(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some((start, start_len)) = agent_text::find_thinking_start_tag(rest) {
        out.push_str(&rest[..start]);
        match agent_text::find_thinking_end_tag(&rest[start + start_len..]) {
            Some((end_rel, end_len)) => {
                rest = &rest[start + start_len + end_rel + end_len..];
            }
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

pub(crate) fn find_start(text: &str) -> Option<usize> {
    for (idx, _) in text.char_indices() {
        if !is_boundary(text, idx) {
            continue;
        }
        if leading_candidate(&text[idx..]).is_some() {
            return Some(idx);
        }
    }
    None
}

fn is_boundary(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    text[..idx]
        .chars()
        .last()
        .map(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '<' | '‹' | '〈' | '`' | '(' | '[' | '{' | '"' | '\'' | '「' | '『'
                )
        })
        .unwrap_or(true)
}

pub(crate) fn contains_leading(text: &str) -> bool {
    let candidate = agent_text::trim_pseudo_prefixes(text).to_ascii_lowercase();
    agent_text::PSEUDO_TOOL_MARKERS
        .iter()
        .any(|marker| candidate.starts_with(marker))
}
fn leading_candidate(text: &str) -> Option<&str> {
    let candidate = agent_text::trim_pseudo_prefixes(text);
    if contains_leading(candidate) {
        Some(candidate)
    } else {
        None
    }
}
pub(crate) fn has_any(answer: &str) -> bool {
    let visible = strip_think(answer);
    find_start(&visible).is_some()
}
pub(crate) fn visible(answer: &str) -> String {
    strip_think(answer)
}
