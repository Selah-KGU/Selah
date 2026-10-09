//! Parser for malformed visible pseudo-tool calls emitted by chat models.
//!
//! The real Agent protocol uses JSON plans and registered tools. This parser is
//! only a recovery/safety bridge for model outputs such as
//! `call:view_file {"path":"..."}` or Gemini's `MALFORMED_FUNCTION_CALL`
//! finish messages.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_text;
use crate::agent_tools;

mod detection;
pub(crate) use detection::{find_start, has_any};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct RawToolCall {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

pub(crate) fn parse_leading(answer: &str) -> Option<ToolCall> {
    let visible = agent_text::visible_without_thinking(answer);
    let visible = leading_candidate(&visible)?;
    for marker in agent_text::PSEUDO_TOOL_MARKERS {
        let Some(after_marker) = visible.strip_prefix(marker) else {
            continue;
        };
        let (_raw_name, canonical, rest) = split_tool_name_and_rest(after_marker)?;
        let rest = rest.trim_start();
        let args = if let Some(rest) = rest.strip_prefix('(') {
            let args_body = extract_parenthesized_body(rest)?;
            parse_function_style_args(args_body)
        } else if rest.starts_with('{') {
            let obj = first_json_object(rest)?;
            parse_braced_args(obj)?
        } else if let Some(rest) = rest.strip_prefix('〔') {
            let args_body = extract_until_char(rest, '〕')?;
            parse_braced_args(&format!("{{{args_body}}}"))?
        } else if let Some(rest) = rest.strip_prefix('[') {
            let args_body = extract_until_char(rest, ']')?;
            parse_braced_args(&format!("{{{args_body}}}"))?
        } else if rest.contains('=') {
            parse_loose_key_value_args(rest)
        } else {
            json!({})
        };
        let args = agent_tools::sanitize_tool_args(canonical, &args)?;
        return Some(ToolCall {
            name: canonical.to_string(),
            args,
        });
    }
    None
}

#[cfg(test)]
pub(crate) fn parse_any(answer: &str) -> Option<ToolCall> {
    let visible = agent_text::visible_without_thinking(answer);
    let idx = find_start(&visible)?;
    parse_leading(&visible[idx..])
}

pub(crate) fn parse_any_raw(answer: &str) -> Option<RawToolCall> {
    let visible = agent_text::visible_without_thinking(answer);
    let idx = find_start(&visible)?;
    parse_leading_raw(&visible[idx..])
}

pub(crate) fn starts_with(text: &str) -> bool {
    agent_text::contains_leading_pseudo_tool_call(text)
}

#[cfg(test)]
pub(crate) fn maybe_starts_with_prefix(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }
    for wrapper in ["", "‹", "〈", "<", "```"] {
        for marker in agent_text::PSEUDO_TOOL_MARKERS {
            let candidate = format!("{wrapper}{marker}");
            if candidate.starts_with(&lower) || lower.starts_with(&candidate) {
                return true;
            }
        }
    }
    false
}

fn split_tool_name_and_rest(s: &str) -> Option<(&str, &'static str, &str)> {
    let name_scan_len = s
        .char_indices()
        .take_while(|(_, ch)| {
            ch.is_ascii_alphanumeric() || matches!(*ch, '_' | ':' | '.' | '-' | '/')
        })
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if name_scan_len == 0 {
        return None;
    }

    let mut best: Option<(&str, &'static str, &str)> = None;
    for (idx, _) in s[..name_scan_len].char_indices() {
        if idx == 0 {
            continue;
        }
        let raw_name = &s[..idx];
        if let Some(canonical) = agent_tools::canonical_tool_name(raw_name) {
            best = Some((raw_name, canonical, &s[idx..]));
        }
    }
    let raw_name = &s[..name_scan_len];
    if let Some(canonical) = agent_tools::canonical_tool_name(raw_name) {
        best = Some((raw_name, canonical, &s[name_scan_len..]));
    }
    best
}

fn parse_leading_raw(answer: &str) -> Option<RawToolCall> {
    let visible = agent_text::visible_without_thinking(answer);
    let visible = leading_candidate(&visible)?;
    for marker in agent_text::PSEUDO_TOOL_MARKERS {
        let Some(after_marker) = visible.strip_prefix(marker) else {
            continue;
        };
        let (raw_name, rest) = split_raw_tool_name_and_rest(after_marker)?;
        let rest = rest.trim_start();
        let args = if let Some(rest) = rest.strip_prefix('(') {
            let args_body = extract_parenthesized_body(rest)?;
            parse_function_style_args(args_body)
        } else if rest.starts_with('{') {
            let obj = first_json_object(rest)?;
            parse_braced_args(obj)?
        } else if let Some(rest) = rest.strip_prefix('〔') {
            let args_body = extract_until_char(rest, '〕')?;
            parse_braced_args(&format!("{{{args_body}}}"))?
        } else if let Some(rest) = rest.strip_prefix('[') {
            let args_body = extract_until_char(rest, ']')?;
            parse_braced_args(&format!("{{{args_body}}}"))?
        } else if rest.contains('=') {
            parse_loose_key_value_args(rest)
        } else {
            json!({})
        };
        return Some(RawToolCall {
            name: raw_name.to_string(),
            args,
        });
    }
    None
}

fn split_raw_tool_name_and_rest(s: &str) -> Option<(&str, &str)> {
    let name_scan_len = s
        .char_indices()
        .take_while(|(_, ch)| {
            ch.is_ascii_alphanumeric() || matches!(*ch, '_' | ':' | '.' | '-' | '/')
        })
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if name_scan_len == 0 {
        None
    } else {
        Some((&s[..name_scan_len], &s[name_scan_len..]))
    }
}

fn leading_candidate(text: &str) -> Option<&str> {
    let candidate = agent_text::trim_pseudo_prefixes(text);
    if starts_with(candidate) {
        Some(candidate)
    } else {
        None
    }
}

fn extract_parenthesized_body(s: &str) -> Option<&str> {
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(quote) = in_quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_quote = Some(ch),
            ')' => return Some(&s[..idx]),
            _ => {}
        }
    }
    None
}

fn extract_until_char(s: &str, end_char: char) -> Option<&str> {
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(quote) = in_quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_quote = Some(ch),
            c if c == end_char => return Some(&s[..idx]),
            _ => {}
        }
    }
    None
}

fn parse_function_style_args(s: &str) -> Value {
    let mut out = serde_json::Map::new();
    for part in split_arg_pairs(s) {
        let Some((key, value)) = split_arg_pair(&part) else {
            continue;
        };
        let Some(key) = sanitize_arg_key(key) else {
            continue;
        };
        out.insert(key.to_string(), parse_arg_value(value.trim()));
    }
    Value::Object(out)
}

fn parse_braced_args(s: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(s) {
        return Some(value);
    }
    let body = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut out = serde_json::Map::new();
    for part in split_arg_pairs(body) {
        let Some((key, value)) = split_arg_pair(&part) else {
            continue;
        };
        let Some(key) = sanitize_arg_key(key) else {
            continue;
        };
        out.insert(key.to_string(), parse_arg_value(value.trim()));
    }
    Some(Value::Object(out))
}

fn parse_loose_key_value_args(s: &str) -> Value {
    const KEYS: &[&str] = &[
        "attachment_name",
        "activity_title",
        "activityTitle",
        "amount",
        "code",
        "content",
        "course_name",
        "course_code",
        "date",
        "description",
        "direction",
        "end_time",
        "event_id",
        "file_name",
        "idnumber",
        "key",
        "kgc_code",
        "luna_id",
        "message_id",
        "filename",
        "name",
        "start_time",
        "course",
        "keyword",
        "selector",
        "target",
        "text",
        "timeout_ms",
        "query",
        "title",
        "path",
        "url",
        "limit",
    ];

    let mut positions = Vec::new();
    for key in KEYS {
        let needle = format!("{key}=");
        for (idx, _) in s.match_indices(&needle) {
            positions.push((idx, *key, needle.len()));
        }
    }
    positions.sort_by_key(|(idx, _, _)| *idx);
    positions.dedup_by(|a, b| a.0 == b.0);

    let mut out = serde_json::Map::new();
    for (i, (idx, key, key_len)) in positions.iter().enumerate() {
        let value_start = idx + key_len;
        let value_end = positions
            .get(i + 1)
            .map(|(next_idx, _, _)| *next_idx)
            .unwrap_or_else(|| s.len());
        if value_start > value_end || value_start > s.len() || value_end > s.len() {
            continue;
        }
        let raw_value = s[value_start..value_end]
            .trim()
            .trim_matches(',')
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if raw_value.is_empty() {
            continue;
        }
        let value = if *key == "limit" {
            parse_arg_value(raw_value)
        } else {
            Value::String(raw_value.to_string())
        };
        out.insert((*key).to_string(), value);
    }
    Value::Object(out)
}

fn split_arg_pair(part: &str) -> Option<(&str, &str)> {
    split_arg_pair_on(part, '=')
        .or_else(|| split_arg_pair_on(part, ':'))
        .or_else(|| split_arg_pair_on(part, '：'))
}

fn split_arg_pair_on(part: &str, delimiter: char) -> Option<(&str, &str)> {
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in part.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(quote) = in_quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_quote = Some(ch),
            c if c == delimiter => return Some((&part[..idx], &part[idx + ch.len_utf8()..])),
            _ => {}
        }
    }
    None
}

fn sanitize_arg_key(key: &str) -> Option<&str> {
    let key = key.trim().trim_matches('"').trim_matches('\'');
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        None
    } else {
        Some(key)
    }
}

fn split_arg_pairs(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if let Some(quote) = in_quote {
            if ch == '\\' {
                escaped = true;
                current.push(ch);
            } else if ch == quote {
                in_quote = None;
                current.push(ch);
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_quote = Some(ch);
                current.push(ch);
            }
            ',' => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_string());
    }
    parts
}

fn parse_arg_value(raw: &str) -> Value {
    let unquoted = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')));
    if let Some(value) = unquoted {
        return Value::String(value.replace("\\\"", "\"").replace("\\'", "'"));
    }
    if let Ok(n) = raw.parse::<u64>() {
        return Value::Number(n.into());
    }
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::String(raw.to_string()),
    }
}

fn first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut start: Option<usize> = None;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
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
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return start.map(|st| &s[st..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_keeps_case_sensitive_markers_and_complete_unicode_arguments() {
        let text = "正文\n‹call：read_file {\"path\":\"/tmp/漢字🙂.pdf\"}›";
        let raw = parse_any_raw(text).unwrap();
        assert_eq!(raw.name, "read_file");
        assert_eq!(raw.args["path"], "/tmp/漢字🙂.pdf");
        // Detection accepts ASCII case variants; parsing remains case-sensitive.
        assert!(has_any("CALL:read_file {\"path\":\"/tmp/a.pdf\"}"));
        assert!(parse_any_raw("CALL:read_file {}").is_none());
        let raw = parse_any_raw(
            "<think>call:hidden</think>call:read_file {\"path\":\"/tmp/a.pdf\"}",
        )
        .unwrap();
        assert_eq!(raw.name, "read_file");
        assert_eq!(raw.args["path"], "/tmp/a.pdf");
    }

    #[test]
    fn parses_visible_task_call() {
        let call = parse_leading(
            "‹task_call:download_course_material(luna_id=\"2026341390020201\",filename=\"midterm.pdf\")›",
        )
        .expect("tool call");
        assert_eq!(call.name, "download_course_material");
        assert_eq!(
            call.args.get("filename").and_then(|v| v.as_str()),
            Some("midterm.pdf")
        );
    }

    #[test]
    fn detects_unknown_call_without_parsing() {
        let answer = r#"call:imaginary_file_tool {"path":"/tmp/a.md"}"#;
        assert!(parse_leading(answer).is_none());
        assert!(has_any(answer));
        let raw = parse_any_raw(answer).expect("raw pseudo call");
        assert_eq!(raw.name, "imaginary_file_tool");
        assert_eq!(
            raw.args.get("path").and_then(|v| v.as_str()),
            Some("/tmp/a.md")
        );
    }

    #[test]
    fn detects_split_prefixes_without_local_marker_lists() {
        assert!(maybe_starts_with_prefix("‹task_"));
        assert!(maybe_starts_with_prefix("```call "));
        assert!(!maybe_starts_with_prefix("我看了一下資料"));
    }
}
