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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTermExplanation {
    pub term: String,
    pub explanation: String,
    #[serde(default)]
    pub source_excerpt: String,
    #[serde(default)]
    pub external_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveWhiteboardEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSummaryChunk {
    pub title: String,
    pub range_label: String,
    pub body: String,
    pub line_count: usize,
    #[serde(default)]
    pub terms: Vec<LiveTermExplanation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whiteboard: Option<LiveWhiteboard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionSnapshot {
    pub active: bool,
    pub course: Option<LiveCourseInfo>,
    pub started_at: Option<String>,
    // Wrapped in Arc so building a snapshot is a refcount bump rather than
    // a deep clone of three potentially large Vec<...>. The wire format is
    // unchanged because serde serializes Arc<T> transparently as T.
    pub transcript_lines: Arc<Vec<LiveTranscriptLine>>,
    pub pending_lines: Arc<Vec<LiveTranscriptLine>>,
    pub summaries: Arc<Vec<LiveSummaryChunk>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSaveResult {
    pub saved: bool,
    pub path: String,
    pub markdown: String,
    pub snapshot: LiveSessionSnapshot,
    #[serde(default)]
    pub suggested_todos: Vec<LiveTodoSuggestion>,
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

pub(super) struct LiveChunkAiResult {
    pub(super) body: String,
    pub(super) terms: Vec<LiveTermExplanation>,
    pub(super) whiteboard: Option<LiveWhiteboard>,
}
