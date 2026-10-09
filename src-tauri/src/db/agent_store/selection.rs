//! Shared selection is admitted against the same durable conversation store.
use super::*;
use rusqlite::OptionalExtension;

pub(crate) const ACTIVE_CONV_KEY: &str = "agent_active_conversation";

impl Database {
    pub(crate) fn agent_active_conversation(&self) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        conn.query_row(
            "SELECT c.id FROM data_cache d JOIN agent_conversations c ON c.id=d.data_json
             WHERE d.cache_key=?1",
            params![ACTIVE_CONV_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("DB active conversation: {e}"))
    }

    pub(crate) fn agent_set_active_conversation(&self, id: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("DB selection transaction: {e}"))?;
        // A single guarded write avoids resurrecting a pointer to a deleted
        // conversation, including writers using a different SQLite connection.
        let changed = tx.execute(
            "INSERT INTO data_cache(cache_key,data_json,updated_at)
             SELECT ?1,?2,?3 WHERE ?2='' OR EXISTS(SELECT 1 FROM agent_conversations WHERE id=?2)
             ON CONFLICT(cache_key) DO UPDATE SET data_json=excluded.data_json, updated_at=excluded.updated_at
             WHERE data_cache.data_json<>excluded.data_json",
            params![ACTIVE_CONV_KEY, id, epoch_secs()],
        ).map_err(|e| format!("DB select conversation: {e}"))?;
        if changed == 0 && !id.is_empty() {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_conversations WHERE id=?1)",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("DB selection check: {e}"))?;
            if !exists {
                return Err("会話が見つかりません。新しい会話を選んでください。".into());
            }
        }
        tx.commit()
            .map_err(|e| format!("DB selection commit: {e}"))?;
        Ok(changed != 0)
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
