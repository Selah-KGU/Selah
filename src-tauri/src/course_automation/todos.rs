//! SenseA todo cache sync.
//!
//! Reads existing course todos for the planner and writes action items
//! back into the shared detail-todo cache. The cache key constants and
//! DetailTodo shape stay in the parent so tests can construct them.

use super::*;

fn sensea_todo_id(luna_id: &str, item_id: &str) -> String {
    format!("sensea-{}-{}", luna_id, item_id)
}

fn todo_context_course_name(value: &str) -> String {
    crate::commands::simplify_course_name(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_active_detail_todo(todo: &DetailTodo) -> bool {
    todo.completed_at.is_none() && todo.archived_at.is_none()
}

fn todo_context_same_course(course_name: &str, candidate: &str) -> bool {
    let course_name = todo_context_course_name(course_name);
    let candidate = todo_context_course_name(candidate);
    !course_name.is_empty() && course_name == candidate
}

fn push_existing_course_todo(
    todos: &mut Vec<context::ExistingCourseTodo>,
    seen: &mut HashSet<String>,
    title: impl Into<String>,
    content_type: impl Into<String>,
    deadline: impl Into<String>,
    status: impl Into<String>,
    source: impl Into<String>,
) {
    let todo = context::ExistingCourseTodo {
        title: title.into().trim().to_string(),
        content_type: content_type.into().trim().to_string(),
        deadline: deadline.into().trim().to_string(),
        status: status.into().trim().to_string(),
        source: source.into().trim().to_string(),
    };
    if todo.title.is_empty() {
        return;
    }
    let key = format!(
        "{}|{}|{}|{}|{}",
        todo.title.to_lowercase(),
        todo.content_type.to_lowercase(),
        todo.deadline.to_lowercase(),
        todo.status.to_lowercase(),
        todo.source.to_lowercase()
    );
    if seen.insert(key) {
        todos.push(todo);
    }
}

pub(super) fn load_existing_course_todos(
    db: &Database,
    course_name: &str,
) -> Vec<context::ExistingCourseTodo> {
    const MAX_EXISTING_COURSE_TODOS: usize = 40;

    let mut todos = Vec::new();
    let mut seen = HashSet::new();

    let luna_todos: Vec<crate::luna_parser::LunaTodoItem> =
        load_json(db, LUNA_TODO_CACHE_KEY).unwrap_or_default();
    for todo in luna_todos {
        if !todo_context_same_course(course_name, &todo.course_name) {
            continue;
        }
        push_existing_course_todo(
            &mut todos,
            &mut seen,
            todo.content_name,
            todo.content_type,
            todo.deadline,
            todo.status,
            "LUNA",
        );
        if todos.len() >= MAX_EXISTING_COURSE_TODOS {
            return todos;
        }
    }

    for (cache_key, source) in [
        (DETAIL_TODO_CACHE_KEY, "アプリ内追加"),
        (LIVE_TODO_CACHE_KEY, "Live"),
    ] {
        let generated: Vec<DetailTodo> = load_json(db, cache_key).unwrap_or_default();
        for todo in generated {
            if !is_active_detail_todo(&todo)
                || !todo_context_same_course(course_name, &todo.course_name)
            {
                continue;
            }
            push_existing_course_todo(
                &mut todos,
                &mut seen,
                todo.title,
                todo.content_type,
                todo.deadline,
                "未完了",
                source,
            );
            if todos.len() >= MAX_EXISTING_COURSE_TODOS {
                return todos;
            }
        }
    }

    todos
}

pub(super) async fn refresh_luna_todo_cache_for_agent(app: &AppHandle) {
    match crate::luna_commands::luna_fetch_todo(
        app.state::<crate::LunaState>(),
        app.state::<Database>(),
    )
    .await
    {
        Ok(items) => {
            log::info!(
                "[course_automation] refreshed {} existing LUNA todos for AI context",
                items.len()
            );
        }
        Err(error) => {
            log::warn!(
                "[course_automation] existing LUNA todo refresh failed; using cache if present: {}",
                error
            );
        }
    }
}

/// The TODO sink: reconciles the main TODO page with this course's action
/// findings. Runs in the backend (not the dock), so delivery happens on every
/// summary regardless of whether the UI is open. Adds a todo for each active
/// action item, reactivates/updates existing ones, and completes orphans whose
/// finding has expired, been acknowledged, or disappeared — keeping the two
/// sides in sync. Emits a cache-update only when something changed so other
/// windows refresh. Best-effort: failures are logged, never abort.
pub(super) fn sync_action_todos(app: &AppHandle, db: &Database, status: &CourseAutomationStatus) {
    let owned_prefix = format!("sensea-{}-", status.luna_id);
    let now = chrono::Utc::now().to_rfc3339();

    // Active action items the student still needs to do.
    let desired: Vec<&Item> = status
        .analysis
        .findings
        .iter()
        .filter(|item| {
            item.flags.action
                && !matches!(
                    status.item_states.get(&item.id).map(String::as_str),
                    Some("done") | Some("known")
                )
        })
        .collect();

    let mut todos: Vec<DetailTodo> = load_json(db, DETAIL_TODO_CACHE_KEY).unwrap_or_default();
    let original = todos.clone();
    // Drop legacy frontend-pushed SenseA todos (marked by note, not our id), so
    // ownership moves cleanly to the backend without leaving duplicates.
    todos.retain(|todo| !todo.note.starts_with("SenseA") || todo.id.starts_with("sensea-"));

    // Reconcile our own todos: reactivate+update those still desired, complete
    // those no longer desired.
    for todo in todos.iter_mut() {
        if !todo.id.starts_with(&owned_prefix) {
            continue;
        }
        if let Some(item) = desired
            .iter()
            .find(|item| sensea_todo_id(&status.luna_id, &item.id) == todo.id)
        {
            todo.completed_at = None;
            todo.archived_at = None;
            todo.title = item.text.clone();
            todo.deadline = item.expires_at.clone();
        } else if todo.completed_at.is_none() {
            todo.completed_at = Some(now.clone());
        }
    }

    // Add todos for desired items we don't have yet.
    let mut existing: HashSet<String> = todos.iter().map(|todo| todo.id.clone()).collect();
    for item in &desired {
        let id = sensea_todo_id(&status.luna_id, &item.id);
        if !existing.insert(id.clone()) {
            continue;
        }
        todos.push(DetailTodo {
            id,
            title: item.text.clone(),
            course_name: status.course_name.clone(),
            content_type: "課題".into(),
            deadline: item.expires_at.clone(),
            note: "自動検知".into(),
            created_at: now.clone(),
            ..Default::default()
        });
    }

    if todos == original {
        return;
    }
    if let Err(error) = save_json(db, DETAIL_TODO_CACHE_KEY, &todos) {
        log::warn!("[course_automation] todo sink write failed: {}", error);
        return;
    }
    // Reuse the app's cache-sync event so the main TODO page rebuilds luna_todo
    // (which re-merges the detail-generated todos we just wrote).
    let _ = app.emit(
        "backend-cache-updated",
        json!({ "keys": [LUNA_TODO_CACHE_KEY] }),
    );
}
