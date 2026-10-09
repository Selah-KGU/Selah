use super::*;

pub fn truncate_for_log(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

// ─────────────────────── Text Utilities ───────────────────────

pub fn normalize_planner_text(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !"[]()（）【】「」『』・,，.。:：!?！？_-".contains(*c))
        .collect()
}

pub fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

#[cfg(test)]
pub fn extract_kgc_code(text: &str) -> Option<String> {
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_alphanumeric() {
            start.get_or_insert(idx);
        } else if let Some(st) = start.take() {
            let token = &text[st..idx];
            if looks_like_kgc_code(token) {
                return Some(token.to_uppercase());
            }
        }
    }
    if let Some(st) = start {
        let token = &text[st..];
        if looks_like_kgc_code(token) {
            return Some(token.to_uppercase());
        }
    }
    None
}

/// Real KGC course codes start with a small set of faculty-letter prefixes.
/// Adding the whitelist here prevents tokens like `PDF12345` or `MAC10000` —
/// which fit the structural pattern of letters+digits — from being
/// dispatched as syllabus lookups.
#[cfg(test)]
const KGC_PREFIX_WHITELIST: &[&str] = &[
    "AB", "AE", "AL", "AS", "BL", "BU", "CO", "CS", "DC", "EC", "ED", "EN", "FD", "GE", "GS", "HS",
    "HU", "IB", "IC", "IS", "JP", "LA", "LB", "LE", "LI", "LR", "LS", "MA", "MD", "ME", "MM", "MS",
    "NS", "PA", "PE", "PH", "PL", "PO", "PS", "RC", "RE", "SC", "SD", "SO", "SP", "ST", "TA", "TC",
    "TH", "TM", "TS", "UC",
];

#[cfg(test)]
fn looks_like_kgc_code(token: &str) -> bool {
    let letters_n = token
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    let digits_n = token
        .chars()
        .skip(letters_n)
        .take_while(|c| c.is_ascii_digit())
        .count();
    if !(letters_n >= 2 && digits_n >= 3 && letters_n + digits_n == token.len()) {
        return false;
    }
    // Real KGC codes are typically 2-3 letter prefix + 4-5 digits.
    if letters_n > 4 || digits_n > 6 {
        return false;
    }
    let prefix: String = token
        .chars()
        .take(2)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    KGC_PREFIX_WHITELIST.contains(&prefix.as_str())
}

pub fn local_text(text: &str, tokens: usize) -> String {
    crate::local_ai::trim_apple_text(text, tokens)
}

pub fn render_local_json(value: &Value, tokens: usize) -> String {
    let compact = crate::local_ai::compact_json_value(value, tokens);
    serde_json::to_string(&compact).unwrap_or_else(|_| "{}".into())
}
pub fn trim_to(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}…", truncated)
}

pub fn preview_of(v: &Value) -> String {
    tool_result::preview(v, CFG.preview_bytes)
}

// ─────────────────────── History Helpers ───────────────────────

pub fn slice_history(
    rows: &[crate::db::AgentMessageRow],
    window: usize,
) -> &[crate::db::AgentMessageRow] {
    // Rows already exclude the committed input by ID at the SQL boundary.
    let end = rows.len();
    let start = end.saturating_sub(window);
    &rows[start..end]
}

pub fn maybe_autotitle(db: &Database, conv_id: &str, user_text: &str) {
    let title: String = user_text
        .chars()
        .filter(|c| !c.is_control())
        .take(24)
        .collect();
    let title = if title.trim().is_empty() {
        "新しい会話".to_string()
    } else {
        title
    };
    let _ = db.agent_autotitle_conversation(conv_id, &title);
}
