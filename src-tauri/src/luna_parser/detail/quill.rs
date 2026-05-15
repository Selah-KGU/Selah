use super::super::*;

/// Extract Quill text from a specific named variable (e.g. "themeContents", "threadContents0")
pub(in crate::luna_parser) fn extract_named_quill_text(
    html: &str,
    var_name: &str,
) -> Option<String> {
    // Pattern: _QuillUtil.varName.setJsonData("...", ...)
    let pattern = format!("{}.setJsonData(\"", var_name);
    let pos = html.find(&pattern)?;
    let after = &html[pos + pattern.len()..];
    // Byte-level scan for closing unescaped ": safe because ASCII 0x22/0x5C
    // cannot appear as UTF-8 continuation bytes (continuation bytes are 0x80–0xBF).
    let bytes = after.as_bytes();
    let mut end: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2; // skip escaped char
            continue;
        }
        if bytes[i] == b'"' {
            end = Some(i);
            break;
        }
        i += 1;
    }
    let json_str = &after[..end?];
    extract_quill_rich_html(json_str)
}

fn escape_html_fragment(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn wrap_quill_inline_attrs(
    text: &str,
    attrs: Option<&serde_json::Map<String, serde_json::Value>>,
) -> String {
    let mut out = escape_html_fragment(text);
    let Some(attrs) = attrs else {
        return out;
    };

    if attrs.get("bold").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<strong>{}</strong>", out);
    }
    if attrs.get("italic").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<em>{}</em>", out);
    }
    if attrs.get("underline").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<u>{}</u>", out);
    }
    if attrs.get("code").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<code>{}</code>", out);
    }

    let mut style_parts: Vec<String> = Vec::new();
    if let Some(color) = attrs.get("color").and_then(|v| v.as_str()) {
        style_parts.push(format!("color:{}", escape_html_fragment(color)));
    }
    if let Some(bg) = attrs.get("background").and_then(|v| v.as_str()) {
        style_parts.push(format!("background-color:{}", escape_html_fragment(bg)));
    }
    if !style_parts.is_empty() {
        out = format!("<span style=\"{}\">{}</span>", style_parts.join(";"), out);
    }

    if let Some(link) = attrs.get("link").and_then(|v| v.as_str()) {
        let lower = link.to_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:")
            || lower.starts_with("tel:")
        {
            out = format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener\">{}</a>",
                escape_html_fragment(link),
                out
            );
        }
    }

    out
}

pub(in crate::luna_parser) fn extract_quill_rich_html(json_str: &str) -> Option<String> {
    let unescaped = unescape_js_string(json_str);
    let val: serde_json::Value = serde_json::from_str(&unescaped).ok()?;
    let ops = val.get("ops")?.as_array()?;

    let mut html = String::new();
    // Track whether any non-newline content was emitted to avoid allocating
    // temporary strings just for the emptiness check below.
    let mut has_content = false;
    for op in ops {
        let Some(insert) = op.get("insert").and_then(|v| v.as_str()) else {
            continue;
        };
        let attrs = op.get("attributes").and_then(|a| a.as_object());

        let mut segment = String::new();
        for ch in insert.chars() {
            if ch == '\n' {
                if !segment.is_empty() {
                    has_content = true;
                    html.push_str(&wrap_quill_inline_attrs(&segment, attrs));
                    segment.clear();
                }
                html.push_str("<br>");
            } else {
                segment.push(ch);
            }
        }
        if !segment.is_empty() {
            has_content = true;
            html.push_str(&wrap_quill_inline_attrs(&segment, attrs));
        }
    }

    if has_content {
        Some(html)
    } else {
        None
    }
}
