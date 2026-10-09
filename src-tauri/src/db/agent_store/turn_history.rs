//! Model history uses a committed input ID, not the wall clock or final row.
use super::*;

const PRIOR_HISTORY_SQL: &str =
    "SELECT id, conv_id, role, content, images_json, tool_name, tool_result_json, created_at
     FROM agent_messages WHERE conv_id=?1 AND id<?2 ORDER BY created_at DESC,id DESC LIMIT ?3";

pub(super) fn check_input(
    tx: &rusqlite::Transaction<'_>,
    conv_id: &str,
    input_id: i64,
) -> Result<(), String> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_messages WHERE id=?1 AND conv_id=?2 AND role='user')",
        params![input_id, conv_id], |row| row.get(0),
    ).map_err(|error| format!("DB input history boundary: {error}"))?;
    if !exists {
        return Err("入力の保存記録が見つかりません".into());
    }
    Ok(())
}

impl Database {
    pub(crate) fn agent_load_turn_prior_messages(
        &self,
        conv_id: &str,
        input_id: i64,
        limit: usize,
    ) -> Result<Vec<AgentMessageRow>, String> {
        let limit = i64::try_from(limit).map_err(|_| "DB history limit is too large")?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| format!("DB lock: {error}"))?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("DB input history snapshot: {error}"))?;
        check_input(&tx, conv_id, input_id)?;
        let mut result = {
            let mut statement = tx
                .prepare_cached(PRIOR_HISTORY_SQL)
                .map_err(|error| format!("DB input history prepare: {error}"))?;
            let rows = statement
                .query_map(params![conv_id, input_id, limit], message_row)
                .map_err(|error| format!("DB input history query: {error}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("DB input history read: {error}"))?
        };
        result.reverse();
        tx.commit()
            .map_err(|error| format!("DB input history commit: {error}"))?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "turn_history/tests.rs"]
mod tests;
