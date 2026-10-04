//! Clean, normalize, and repair JSON returned by the analysis models.

#[path = "response_json/extract.rs"]
mod extract;
#[path = "response_json/sanitize.rs"]
mod sanitize;
#[path = "response_json/schedule.rs"]
mod schedule;
#[path = "response_json/todo.rs"]
mod todo;
#[path = "response_json/values.rs"]
mod values;

pub(super) use extract::{
    extract_json_from_local_response, extract_json_from_response, repair_truncated_json,
};
pub(super) use sanitize::sanitize_ai_response_text;
pub(super) use schedule::normalize_ai_schedule_json;
pub(super) use todo::normalize_ai_todo_json;
