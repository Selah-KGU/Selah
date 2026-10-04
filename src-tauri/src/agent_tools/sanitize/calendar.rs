use serde_json::Value;

use super::text::sanitize_text_arg;

pub(in crate::agent_tools) fn sanitize_calendar_event_args(args: &Value) -> Option<Value> {
    // title, date, start_time, end_time are required.
    let title = sanitize_text_arg(args, "title", 200)?;
    let date = sanitize_text_arg(args, "date", 10)?;
    let start_time = sanitize_text_arg(args, "start_time", 5)?;
    let end_time = sanitize_text_arg(args, "end_time", 5)?;
    // Basic structural validation — real format validation happens inside the tool.
    if date.len() != 10 || !date.chars().nth(4).map(|c| c == '-').unwrap_or(false) {
        return None;
    }
    if start_time.len() != 5 || end_time.len() != 5 {
        return None;
    }
    let location = sanitize_text_arg(args, "location", 200);
    let description = sanitize_text_arg(args, "description", 500);
    let mut out = serde_json::Map::new();
    out.insert("title".into(), Value::String(title));
    out.insert("date".into(), Value::String(date));
    out.insert("start_time".into(), Value::String(start_time));
    out.insert("end_time".into(), Value::String(end_time));
    if let Some(loc) = location {
        out.insert("location".into(), Value::String(loc));
    }
    if let Some(desc) = description {
        out.insert("description".into(), Value::String(desc));
    }
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_calendar_update_args(args: &Value) -> Option<Value> {
    // event_id is required; all other fields are optional.
    let event_id = sanitize_text_arg(args, "event_id", 200)?;
    let title = sanitize_text_arg(args, "title", 200);
    let date = sanitize_text_arg(args, "date", 10)
        .filter(|d| d.len() == 10 && d.chars().nth(4).map(|c| c == '-').unwrap_or(false));
    let start_time = sanitize_text_arg(args, "start_time", 5).filter(|t| t.len() == 5);
    let end_time = sanitize_text_arg(args, "end_time", 5).filter(|t| t.len() == 5);
    let location = sanitize_text_arg(args, "location", 200);
    let description = sanitize_text_arg(args, "description", 500);
    let mut out = serde_json::Map::new();
    out.insert("event_id".into(), Value::String(event_id));
    if let Some(v) = title {
        out.insert("title".into(), Value::String(v));
    }
    if let Some(v) = date {
        out.insert("date".into(), Value::String(v));
    }
    if let Some(v) = start_time {
        out.insert("start_time".into(), Value::String(v));
    }
    if let Some(v) = end_time {
        out.insert("end_time".into(), Value::String(v));
    }
    // Pass through location/description even if empty so tool knows to clear them.
    if args.get("location").is_some() {
        out.insert(
            "location".into(),
            location.map(Value::String).unwrap_or(Value::Null),
        );
    }
    if args.get("description").is_some() {
        out.insert(
            "description".into(),
            description.map(Value::String).unwrap_or(Value::Null),
        );
    }
    Some(Value::Object(out))
}
