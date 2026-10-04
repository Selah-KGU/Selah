//! Load and save SenseA course config and status.
//!
//! Status writes skip SQLite and the dock event when the serialized payload
//! has not changed since the last emit.

use super::*;

pub(super) fn load_student_profile(db: &Database) -> Value {
    let profile = db
        .get_data_cache("student_profile")
        .ok()
        .flatten()
        .and_then(|(raw, _)| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({}));
    context::compact_student_profile(&profile)
}

pub(super) fn load_view(db: &Database, luna_id: &str, course_name: &str) -> CourseAutomationView {
    CourseAutomationView {
        config: load_config(db, luna_id, course_name),
        status: load_status(db, luna_id, course_name),
    }
}

pub(super) fn load_config(
    db: &Database,
    luna_id: &str,
    course_name: &str,
) -> CourseAutomationConfig {
    load_json(db, &config_key(luna_id)).unwrap_or_else(|| {
        CourseAutomationConfig::new(luna_id.to_string(), course_name.to_string())
    })
}

pub(super) fn load_status(
    db: &Database,
    luna_id: &str,
    course_name: &str,
) -> CourseAutomationStatus {
    let mut status: CourseAutomationStatus =
        load_json(db, &status_key(luna_id)).unwrap_or_else(|| CourseAutomationStatus {
            luna_id: luna_id.to_string(),
            course_name: course_name.to_string(),
            ..Default::default()
        });
    // Drop legacy run-log entries from before the per-operation format (they have
    // no `level`), so the log only shows the new file/operation lines.
    status
        .run_log
        .retain(|entry| !entry.level.trim().is_empty());
    migrate_legacy_status_errors(&mut status);
    status
}

pub(super) fn load_json<T: for<'de> Deserialize<'de>>(db: &Database, key: &str) -> Option<T> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(raw, _)| serde_json::from_str(&raw).ok())
}

pub(super) fn save_json<T: Serialize>(db: &Database, key: &str, value: &T) -> Result<(), String> {
    let raw = serde_json::to_string(value).map_err(|error| error.to_string())?;
    db.save_data_cache(key, &raw)
}

pub(super) fn save_status_and_emit(
    app: &AppHandle,
    db: &Database,
    status: &CourseAutomationStatus,
) -> Result<(), String> {
    use std::hash::{Hash, Hasher};
    let raw = serde_json::to_string(status).map_err(|error| error.to_string())?;
    let key = status_key(&status.luna_id);
    let digest = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        raw.hash(&mut hasher);
        hasher.finish()
    };
    // A cycle fires this on every reused artifact/document, so most calls carry
    // no change. Skip the SQLite write and the IPC event (which re-renders the
    // dock) when the serialized status matches what we last persisted.
    let state = app.state::<CourseAutomationState>();
    if state
        .last_emitted
        .lock()
        .map(|last| last.get(&key) == Some(&digest))
        .unwrap_or(false)
    {
        return Ok(());
    }
    db.save_data_cache(&key, &raw)?;
    app.emit("course-automation-updated", status)
        .map_err(|error| error.to_string())?;
    if let Ok(mut last) = state.last_emitted.lock() {
        last.insert(key, digest);
    }
    Ok(())
}

pub(super) fn config_key(luna_id: &str) -> String {
    format!("{}{}", CONFIG_PREFIX, luna_id)
}

pub(super) fn status_key(luna_id: &str) -> String {
    format!("{}{}", STATUS_PREFIX, luna_id)
}

pub(super) fn sha256_json<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    let raw = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

pub(super) fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&text[start..start + offset + character.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}
