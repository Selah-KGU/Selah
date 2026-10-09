//! Page-only transport shares board versions within one reply. Archives and
//! legacy RPCs still serialize every complete board object.
use super::*;
use serde::ser::{SerializeSeq, Serializer};
use std::collections::HashMap;

pub(in crate::live) struct CompactSurfaceSnapshot {
    page: LiveSurfaceSnapshot,
    boards: Vec<SharedWhiteboard>,
    references: Vec<Option<usize>>,
}

impl From<LiveSurfaceSnapshot> for CompactSurfaceSnapshot {
    fn from(page: LiveSurfaceSnapshot) -> Self {
        let mut indexes = HashMap::new();
        let mut contents = HashMap::new();
        let mut boards = Vec::new();
        let references = page
            .summaries
            .iter()
            .map(|chunk| {
                chunk.whiteboard.as_ref().map(|board| {
                    // Summary owners keep every board alive during this walk.
                    // Pointer lookup skips rehashing shared versions. Full
                    // equality also merges independently deserialized cache
                    // boards; hash collisions never merge unequal content.
                    *indexes.entry(Arc::as_ptr(board)).or_insert_with(|| {
                        *contents.entry(board.as_ref()).or_insert_with(|| {
                            let index = boards.len();
                            boards.push(board.clone());
                            index
                        })
                    })
                })
            })
            .collect();
        drop(contents);
        Self {
            page,
            boards,
            references,
        }
    }
}

impl From<LiveSessionSnapshot> for CompactSurfaceSnapshot {
    fn from(snapshot: LiveSessionSnapshot) -> Self {
        LiveSurfaceSnapshot::from(snapshot).into()
    }
}

struct SummaryReferences<'a> {
    chunks: &'a [SharedSummaryChunk],
    references: &'a [Option<usize>],
}
impl Serialize for SummaryReferences<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Chunk<'a> {
            title: &'a str,
            range_label: &'a str,
            body: &'a str,
            line_count: usize,
            terms: &'a [LiveTermExplanation],
            #[serde(skip_serializing_if = "Option::is_none")]
            whiteboard_ref: Option<usize>,
        }
        let mut sequence = serializer.serialize_seq(Some(self.chunks.len()))?;
        for (chunk, reference) in self.chunks.iter().zip(self.references) {
            sequence.serialize_element(&Chunk {
                title: &chunk.title,
                range_label: &chunk.range_label,
                body: &chunk.body,
                line_count: chunk.line_count,
                terms: &chunk.terms,
                whiteboard_ref: *reference,
            })?;
        }
        sequence.end()
    }
}

impl Serialize for CompactSurfaceSnapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            whiteboard_table_version: u8,
            update_revision: u64,
            session_id: &'a Option<String>,
            active: bool,
            course: &'a Option<LiveCourseInfo>,
            started_at: &'a Option<String>,
            transcript_line_count: usize,
            visible_lines: &'a [SharedTranscriptLine],
            pending_from_line: usize,
            summaries: SummaryReferences<'a>,
            whiteboards: &'a [SharedWhiteboard],
            next_summary_at_ms: Option<i64>,
            summarizing: bool,
            finish_phase: Option<LiveFinishPhase>,
            finish_revision: u64,
        }
        let page = &self.page;
        Wire {
            whiteboard_table_version: 1,
            update_revision: page.update_revision,
            session_id: &page.session_id,
            active: page.active,
            course: &page.course,
            started_at: &page.started_at,
            transcript_line_count: page.transcript_line_count,
            visible_lines: &page.visible_lines,
            pending_from_line: page.pending_from_line,
            summaries: SummaryReferences {
                chunks: &page.summaries,
                references: &self.references,
            },
            whiteboards: &self.boards,
            next_summary_at_ms: page.next_summary_at_ms,
            summarizing: page.summarizing,
            finish_phase: page.finish_phase,
            finish_revision: page.finish_revision,
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
pub(in crate::live) struct CompactSurfaceSaveResult {
    saved: bool,
    path: String,
    summary_markdown: String,
    snapshot: CompactSurfaceSnapshot,
    suggested_todos: Vec<LiveTodoSuggestion>,
    todos_pending: bool,
}
impl From<LiveSaveResult> for CompactSurfaceSaveResult {
    fn from(result: LiveSaveResult) -> Self {
        let page = LiveSurfaceSaveResult::from(result);
        Self {
            saved: page.saved,
            path: page.path,
            summary_markdown: page.summary_markdown,
            snapshot: page.snapshot.into(),
            suggested_todos: page.suggested_todos,
            todos_pending: page.todos_pending,
        }
    }
}

#[cfg(test)]
mod tests;
