pub(in crate::timetable::ai_analysis::response_json) fn value_to_i32(v: serde_json::Value) -> i32 {
    match v {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        serde_json::Value::String(s) => s.trim().parse::<i32>().unwrap_or(0),
        serde_json::Value::Bool(b) => {
            if b {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

pub(in crate::timetable::ai_analysis::response_json) fn value_to_bool(
    v: serde_json::Value,
) -> bool {
    match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        serde_json::Value::String(s) => {
            let t = s.trim().to_lowercase();
            t == "true" || t == "1" || t == "yes"
        }
        _ => false,
    }
}

pub(in crate::timetable::ai_analysis::response_json) fn value_to_f64(v: serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        serde_json::Value::Bool(b) => {
            if b {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

pub(in crate::timetable::ai_analysis::response_json) fn value_to_string_vec(
    v: serde_json::Value,
) -> Vec<String> {
    match v {
        serde_json::Value::Array(a) => a
            .into_iter()
            .map(value_to_string)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        serde_json::Value::Null => Vec::new(),
        other => {
            let s = value_to_string(other).trim().to_string();
            if s.is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
    }
}

pub(in crate::timetable::ai_analysis::response_json) fn value_to_string(
    v: serde_json::Value,
) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => {
            if b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        serde_json::Value::Array(a) => {
            let parts: Vec<String> = a
                .into_iter()
                .map(value_to_string)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            parts.join(" / ")
        }
        serde_json::Value::Object(mut m) => {
            for key in [
                "text", "value", "name", "title", "content", "teacher", "room",
            ] {
                if let Some(val) = m.remove(key) {
                    let s = value_to_string(val).trim().to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            serde_json::to_string(&serde_json::Value::Object(m)).unwrap_or_default()
        }
    }
}
