//! Page replies retain bounded line owners; complete records remain in storage.
use super::*;
use serde::{Deserialize, Serialize};

const VISIBLE_LINES: usize = 120;

mod compact;
pub(super) use compact::{CompactSurfaceSaveResult, CompactSurfaceSnapshot};

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct LiveSurfaceSnapshot {
    update_revision: u64,
    session_id: Option<String>,
    active: bool,
    course: Option<LiveCourseInfo>,
    started_at: Option<String>,
    transcript_line_count: usize,
    visible_lines: Vec<SharedTranscriptLine>,
    pending_from_line: usize,
    summaries: LiveSummaryChunks,
    next_summary_at_ms: Option<i64>,
    summarizing: bool,
    finish_phase: Option<LiveFinishPhase>,
    finish_revision: u64,
}

impl From<LiveSessionSnapshot> for LiveSurfaceSnapshot {
    fn from(snapshot: LiveSessionSnapshot) -> Self {
        let count = snapshot.transcript_lines.len();
        Self {
            update_revision: snapshot.update_revision,
            session_id: snapshot.session_id,
            active: snapshot.active,
            course: snapshot.course,
            started_at: snapshot.started_at,
            transcript_line_count: count,
            visible_lines: snapshot.transcript_lines[count.saturating_sub(VISIBLE_LINES)..]
                .to_vec(),
            pending_from_line: count.saturating_sub(snapshot.pending_lines.len()),
            summaries: snapshot.summaries,
            next_summary_at_ms: snapshot.next_summary_at_ms,
            summarizing: snapshot.summarizing,
            finish_phase: snapshot.finish_phase,
            finish_revision: snapshot.finish_revision,
        }
        // The full transcript/pending indexes are dropped before serialization.
    }
}

pub(super) fn current(state: &LiveState) -> LiveSurfaceSnapshot {
    state
        .session
        .lock()
        .ok()
        // Project while the capture lock is still held. A background encoder
        // never keeps the full index alive or forces append to copy that index.
        .map(|guard| LiveSurfaceSnapshot::from(state.capture_snapshot(guard.as_ref())))
        .unwrap_or_else(|| empty_snapshot().into())
}

/// Only the saved-note summary is rendered by the page. The complete Markdown
/// has already been persisted and remains available through the full API.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct LiveSurfaceSaveResult {
    saved: bool,
    path: String,
    summary_markdown: String,
    snapshot: LiveSurfaceSnapshot,
    suggested_todos: Vec<LiveTodoSuggestion>,
    todos_pending: bool,
}

impl From<LiveSaveResult> for LiveSurfaceSaveResult {
    fn from(result: LiveSaveResult) -> Self {
        Self {
            saved: result.saved,
            path: result.path,
            summary_markdown: summary_markdown(&result.markdown).to_string(),
            snapshot: result.snapshot.into(),
            suggested_todos: result.suggested_todos,
            todos_pending: result.todos_pending,
        }
    }
}

/// Preserve the page's existing extractOverallSummary section boundaries and
/// ECMAScript trim semantics, including FEFF and excluding the NEL character.
fn summary_markdown(markdown: &str) -> &str {
    let Some(start) = markdown.find("### 全体要約") else {
        return "";
    };
    let Some(newline) = markdown[start..].find('\n') else {
        return "";
    };
    let tail = &markdown[start + newline + 1..];
    let end = tail.find("\n###").or_else(|| tail.find("\n## "));
    tail[..end.unwrap_or(tail.len())].trim_matches(|ch| {
        matches!(ch, '\u{0009}'..='\u{000D}' | '\u{0020}' | '\u{00A0}' | '\u{1680}'
            | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}'
            | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
    })
}

#[cfg(test)]
#[path = "surface/tests.rs"]
mod tests;
