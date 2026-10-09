//! LIVE model calls for chunk notes, whiteboards, overall summaries, and TODOs.
//!
//! Coordinates async model calls. Request/configuration/course-plan work and
//! result parsing run in their worker modules; prompt policy stays in prompts.

use super::*;

mod processing;
mod request_text;
mod requests;

/// Two-pass chunk pipeline:
///   Call 1 → summary_markdown + terms (sees raw transcript only)
///   Call 2 → whiteboard JSON only (sees all prior summaries+terms, the
///            current cumulative board, the just-produced summary+terms, and
///            the raw transcript for completeness)
///
/// Splitting the calls lets each output budget breathe (the whiteboard JSON no
/// longer competes with summary tokens) and lets the whiteboard call work from
/// already-distilled prior material rather than re-parsing every transcript.
/// If Call 2 fails, the result worker uses `reconcile_whiteboard` to carry the
/// previous board forward before returning the completed chunk.
pub(super) async fn summarize_chunk(
    course: &LiveCourseInfo,
    lines: &LiveTranscriptLines,
    recent_summaries: &LiveSummaryChunks,
    range_label: &str,
) -> Result<SharedLiveChunkAiResult, String> {
    let prepared =
        requests::chunk(requests::Captured::new(course, lines, recent_summaries)).await?;
    let raw = crate::ai::chat_completion_public(&prepared.cfg, prepared.messages).await?;
    let parsed = processing::summary(raw).await?;
    // Prepare the second prompt only after a usable first summary. No duplicate
    // whiteboard transcript string is retained during the first model call.
    let (parsed, messages) = requests::whiteboard(
        requests::Captured::new(course, lines, recent_summaries),
        prepared.cfg.reply_language.clone(),
        parsed,
        range_label.to_owned(),
    )
    .await?;
    let raw_board = match crate::ai::chat_completion_public(&prepared.cfg, messages).await {
        Ok(raw) => Some(raw),
        Err(err) => {
            eprintln!(
                "[Live whiteboard] secondary call failed: {err}; carrying previous board forward"
            );
            None
        }
    };
    processing::whiteboard(parsed, raw_board, lines.clone(), recent_summaries.clone()).await
}

pub(super) async fn generate_overall_summary(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    summaries: &LiveSummaryChunks,
    transcript_lines: &LiveTranscriptLines,
) -> String {
    match requests::overall(
        requests::Captured::new(course, transcript_lines, summaries),
        started_at,
        ended_at,
    )
    .await
    {
        Ok(requests::Overall::Ready(text)) => text,
        Ok(requests::Overall::Model { request, fallback }) => {
            match crate::ai::chat_completion_public(&request.cfg, request.messages).await {
                Ok(raw) => processing::overall(raw).await.unwrap_or(fallback),
                Err(_) => fallback,
            }
        }
        Err(error) => {
            log::warn!("[Live] overall preparation failed: {error}");
            if should_skip_ai_summarization(started_at, ended_at) {
                short_session_overall_summary(course, transcript_lines.len(), "ja")
            } else {
                fallback_overall_summary(course, transcript_lines.len(), summaries.len(), "ja")
            }
        }
    }
}

pub(super) async fn extract_todo_suggestions(
    app: &tauri::AppHandle,
    course: &LiveCourseInfo,
    summaries: &LiveSummaryChunks,
    transcript_lines: &LiveTranscriptLines,
    ended_at: DateTime<Local>,
) -> Vec<LiveTodoSuggestion> {
    if course.is_free_note || transcript_lines.is_empty() {
        return Vec::new();
    }
    let Ok(request) = requests::todo(
        requests::Captured::new(course, transcript_lines, summaries),
        app.clone(),
        ended_at,
    )
    .await
    else {
        return Vec::new();
    };
    let Ok(raw) = crate::ai::chat_completion_public(&request.cfg, request.messages).await else {
        return Vec::new();
    };
    processing::todos(course.clone(), raw)
        .await
        .unwrap_or_default()
}

pub(super) fn build_chunk_title(
    index: usize,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> String {
    format!(
        "Chunk {:02} | {}-{}",
        index,
        format_time(start),
        format_time(end)
    )
}
