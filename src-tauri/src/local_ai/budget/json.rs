use super::tokens::estimate_apple_tokens;

/// Shrink a JSON value without cutting through an object, array, or string.
/// Arrays lose whole items. Long strings are shortened inside the quotes.
pub(crate) fn compact_json_value(value: &serde_json::Value, budget: usize) -> serde_json::Value {
    if budget < 2 {
        return serde_json::Value::Null;
    }
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    if estimate_apple_tokens(&raw) <= budget {
        return value.clone();
    }
    match value {
        serde_json::Value::String(text) => {
            serde_json::Value::String(truncate_json_string(text, budget))
        }
        serde_json::Value::Array(items) => compact_json_array(items, budget),
        serde_json::Value::Object(map) => compact_json_object(map, budget),
        other => {
            if estimate_apple_tokens(&raw) <= budget {
                other.clone()
            } else {
                serde_json::Value::Null
            }
        }
    }
}

fn compact_json_array(items: &[serde_json::Value], budget: usize) -> serde_json::Value {
    let mut kept = Vec::new();
    for item in items {
        let used = serde_json::to_string(&serde_json::Value::Array(kept.clone()))
            .map(|text| estimate_apple_tokens(&text))
            .unwrap_or(2);
        let remaining = budget.saturating_sub(used).saturating_sub(1);
        if remaining < 2 {
            break;
        }
        let shrunk = compact_json_value(item, remaining);
        let mut trial = kept.clone();
        trial.push(shrunk.clone());
        let rendered =
            serde_json::to_string(&serde_json::Value::Array(trial)).unwrap_or_else(|_| "[]".into());
        if estimate_apple_tokens(&rendered) > budget {
            break;
        }
        kept.push(shrunk);
    }
    serde_json::Value::Array(kept)
}

fn compact_json_object(
    map: &serde_json::Map<String, serde_json::Value>,
    budget: usize,
) -> serde_json::Value {
    // Keep smaller fields first so a large array is shortened instead of deleted.
    let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
    entries.sort_by_key(|(_, value)| {
        serde_json::to_string(value)
            .map(|text| text.len())
            .unwrap_or(0)
    });
    let mut kept = serde_json::Map::new();
    for (key, value) in entries {
        let used = serde_json::to_string(&serde_json::Value::Object(kept.clone()))
            .map(|text| estimate_apple_tokens(&text))
            .unwrap_or(2);
        let key_cost = estimate_apple_tokens(key) + 3;
        let remaining = budget.saturating_sub(used).saturating_sub(key_cost);
        if remaining < 2 {
            continue;
        }
        let shrunk = compact_json_value(value, remaining);
        let mut trial = kept.clone();
        trial.insert(key.clone(), shrunk.clone());
        let rendered = serde_json::to_string(&serde_json::Value::Object(trial))
            .unwrap_or_else(|_| "{}".into());
        if estimate_apple_tokens(&rendered) > budget {
            continue;
        }
        kept.insert(key.clone(), shrunk);
    }
    serde_json::Value::Object(kept)
}
fn truncate_json_string(text: &str, budget: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high + 1) / 2;
        let candidate: String = chars[..mid].iter().collect();
        let rendered = serde_json::to_string(&serde_json::Value::String(candidate))
            .unwrap_or_else(|_| "\"\"".into());
        if estimate_apple_tokens(&rendered) <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut out: String = chars[..low].iter().collect();
    if low < chars.len()
        && estimate_apple_tokens(
            &serde_json::to_string(&serde_json::Value::String(format!("{out}…")))
                .unwrap_or_default(),
        ) <= budget
    {
        out.push('…');
    }
    out
}

pub(in crate::local_ai) fn outermost_brace_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    let mut index = 0usize;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or('\0');
        let len = ch.len_utf8();
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += len;
            continue;
        }
        if ch == '"' {
            in_string = true;
            index += len;
            continue;
        }
        if ch == '{' || ch == '[' {
            if let Some(end) = matching_close(text, index) {
                spans.push((index, end));
                index = end;
                continue;
            }
        }
        index += len;
    }
    spans
}

pub(in crate::local_ai) fn matching_close(text: &str, open_byte: usize) -> Option<usize> {
    let opener = text[open_byte..].chars().next()?;
    let closer = if opener == '{' { '}' } else { ']' };
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut index = open_byte;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        let len = ch.len_utf8();
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return (ch == closer).then_some(index + len);
                    }
                }
                _ => {}
            }
        }
        index += len;
    }
    None
}

pub(in crate::local_ai) fn compact_embedded_json(text: &str, span_budget: usize) -> String {
    let spans = outermost_brace_spans(text);
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut cursor = 0usize;
    for (start, end) in spans {
        out.push_str(&text[cursor..start]);
        let span = &text[start..end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(span) {
            if estimate_apple_tokens(span) > span_budget {
                let compact = compact_json_value(&value, span_budget.max(16));
                out.push_str(&serde_json::to_string(&compact).unwrap_or_else(|_| "{}".into()));
            } else {
                out.push_str(span);
            }
        } else {
            out.push_str(span);
        }
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}
