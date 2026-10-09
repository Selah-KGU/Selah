use super::{
    sanitize_model_output, LiveChunkAiResult, LiveTermExplanation, LiveWhiteboard,
    LiveWhiteboardEdge, LiveWhiteboardNode, SharedWhiteboard, MAX_LIVE_TERM_EXPLANATION_CHARS,
};

mod board;
mod context;
mod json;
mod parse;
mod reconcile;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) use context::latest_whiteboard;
pub(super) use context::{format_latest_whiteboard_context, latest_shared_whiteboard};
pub(super) use json::{
    clamp_chars, extract_json_object, repair_json_object, salvage_json_string_field,
    value_to_trimmed_string,
};
pub(super) use parse::parse_chunk_ai_result;
pub(super) use reconcile::reconcile_whiteboard;

pub(in crate::live::ai_output) use board::parse_live_whiteboard;
