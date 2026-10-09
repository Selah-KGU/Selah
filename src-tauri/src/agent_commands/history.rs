//! The display path never reads or deserializes hidden tool history.
use super::{worker, AgentMessageDto, Database};

pub(super) fn load_display_messages(
    db: &Database,
    id: &str,
) -> Result<Vec<AgentMessageDto>, String> {
    restore_documents(
        db,
        id,
        db.agent_load_display_messages(id)?
            .into_iter()
            .map(Into::into)
            .collect(),
    )
}

fn restore_documents(
    db: &Database,
    id: &str,
    mut rows: Vec<AgentMessageDto>,
) -> Result<Vec<AgentMessageDto>, String> {
    let mut documents = db.agent_load_message_documents(id)?;
    for row in &mut rows {
        if let Some(json) = documents.remove(&row.id) {
            let saved: crate::agent_attachments::SavedDocuments = serde_json::from_str(&json)
                .map_err(|e| format!("添付履歴を読み込めません: {e}"))?;
            row.content = saved.content;
            row.documents = saved.documents;
        }
    }
    Ok(rows)
}

/// Pre-encode JSON on the blocking worker as well. Response passes this JSON
/// body through Tauri without serializing it again on an async executor thread.
pub(super) fn load_display_response(
    db: &Database,
    id: &str,
) -> Result<tauri::ipc::Response, String> {
    let rows = load_display_messages(db, id)?;
    worker::json_response(&rows)
}

pub(super) fn load_full_response(db: &Database, id: &str) -> Result<tauri::ipc::Response, String> {
    let rows = db
        .agent_load_messages(id)?
        .into_iter()
        .map(Into::into)
        .collect::<Vec<AgentMessageDto>>();
    worker::json_response(&restore_documents(db, id, rows)?)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
