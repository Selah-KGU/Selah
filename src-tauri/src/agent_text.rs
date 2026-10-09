//! Shared text-protocol helpers for the Agent pipeline.
//!
//! Provider adapters, planning, and UI guards all need to recognize the same
//! thinking blocks and malformed pseudo-tool-call markers. Keeping those rules
//! here prevents subtle drift between streaming, parsing, and sanitization.

pub(crate) const PSEUDO_TOOL_MARKERS: &[&str] = &[
    "task_call:",
    "task_call：",
    "tool_call:",
    "tool_call：",
    "function_call:",
    "function_call：",
    "call:",
    "call：",
    "call ",
];

pub(crate) const THINKING_START_PREFIXES: &[&str] = &["<thinking", "<thought", "<think"];
pub(crate) const THINKING_END_TAGS: &[&str] = &["</think>", "</thought>", "</thinking>"];
pub(crate) const THINKING_TAG_HOLDBACK: usize = 10;

pub(crate) fn strip_think(s: &str) -> String {
    visible_without_thinking(s).into_owned()
}

pub(crate) fn visible_without_thinking(s: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let Some(first) = find_thinking_start_tag(s) else {
        return Cow::Borrowed(s);
    };
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut next = Some(first);
    while let Some((start, start_len)) = next {
        out.push_str(&rest[..start]);
        match find_thinking_end_tag(&rest[start + start_len..]) {
            Some((end_rel, end_len)) => {
                rest = &rest[start + start_len + end_rel + end_len..];
            }
            None => {
                rest = "";
                break;
            }
        }
        next = find_thinking_start_tag(rest);
    }
    out.push_str(rest);
    Cow::Owned(out)
}

pub(crate) fn find_thinking_start_tag(s: &str) -> Option<(usize, usize)> {
    THINKING_START_PREFIXES
        .iter()
        .filter_map(|prefix| {
            s.find(prefix).map(|idx| {
                let after_prefix = idx + prefix.len();
                let tag_len = if s[after_prefix..].starts_with('>') {
                    prefix.len() + 1
                } else {
                    prefix.len()
                };
                (idx, tag_len)
            })
        })
        .min_by_key(|(idx, _)| *idx)
}

pub(crate) fn find_thinking_end_tag(s: &str) -> Option<(usize, usize)> {
    find_earliest_tag(s, THINKING_END_TAGS)
}

pub(crate) fn find_earliest_tag(s: &str, tags: &[&str]) -> Option<(usize, usize)> {
    tags.iter()
        .filter_map(|tag| s.find(tag).map(|idx| (idx, tag.len())))
        .min_by_key(|(idx, _)| *idx)
}

/// Return the byte index up to which it's safe to emit, keeping `keep` bytes
/// in reserve at the tail so a partial tag is not cut in half.
pub(crate) fn holdback(s: &str, keep: usize) -> usize {
    if s.len() <= keep {
        return 0;
    }
    let cutoff = s.len() - keep;
    let mut idx = cutoff;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

pub(crate) fn neutralize_pseudo_tool_calls(text: &str) -> String {
    neutralized_tool_text(text).into_owned()
}

const TOOL_TEXT_REPLACEMENTS: [(&str, &str); 9] = [
    ("<call:", "<call："),
    ("</call:", "</call："),
    ("task_call:", "task_call："),
    ("tool_call:", "tool_call："),
    ("function_call:", "function_call："),
    ("call:", "call："),
    ("‹", "〈"),
    ("›", "〉"),
    ("MALFORMED_FUNCTION_CALL", "MALFORMED FUNCTION CALL"),
];

pub(crate) fn neutralized_tool_prefix(text: &str, byte_limit: usize) -> std::borrow::Cow<'_, str> {
    // Current replacements never shorten UTF-8. Read through the longest
    // marker after the requested prefix so boundary-crossing calls are still
    // neutralized. If a future rule shrinks text, retain the full-input path.
    let mut end = if TOOL_TEXT_REPLACEMENTS
        .iter()
        .all(|(from, to)| to.len() >= from.len())
    {
        byte_limit
            .saturating_add(3)
            .saturating_add(
                TOOL_TEXT_REPLACEMENTS
                    .iter()
                    .map(|(from, _)| from.len())
                    .max()
                    .unwrap_or(0),
            )
            .min(text.len())
    } else {
        text.len()
    };
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    neutralized_tool_text(&text[..end])
}

pub(crate) fn neutralized_tool_text(text: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let mut text = Cow::Borrowed(text);
    for (from, to) in TOOL_TEXT_REPLACEMENTS {
        if text.contains(from) {
            text = Cow::Owned(text.replace(from, to));
        }
    }
    text
}

pub(crate) fn extract_pseudo_call_from_text(text: &str) -> Option<String> {
    let text = text
        .trim()
        .strip_prefix("Malformed function call:")
        .unwrap_or(text)
        .trim();
    let start = PSEUDO_TOOL_MARKERS
        .iter()
        .filter_map(|marker| text.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)
        .map(|(index, _)| index)?;
    let call = trim_pseudo_wrappers(&text[start..]);
    if call.is_empty() {
        None
    } else {
        Some(call.to_string())
    }
}

pub(crate) fn contains_leading_pseudo_tool_call(text: &str) -> bool {
    has_pseudo_marker_prefix(trim_pseudo_prefixes(text))
}

pub(crate) fn has_pseudo_marker_prefix(candidate: &str) -> bool {
    PSEUDO_TOOL_MARKERS.iter().any(|marker| {
        candidate
            .as_bytes()
            .get(..marker.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(marker.as_bytes()))
    })
}

pub(crate) fn trim_pseudo_prefixes(text: &str) -> &str {
    let mut candidate = text.trim_start();
    loop {
        let next = candidate
            .strip_prefix('`')
            .or_else(|| candidate.strip_prefix('<'))
            .or_else(|| candidate.strip_prefix('‹'))
            .or_else(|| candidate.strip_prefix('〈'))
            .map(str::trim_start);
        match next {
            Some(rest) => candidate = rest,
            None => return candidate,
        }
    }
}

fn trim_pseudo_wrappers(text: &str) -> &str {
    text.trim()
        .trim_matches('`')
        .trim_matches('<')
        .trim_matches('>')
        .trim_matches('‹')
        .trim_matches('›')
        .trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_think_blocks() {
        assert_eq!(
            strip_think("<think>reasoning</think>{\"tools\":[]}"),
            "{\"tools\":[]}"
        );
        assert_eq!(
            strip_think("<thought>reasoning</thought>visible"),
            "visible"
        );
        assert_eq!(
            strip_think("before<thinking>reasoning</thinking>after"),
            "beforeafter"
        );
        assert_eq!(strip_think("<thoughtThe user wants tools"), "");
        assert_eq!(strip_think("no tags here"), "no tags here");
    }

    #[test]
    fn extracts_wrapped_pseudo_call() {
        assert_eq!(
            extract_pseudo_call_from_text(
                "Malformed function call: ‹call：read_file〔path: \"/tmp/a.pdf\"〕›"
            )
            .as_deref(),
            Some("call：read_file〔path: \"/tmp/a.pdf\"〕")
        );
    }

    #[test]
    fn leading_detector_ignores_normal_words() {
        assert!(!contains_leading_pseudo_tool_call("callback: done"));
        assert!(contains_leading_pseudo_tool_call("‹task_call:download("));
    }
}
