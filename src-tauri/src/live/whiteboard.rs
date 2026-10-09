use super::{LiveTermExplanation, LiveWhiteboard, LiveWhiteboardNode, SharedTranscriptLine};

#[path = "whiteboard/history.rs"]
mod history;
pub(in crate::live) use history::WhiteboardContext;
#[cfg(test)]
pub(in crate::live) use history::{
    format_current_chunk_for_whiteboard, format_full_history_for_whiteboard,
    format_recent_summary_context,
};

#[path = "whiteboard/excerpts.rs"]
mod excerpts;
pub(in crate::live) use excerpts::enrich_whiteboard_source_excerpts;
