use super::values::{value_to_f64, value_to_i32, value_to_string, value_to_string_vec};

pub(in crate::timetable::ai_analysis) fn normalize_ai_todo_json(
    mut root: serde_json::Value,
) -> serde_json::Value {
    if !root.is_object() {
        return serde_json::json!({
            "task_guides": [],
            "daily_plan": [],
            "advice": "",
        });
    }

    if let Some(obj) = root.as_object_mut() {
        let task_guides = obj
            .remove("task_guides")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let daily_plan = obj
            .remove("daily_plan")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let advice = remove_first_value(obj, &["advice", "summary"]);

        obj.insert(
            "task_guides".to_string(),
            normalize_todo_task_guides(task_guides),
        );
        obj.insert(
            "daily_plan".to_string(),
            normalize_todo_daily_plan(daily_plan),
        );
        obj.insert(
            "advice".to_string(),
            serde_json::Value::String(value_to_string(advice)),
        );
        obj.remove("_check");
    }

    root
}

fn normalize_todo_task_guides(value: serde_json::Value) -> serde_json::Value {
    let arr = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    let guides: Vec<serde_json::Value> = arr
        .into_iter()
        .filter_map(normalize_todo_task_guide)
        .collect();
    serde_json::Value::Array(guides)
}

fn normalize_todo_task_guide(value: serde_json::Value) -> Option<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(obj) => obj,
        _ => return None,
    };

    let task_name = value_to_string(remove_first_value(
        &mut obj,
        &["task_name", "title", "task"],
    ));
    let course_name = value_to_string(remove_first_value(
        &mut obj,
        &["course_name", "course", "subject"],
    ));
    if task_name.trim().is_empty() && course_name.trim().is_empty() {
        return None;
    }

    let deadline = value_to_string(remove_first_value(
        &mut obj,
        &["deadline", "due", "due_at", "period"],
    ));
    let urgency = normalize_todo_urgency(value_to_string(remove_first_value(
        &mut obj,
        &["urgency", "priority"],
    )));
    let background = value_to_string(remove_first_value(
        &mut obj,
        &["background", "context", "summary"],
    ));
    let live_note_summary = value_to_string(remove_first_value(
        &mut obj,
        &[
            "live_note_summary",
            "note_summary",
            "note_context",
            "class_note_summary",
        ],
    ));
    let study_hints = value_to_string_vec(remove_first_value(
        &mut obj,
        &["study_hints", "steps", "hints", "actions"],
    ));
    let ready_to_use_label = value_to_string(remove_first_value(
        &mut obj,
        &[
            "ready_to_use_label",
            "starter_output_label",
            "draft_label",
            "output_label",
        ],
    ));
    let ready_to_use = value_to_string(remove_first_value(
        &mut obj,
        &[
            "ready_to_use",
            "starter_output",
            "draft",
            "output",
            "template",
        ],
    ));
    let estimated_minutes = value_to_i32(remove_first_value(
        &mut obj,
        &["estimated_minutes", "minutes", "eta_minutes"],
    ))
    .max(0);

    Some(serde_json::json!({
        "task_name": task_name,
        "course_name": course_name,
        "deadline": deadline,
        "urgency": urgency,
        "background": background,
        "live_note_summary": live_note_summary,
        "study_hints": study_hints,
        "ready_to_use_label": ready_to_use_label,
        "ready_to_use": ready_to_use,
        "estimated_minutes": estimated_minutes,
    }))
}

fn normalize_todo_daily_plan(value: serde_json::Value) -> serde_json::Value {
    let arr = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    let plans: Vec<serde_json::Value> = arr
        .into_iter()
        .filter_map(normalize_todo_daily_plan_item)
        .collect();
    serde_json::Value::Array(plans)
}

fn normalize_todo_daily_plan_item(value: serde_json::Value) -> Option<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(obj) => obj,
        _ => return None,
    };

    let label = value_to_string(remove_first_value(&mut obj, &["label", "day"]));
    let tasks = value_to_string_vec(remove_first_value(&mut obj, &["tasks", "items"]));
    if label.trim().is_empty() && tasks.is_empty() {
        return None;
    }

    let free_hours = value_to_f64(remove_first_value(
        &mut obj,
        &["free_hours", "free_time", "hours"],
    ))
    .max(0.0);
    Some(serde_json::json!({
        "label": label,
        "tasks": tasks,
        "free_hours": free_hours,
    }))
}

fn normalize_todo_urgency(urgency: String) -> String {
    match urgency.trim().to_ascii_lowercase().as_str() {
        "overdue" => "overdue".to_string(),
        "critical" | "urgent" => "critical".to_string(),
        "soon" | "warning" => "soon".to_string(),
        _ => "normal".to_string(),
    }
}

fn remove_first_value(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> serde_json::Value {
    for key in keys {
        if let Some(value) = obj.remove(*key) {
            return value;
        }
    }
    serde_json::Value::Null
}
