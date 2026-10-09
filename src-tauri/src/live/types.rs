use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveCourseInfo {
    pub course_name: String,
    #[serde(default)]
    pub course_code: String,
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub teacher: String,
    pub day: i32,
    pub period: i32,
    #[serde(default)]
    pub time_label: String,
    #[serde(default)]
    pub is_free_note: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTranscriptLine {
    pub text: String,
    pub at: String,
}

/// One immutable accepted line shared by history, pending summary and events.
/// Serde keeps the existing {text, at} object representation.
pub type SharedTranscriptLine = Arc<LiveTranscriptLine>;
/// Snapshots retain an immutable index; copy-on-write clones only line owners.
pub type LiveTranscriptLines = Arc<Vec<SharedTranscriptLine>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTermExplanation {
    pub term: String,
    pub explanation: String,
    #[serde(default)]
    pub source_excerpt: String,
    #[serde(default)]
    pub external_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LiveWhiteboardNode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub node_type: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub parent_id: String,
    #[serde(default)]
    pub source_type: String,
    #[serde(default)]
    pub source_excerpt: String,
    #[serde(default)]
    pub external_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LiveWhiteboardEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LiveWhiteboard {
    pub title: String,
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub nodes: Vec<LiveWhiteboardNode>,
    #[serde(default)]
    pub edges: Vec<LiveWhiteboardEdge>,
    /// Protocol version. 0 = legacy/unset, 1 = node_type + normalized_by supported.
    #[serde(default)]
    pub schema_version: u8,
    /// Which layer last performed structural normalization.
    /// "backend"  = parse_live_whiteboard ran (canonical source).
    /// ""         = unknown / legacy / demo board.
    #[serde(default)]
    pub normalized_by: String,
}

/// Completed board versions are immutable. Reused versions share all node and
/// edge buffers; Serde retains the existing complete whiteboard object shape.
pub type SharedWhiteboard = Arc<LiveWhiteboard>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSummaryChunk {
    pub title: String,
    pub range_label: String,
    pub body: String,
    pub line_count: usize,
    #[serde(default)]
    pub terms: Vec<LiveTermExplanation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whiteboard: Option<SharedWhiteboard>,
}

/// One immutable completed chunk shared by snapshots, storage and updates.
/// Serde preserves the existing summary/terms/whiteboard object.
pub type SharedSummaryChunk = Arc<LiveSummaryChunk>;
/// Retained history copies only chunk owners when a new summary is appended.
pub type LiveSummaryChunks = Arc<Vec<SharedSummaryChunk>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveFinishPhase {
    Stopping,
    SavingRecord,
    Summarizing,
    SavingFinal,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveFinishProgress {
    pub session_id: String,
    pub finish_phase: LiveFinishPhase,
    pub finish_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionSnapshot {
    /// Capture order across recordings in this running backend. Cached records
    /// predate this protocol and deserialize with revision zero.
    #[serde(default)]
    pub update_revision: u64,
    #[serde(default)]
    pub session_id: Option<String>,
    pub active: bool,
    pub course: Option<LiveCourseInfo>,
    pub started_at: Option<String>,
    // Wrapped in Arc so building a snapshot is a refcount bump rather than
    // a deep clone of three potentially large Vec<...>. The wire format is
    // unchanged because serde serializes Arc<T> transparently as T.
    pub transcript_lines: LiveTranscriptLines,
    pub pending_lines: LiveTranscriptLines,
    pub summaries: LiveSummaryChunks,
    /// Epoch millis when the next scheduled periodic summary is due
    /// (effective batch start + interval). `None` when no session is active.
    #[serde(default)]
    pub next_summary_at_ms: Option<i64>,
    /// True while a periodic summary is being generated (flush in flight).
    #[serde(default)]
    pub summarizing: bool,
    /// The backend owns finishing even when the WebView has reloaded.
    #[serde(default)]
    pub finish_phase: Option<LiveFinishPhase>,
    /// Orders phase changes and failed-finish retries within this recording.
    #[serde(default)]
    pub finish_revision: u64,
}

/// Low-frequency lifecycle/summary notification. History recovery remains a
/// separate full/surface snapshot RPC; updates never serialize the whole log.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct LiveSessionUpdate<Chunk = SharedSummaryChunk> {
    pub update_revision: u64,
    pub session_id: Option<String>,
    pub active: bool,
    pub course: Option<LiveCourseInfo>,
    pub started_at: Option<String>,
    pub next_summary_at_ms: Option<i64>,
    pub summarizing: bool,
    pub finish_phase: Option<LiveFinishPhase>,
    pub finish_revision: u64,
    pub transcript_line_count: usize,
    pub pending_line_count: usize,
    pub summary_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_summary: Option<Chunk>,
}

/// Native overlays skip all course/summary/history fields during JSON parsing.
#[derive(Debug, Deserialize)]
pub(crate) struct LiveSessionStatus {
    pub update_revision: u64,
    pub active: bool,
    pub session_id: Option<String>,
}

/// Ordered delta for the UI. STT commits the line before emitting this event,
/// so a suspended or crashed WebView cannot interrupt transcript storage.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct LiveTranscriptUpdate {
    pub session_id: String,
    pub line_count: usize,
    pub line: SharedTranscriptLine,
    /// STT capture order, absent for a manually appended line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSaveResult {
    pub saved: bool,
    pub path: String,
    pub markdown: String,
    pub snapshot: LiveSessionSnapshot,
    #[serde(default)]
    pub suggested_todos: Vec<LiveTodoSuggestion>,
    /// True when TODO/DDL extraction was kicked off in the background. The save
    /// returns immediately; the suggestions arrive later via the
    /// `live-todo-suggestions` event so the UI can move to the TODO page now.
    #[serde(default)]
    pub todos_pending: bool,
}

/// Payload of the `live-todo-suggestions` event emitted once the background
/// TODO/DDL judgment finishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTodoSuggestionsEvent {
    pub suggestions: Vec<LiveTodoSuggestion>,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTodoSuggestion {
    pub title: String,
    pub course_name: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub deadline: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub source_excerpt: String,
    pub day: i32,
    pub period: i32,
}

pub(super) struct LiveChunkAiResult<Board = LiveWhiteboard> {
    pub(super) body: String,
    pub(super) terms: Vec<LiveTermExplanation>,
    pub(super) whiteboard: Option<Board>,
}

/// Parsing/enrichment still own mutable model output. Only completed results
/// contain shared immutable board versions, ready to move into summary history.
pub(super) type SharedLiveChunkAiResult = LiveChunkAiResult<SharedWhiteboard>;
