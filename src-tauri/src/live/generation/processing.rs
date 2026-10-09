//! CPU work owns immutable captured inputs; no LIVE/storage locks or model IO.
use super::*;

const FAILURE: &str = "Live生成結果の処理失敗";

pub(super) async fn summary(raw: String) -> Result<LiveChunkAiResult, String> {
    crate::background_ipc::run(FAILURE, move || {
        let parsed = parse_chunk_ai_result(&raw);
        // An empty reply leaves pending speech and its batch start untouched,
        // allowing the existing scheduled/final-flush retry paths to run.
        if parsed.body.trim().is_empty() {
            return Err("AI要約の本文が空でした（再試行します）".into());
        }
        Ok(parsed)
    })
    .await
}

pub(super) async fn whiteboard(
    parsed: LiveChunkAiResult,
    raw: Option<String>,
    lines: LiveTranscriptLines,
    previous: LiveSummaryChunks,
) -> Result<SharedLiveChunkAiResult, String> {
    crate::background_ipc::run(FAILURE, move || {
        let board = raw.and_then(|raw| parse_chunk_ai_result(&raw).whiteboard);
        let previous = latest_shared_whiteboard(&previous);
        let board = enrich_whiteboard_source_excerpts(
            board,
            previous.map(|board| board.as_ref()),
            &parsed.terms,
            &lines,
        );
        Ok(LiveChunkAiResult {
            body: parsed.body,
            terms: parsed.terms,
            whiteboard: reconcile_whiteboard(previous, board),
        })
    })
    .await
}

pub(super) async fn overall(raw: String) -> Result<String, String> {
    crate::background_ipc::run(FAILURE, move || Ok(sanitize_model_output(&raw))).await
}
pub(super) async fn todos(
    course: LiveCourseInfo,
    raw: String,
) -> Result<Vec<LiveTodoSuggestion>, String> {
    crate::background_ipc::run(FAILURE, move || Ok(parse_todos(&course, &raw))).await
}
fn parse_todos(course: &LiveCourseInfo, raw: &str) -> Vec<LiveTodoSuggestion> {
    let Some(json_text) = extract_json_object(raw) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return Vec::new();
    };
    let Some(items) = value.get("todos").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items.iter().take(6) {
        let title = value_to_trimmed_string(item.get("title"));
        if title.is_empty() {
            continue;
        }
        let content_type = value_to_trimmed_string(item.get("content_type"));
        out.push(LiveTodoSuggestion {
            title,
            course_name: course.course_name.clone(),
            content_type: if content_type.is_empty() {
                "課題".to_string()
            } else {
                content_type
            },
            deadline: value_to_trimmed_string(item.get("deadline")),
            note: value_to_trimmed_string(item.get("note")),
            source_excerpt: value_to_trimmed_string(item.get("source_excerpt")),
            day: course.day,
            period: course.period,
        });
    }
    out
}

#[cfg(test)]
mod tests;
