use super::*;

pub(super) async fn get_grades(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db
        .get_data_cache("grades")?
        .ok_or_else(|| "成績データがまだ取得されていません".to_string())?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let curriculum = v.get("curriculum").and_then(|x| x.as_array());
    let items: Vec<Value> = curriculum
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    json!({
                        "category": c.get("category").and_then(|x| x.as_str()).unwrap_or(""),
                        "required": c.get("required_credits").and_then(|x| x.as_str()).unwrap_or(""),
                        "earned": c.get("earned_credits").and_then(|x| x.as_str()).unwrap_or(""),
                        "enrolled": c.get("enrolled_credits").and_then(|x| x.as_str()).unwrap_or(""),
                        "deficit": c.get("is_deficit").and_then(|x| x.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "curriculum": items }))
}

/// Read entries from a cached JSON object, project specified string fields, and
/// wrap in a result key. Covers cancellations, makeup, room changes, exams.
fn read_cache_entries(
    db: &Database,
    cache_key: &str,
    result_key: &str,
    error_hint: &str,
    fields: &[&str],
) -> Result<Value, String> {
    let (json_str, _) = db
        .get_data_cache(cache_key)?
        .ok_or_else(|| format!("{}がまだ取得されていません", error_hint))?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let items: Vec<Value> = v
        .get("entries")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .take(LIST_CAP)
                .map(|entry| {
                    let mut obj = serde_json::Map::with_capacity(fields.len());
                    for &field in fields {
                        let val = entry
                            .get(field)
                            .cloned()
                            .unwrap_or(Value::String(String::new()));
                        obj.insert(field.to_string(), val);
                    }
                    Value::Object(obj)
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ result_key: items }))
}

pub(super) async fn get_cancellations(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(
        &db,
        "cancellations",
        "cancellations",
        "休講データ",
        &[
            "date",
            "period",
            "course_name",
            "instructor",
            "room",
            "comment",
        ],
    )
}

pub(super) async fn get_makeup_classes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(
        &db,
        "makeup",
        "makeup_classes",
        "補講データ",
        &[
            "date",
            "period",
            "course_name",
            "instructor",
            "room",
            "comment",
        ],
    )
}

pub(super) async fn get_room_changes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(
        &db,
        "rooms",
        "room_changes",
        "教室変更データ",
        &[
            "date",
            "course_name",
            "room",
            "instructor",
            "schedule",
            "comment",
        ],
    )
}

pub(super) async fn get_registration(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db
        .get_data_cache("registration")?
        .ok_or_else(|| "履修データがまだ取得されていません".to_string())?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let credit_summary = v
        .get("credit_summary")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    json!({
                        "semester": s.get("semester").and_then(|x| x.as_str()).unwrap_or(""),
                        "enrolled": s.get("enrolled").and_then(|x| x.as_str()).unwrap_or(""),
                        "limit": s.get("limit").and_then(|x| x.as_str()).unwrap_or(""),
                    })
                })
                .collect::<Vec<Value>>()
        })
        .unwrap_or_default();
    let courses = v
        .get("courses")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    json!({
                        "day": c.get("day").and_then(|x| x.as_str()).unwrap_or(""),
                        "period": c.get("period").and_then(|x| x.as_str()).unwrap_or(""),
                        "course_name": c.get("course_name").and_then(|x| x.as_str()).unwrap_or(""),
                        "instructor": c.get("instructor").and_then(|x| x.as_str()).unwrap_or(""),
                        "credits": c.get("credits").and_then(|x| x.as_str()).unwrap_or(""),
                        "room": c.get("room").and_then(|x| x.as_str()).unwrap_or(""),
                        "status": c.get("status").and_then(|x| x.as_str()).unwrap_or(""),
                    })
                })
                .collect::<Vec<Value>>()
        })
        .unwrap_or_default();
    Ok(json!({
        "year_semester": v.get("year_semester").and_then(|x| x.as_str()).unwrap_or(""),
        "credit_summary": credit_summary,
        "courses": courses,
    }))
}

pub(super) async fn get_exam_timetable(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(
        &db,
        "exam_timetable",
        "exams",
        "試験時間割",
        &["day", "period", "course_name", "room"],
    )
}
