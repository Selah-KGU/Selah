//! Parse a chunk model reply into summary, terms, and whiteboard.

use super::*;

fn is_low_value_live_term(term: &str) -> bool {
    let normalized = term
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '（'
                    | '）'
                    | '('
                    | ')'
                    | '【'
                    | '】'
                    | '['
                    | ']'
            )
        })
        .to_lowercase();
    if normalized.chars().count() <= 1 {
        return true;
    }
    const LOW_VALUE_TERMS: &[&str] = &[
        "授業",
        "講義",
        "先生",
        "教員",
        "教授",
        "学生",
        "大学",
        "教室",
        "出席",
        "欠席",
        "課題",
        "宿題",
        "レポート",
        "資料",
        "教科書",
        "スライド",
        "今日",
        "次回",
        "明日",
        "来週",
        "学校",
        "勉強",
        "学習",
        "考试",
        "作业",
        "报告",
        "老师",
        "学生",
        "大学",
        "教室",
        "今天",
        "下次",
        "tomorrow",
        "today",
        "class",
        "lecture",
        "teacher",
        "student",
        "assignment",
        "report",
        "homework",
        "textbook",
        "slides",
    ];
    LOW_VALUE_TERMS
        .iter()
        .any(|candidate| normalized == *candidate)
}

pub fn parse_chunk_ai_result(raw: &str) -> LiveChunkAiResult {
    let sanitized = sanitize_model_output(raw);
    // Parse the first JSON object — strictly first, then with a repair pass that
    // fixes the two things large replies break on: raw control chars inside
    // strings and truncation. Repair leaves already-valid JSON untouched.
    let value = extract_json_object(&sanitized)
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .or_else(|| {
            repair_json_object(&sanitized)
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        });
    let Some(value) = value else {
        // Unparseable even after repair. If the reply was a JSON object we just
        // couldn't fix, try to lift the summary text out by hand rather than
        // dumping raw braces into the note; only genuine plain prose (a reply
        // that does not start with `{`) is kept verbatim. Otherwise leave the
        // body empty so the caller's retry layers re-attempt the chunk.
        let body = salvage_json_string_field(&sanitized, "summary_markdown")
            .or_else(|| salvage_json_string_field(&sanitized, "summary"))
            .or_else(|| salvage_json_string_field(&sanitized, "body"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if sanitized.trim_start().starts_with('{') {
                    String::new()
                } else {
                    sanitized.clone()
                }
            });
        return LiveChunkAiResult {
            body,
            terms: Vec::new(),
            whiteboard: None,
        };
    };

    let body = value_to_trimmed_string(
        value
            .get("summary_markdown")
            .or_else(|| value.get("summary"))
            .or_else(|| value.get("body")),
    );
    let mut terms = Vec::new();
    if let Some(items) = value.get("terms").and_then(|v| v.as_array()) {
        for item in items.iter().take(5) {
            let term = clamp_chars(&value_to_trimmed_string(item.get("term")), 40);
            let explanation = clamp_chars(
                &value_to_trimmed_string(item.get("explanation")),
                MAX_LIVE_TERM_EXPLANATION_CHARS,
            );
            if term.is_empty() || explanation.is_empty() || is_low_value_live_term(&term) {
                continue;
            }
            terms.push(LiveTermExplanation {
                term,
                explanation,
                source_excerpt: clamp_chars(
                    &value_to_trimmed_string(item.get("source_excerpt")),
                    80,
                ),
                external_source: clamp_chars(
                    &value_to_trimmed_string(item.get("external_source")),
                    180,
                ),
            });
        }
    }
    let whiteboard = parse_live_whiteboard(value.get("whiteboard"));

    // JSON parsed cleanly but the summary field was empty/missing. Do NOT fall
    // back to the raw JSON text (that surfaced a blob of JSON as the "summary").
    // Leave body empty so the caller treats it as a retryable empty result.
    // The sanitized-text fallback only applies above, when no JSON was found at
    // all — there the model returned plain prose that is reasonable to keep.
    LiveChunkAiResult {
        body,
        terms,
        whiteboard,
    }
}
