//! Agent history I/O and tool JSON encoding run outside the async executor.
use super::*;

async fn with_database<R: Send + 'static>(
    app: &AppHandle,
    conv_id: &str,
    work: impl FnOnce(&Database, &str) -> Result<R, AgentError> + Send + 'static,
) -> Result<R, AgentError> {
    let app = app.clone();
    let conv_id = conv_id.to_owned();
    prepare::blocking(move || work(&app.state::<Database>(), &conv_id)).await
}

pub(super) async fn load_planning_history(
    app: &AppHandle,
    conv_id: &str,
    vision: bool,
) -> Result<Vec<crate::db::AgentMessageRow>, AgentError> {
    let history = crate::agent_turn_scope::current(conv_id)
        .ok_or_else(|| AgentError::db("要求の履歴所有者が見つかりません"))?
        .history()?;
    with_database(app, conv_id, move |db, id| {
        planning_history_for_turn(db, id, vision, &history)
    })
    .await
}

#[cfg(test)]
fn planning_history(
    db: &Database,
    id: &str,
    vision: bool,
) -> Result<Vec<crate::db::AgentMessageRow>, AgentError> {
    db.agent_load_planning_messages(
        id,
        CFG.plan_history_turns.max(6),
        if vision {
            Some(&tool_result::has_screenshot_image as &dyn Fn(&str) -> bool)
        } else {
            None
        },
    )
    .map_err(AgentError::db)
}

fn planning_history_for_turn(
    db: &Database,
    id: &str,
    vision: bool,
    history: &crate::agent_turn_scope::History,
) -> Result<Vec<crate::db::AgentMessageRow>, AgentError> {
    // Tool-skip policy inspects six rows, even for the local model's two-row
    // prompt. Keep that policy independent of its text prompt window.
    let limit = CFG.plan_history_turns.max(6);
    let image = if vision {
        Some(&tool_result::has_screenshot_image as &dyn Fn(&str) -> bool)
    } else {
        None
    };
    db.agent_load_turn_planning_messages(id, limit, image, history)
        .map_err(AgentError::db)
}

pub(super) async fn save_answer(
    app: &AppHandle,
    conv_id: &str,
    answer: String,
) -> Result<(), AgentError> {
    let owner = crate::agent_turn_scope::current(conv_id)
        .ok_or_else(|| AgentError::db("要求の履歴所有者が見つかりません"))?;
    with_database(app, conv_id, move |db, id| {
        persist_answer(db, id, &answer, Some(&owner))
    })
    .await
}

fn persist_answer(
    db: &Database,
    conv_id: &str,
    answer: &str,
    owner: Option<&crate::agent_turn_scope::Turn>,
) -> Result<(), AgentError> {
    let message_id = db
        .agent_append_message(conv_id, "assistant", answer, None, None, None)
        .map_err(AgentError::db)?;
    if let Some(owner) = owner {
        owner.record_message(message_id);
    }
    Ok(())
}

pub(super) async fn save_tool_result(
    app: &AppHandle,
    conv_id: &str,
    name: &str,
    result: Value,
) -> Result<Value, AgentError> {
    let name = name.to_owned();
    let owner = crate::agent_turn_scope::current(conv_id)
        .ok_or_else(|| AgentError::db("要求の履歴所有者が見つかりません"))?;
    with_database(app, conv_id, move |db, id| {
        persist_tool_result(db, id, &name, result, Some(&owner))
    })
    .await
}

// Move the result into the worker and return it after persistence. Screenshots
// and downloaded content are neither cloned nor encoded on an async thread.
fn persist_tool_result(
    db: &Database,
    conv_id: &str,
    name: &str,
    result: Value,
    owner: Option<&crate::agent_turn_scope::Turn>,
) -> Result<Value, AgentError> {
    let json = serde_json::to_string(&result).map_err(|e| AgentError::db(e.to_string()))?;
    let message_id = db
        .agent_append_message(conv_id, "tool", "", None, Some(name), Some(&json))
        .map_err(AgentError::db)?;
    if let Some(owner) = owner {
        owner.record_message(message_id);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "persistence/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "persistence/history_tests.rs"]
mod history_tests;
