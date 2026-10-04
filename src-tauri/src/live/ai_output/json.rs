//! JSON extraction and text clamping for live model replies.

pub fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                in_string = false;
            }
            continue;
        }
        match *b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(start..=idx);
                }
            }
            _ => {}
        }
    }
    None
}

/// Best-effort repair of the first JSON object in a model reply. Two failure
/// modes get common once a long lecture (80+ min) makes the reply large:
///   1. literal control characters (newlines/tabs) inside string values — a
///      markdown body routinely contains real newlines, which are invalid JSON;
///   2. truncation — the reply hit the token ceiling mid-object, leaving
///      strings/arrays/objects unclosed.
/// We scan from the first `{`, escaping raw control chars inside strings and
/// tracking the bracket stack, then close whatever is still open at the end.
/// Already-valid JSON round-trips unchanged. Returns None when there is no `{`.
pub fn repair_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut out = String::with_capacity(text.len() - start + 16);
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in text[start..].chars() {
        if in_string {
            if escaped {
                out.push(ch);
                escaped = false;
                continue;
            }
            match ch {
                '\\' => {
                    out.push(ch);
                    escaped = true;
                }
                '"' => {
                    out.push(ch);
                    in_string = false;
                }
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                // Drop other raw control chars; they are never valid in a string.
                c if (c as u32) < 0x20 => {}
                c => out.push(c),
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '{' | '[' => {
                stack.push(ch);
                out.push(ch);
            }
            '}' | ']' => {
                stack.pop();
                out.push(ch);
                if stack.is_empty() {
                    // Completed the first top-level object — ignore any trailing junk.
                    return Some(out);
                }
            }
            c => out.push(c),
        }
    }

    // Truncated mid-object: close everything that is still open.
    if in_string {
        // A lone trailing backslash would otherwise escape our closing quote.
        if escaped {
            out.push('\\');
        }
        out.push('"');
    }
    // Trim a dangling separator / empty key:value fragment so the synthetic
    // close produces parseable JSON instead of `,}` or `:}`.
    let trimmed_len = out.trim_end().len();
    out.truncate(trimmed_len);
    if out.ends_with(',') {
        out.pop();
    } else if out.ends_with(':') {
        out.push_str("null");
    }
    while let Some(open) = stack.pop() {
        out.push(if open == '{' { '}' } else { ']' });
    }
    Some(out)
}

/// Lift a single string field out of a JSON-ish blob by hand, for when the
/// object can't be parsed even after [`repair_json_object`]. Unescapes the
/// value so we render the real summary text rather than a raw JSON dump. Works
/// even if the value was truncated before its closing quote.
pub fn salvage_json_string_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let key_pos = text.find(&needle)?;
    let after = &text[key_pos + needle.len()..];
    let colon = after.find(':')?;
    let mut chars = after[colon + 1..].trim_start().chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            match ch {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                // \" \\ \/ and any other escape: keep the literal char.
                other => out.push(other),
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out), // reached the closing quote
            c => out.push(c),
        }
    }
    // Truncated before the closing quote — keep what we recovered.
    Some(out)
}

pub fn value_to_trimmed_string(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub fn clamp_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}
