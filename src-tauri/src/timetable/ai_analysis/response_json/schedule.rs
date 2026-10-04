use super::values::{value_to_bool, value_to_i32, value_to_string, value_to_string_vec};

pub(in crate::timetable::ai_analysis) fn normalize_ai_schedule_json(
    mut root: serde_json::Value,
) -> serde_json::Value {
    if !root.is_object() {
        return serde_json::json!({
            "current_week": [],
            "next_week": [],
            "weekly_summary": "",
            "cross_week_insights": "",
        });
    }

    if let Some(obj) = root.as_object_mut() {
        let current_week = obj
            .remove("current_week")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let next_week = obj
            .remove("next_week")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let weekly_summary = obj
            .remove("weekly_summary")
            .unwrap_or(serde_json::Value::Null);
        let cross_week_insights = obj
            .remove("cross_week_insights")
            .unwrap_or(serde_json::Value::Null);

        obj.insert(
            "current_week".to_string(),
            normalize_schedule_items(current_week),
        );
        obj.insert("next_week".to_string(), normalize_schedule_items(next_week));
        obj.insert(
            "weekly_summary".to_string(),
            serde_json::Value::String(value_to_string(weekly_summary)),
        );
        obj.insert(
            "cross_week_insights".to_string(),
            serde_json::Value::String(value_to_string(cross_week_insights)),
        );
    }

    root
}

fn normalize_schedule_items(value: serde_json::Value) -> serde_json::Value {
    let arr = match value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    let out: Vec<serde_json::Value> = arr
        .into_iter()
        .filter_map(normalize_schedule_item)
        .collect();
    serde_json::Value::Array(out)
}

fn normalize_schedule_item(value: serde_json::Value) -> Option<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };

    let item = serde_json::json!({
        "day": value_to_i32(obj.remove("day").unwrap_or(serde_json::Value::Null)),
        "period": value_to_i32(obj.remove("period").unwrap_or(serde_json::Value::Null)),
        "course_name": value_to_string(obj.remove("course_name").unwrap_or(serde_json::Value::Null)),
        "delivery_mode": value_to_string(obj.remove("delivery_mode").unwrap_or(serde_json::Value::Null)),
        "room": value_to_string(obj.remove("room").unwrap_or(serde_json::Value::Null)),
        "teacher": value_to_string(obj.remove("teacher").unwrap_or(serde_json::Value::Null)),
        "session_topic": value_to_string(obj.remove("session_topic").unwrap_or(serde_json::Value::Null)),
        "is_cancelled": value_to_bool(obj.remove("is_cancelled").unwrap_or(serde_json::Value::Null)),
        "notifications": value_to_string_vec(obj.remove("notifications").unwrap_or(serde_json::Value::Null)),
        "assignments": value_to_string_vec(obj.remove("assignments").unwrap_or(serde_json::Value::Null)),
        "exams": value_to_string_vec(obj.remove("exams").unwrap_or(serde_json::Value::Null)),
    });

    Some(item)
}
