use super::{Html, SelectOption, Selector, SEL_OPTION, SEL_OPT_SELECTED};

/// Extract text from a Quill Delta JSON string, preserving link URLs.
/// The json_str is double-escaped: it was a JS string literal containing JSON.
/// E.g. in the HTML: setJsonData("{\"ops\":[{\"insert\":\"text\\n\u306F\"}]}", ...)
/// So json_str = {\"ops\":[{\"insert\":\"text\\n\u306F\"}]}
///
/// Links in Quill Delta look like:
///   {"insert":"click here","attributes":{"link":"https://example.com"}}
/// We output them as: click here ( https://example.com )
pub(super) fn extract_quill_plain_text(json_str: &str) -> Option<String> {
    // First, try proper JSON parsing after unescaping
    if let Some(text) = extract_quill_via_json(json_str) {
        return Some(text);
    }

    // Fallback: string-level extraction (no link support)
    let mut result = String::new();
    let marker = "\\\"insert\\\":\\\"";
    let mut search = json_str;

    while let Some(pos) = search.find(marker) {
        let rest = &search[pos + marker.len()..];
        if let Some(end_pos) = find_closing_escaped_quote(rest) {
            let raw_value = &rest[..end_pos];
            let pass1 = unescape_js_string(raw_value);
            let pass2 = unescape_js_string(&pass1);
            result.push_str(&pass2);
            search = if end_pos + 2 < rest.len() {
                &rest[end_pos + 2..]
            } else {
                ""
            };
        } else {
            break;
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Try to parse Quill Delta JSON properly via serde_json.
/// Handles link attributes by appending URLs after the link text.
pub(super) fn extract_quill_via_json(json_str: &str) -> Option<String> {
    // Unescape the JS string to get valid JSON
    let unescaped = unescape_js_string(json_str);

    // Try parsing as JSON
    let val: serde_json::Value = serde_json::from_str(&unescaped).ok()?;
    let ops = val.get("ops")?.as_array()?;

    let mut result = String::new();
    for op in ops {
        if let Some(text) = op.get("insert").and_then(|v| v.as_str()) {
            let link = op
                .get("attributes")
                .and_then(|a| a.get("link"))
                .and_then(|l| l.as_str());

            result.push_str(text);
            if let Some(url) = link {
                // Append link URL after the text, avoiding duplication if text IS the URL
                if text.trim() != url.trim() {
                    result.push_str(" ( ");
                    result.push_str(url);
                    result.push_str(" )");
                }
            }
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Find the position of the closing \" in a JS-escaped string value.
/// Skips past \\\\ (escaped backslash) so \\\\\" is read as \\\\ + \" (end).
pub(super) fn find_closing_escaped_quote(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'"' {
                return Some(i); // found \"
            }
            // skip any escape sequence (\\, \n, \u, etc.)
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

/// Unescape one level of JS/JSON string escaping:
/// \\n → newline, \\t → tab, \\\\ → \\, \\/ → /, \\uXXXX → char
pub(super) fn unescape_js_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('t') => {
                    chars.next();
                    result.push('\t');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('"') => {
                    chars.next();
                    result.push('"');
                }
                Some('/') => {
                    chars.next();
                    result.push('/');
                }
                Some('u') => {
                    chars.next();
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() == 4 {
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                                continue;
                            }
                        }
                    }
                    result.push_str("\\u");
                    result.push_str(&hex);
                }
                _ => {
                    result.push(c);
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Extract setJsonData content from an HTML fragment
pub(super) fn extract_quill_text(html: &str) -> Option<String> {
    // Pattern: setJsonData("{...}", 'reference') or setJsonData("{...}");
    let marker = "setJsonData(\"";
    let pos = html.find(marker)?;
    let rest = &html[pos + marker.len()..];
    // Find the closing: try "\", '" first, then "\");"
    let end = rest.find("\", '").or_else(|| rest.find("\");"))?;
    let json_str = &rest[..end];
    extract_quill_plain_text(json_str)
}

/// Try multiple CSS selectors and return the first non-empty text match.
pub(super) fn try_selectors_text(doc: &Html, selectors: &[&str]) -> String {
    for sel_str in selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            if let Some(el) = doc.select(&sel).next() {
                let text = el.text().collect::<String>().trim().to_string();
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

// ──────────────────────────────────────────────
// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

pub(super) fn extract_selected_value(doc: &Html, selector: &str) -> (String, String) {
    let sel = match Selector::parse(selector) {
        Ok(s) => s,
        Err(_) => return (String::new(), String::new()),
    };
    let select_el = match doc.select(&sel).next() {
        Some(e) => e,
        None => return (String::new(), String::new()),
    };
    let option_sel = &*SEL_OPT_SELECTED;
    match select_el.select(option_sel).next() {
        Some(opt) => {
            let value = opt.value().attr("value").unwrap_or_default().to_string();
            let label = opt.text().collect::<String>().trim().to_string();
            (value, label)
        }
        None => (String::new(), String::new()),
    }
}

pub(super) fn extract_select_options(doc: &Html, selector: &str) -> Vec<SelectOption> {
    let sel = match Selector::parse(selector) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let select_el = match doc.select(&sel).next() {
        Some(e) => e,
        None => return Vec::new(),
    };
    let option_sel = &*SEL_OPTION;
    select_el
        .select(option_sel)
        .map(|opt| SelectOption {
            value: opt.value().attr("value").unwrap_or_default().to_string(),
            label: opt.text().collect::<String>().trim().to_string(),
            selected: opt.value().attr("selected").is_some(),
        })
        .collect()
}

pub(super) fn parse_japanese_number(s: &str) -> u32 {
    if s.contains('１') {
        return 1;
    }
    if s.contains('２') {
        return 2;
    }
    if s.contains('３') {
        return 3;
    }
    if s.contains('４') {
        return 4;
    }
    if s.contains('５') {
        return 5;
    }
    if s.contains('６') {
        return 6;
    }
    if s.contains('７') {
        return 7;
    }
    // Also try ASCII digits
    for c in s.chars() {
        if c.is_ascii_digit() {
            return c.to_digit(10).unwrap_or(0);
        }
    }
    0
}
