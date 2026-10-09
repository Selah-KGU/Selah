use super::*;
use rusqlite::OptionalExtension;

#[path = "agent_store/planning.rs"]
mod planning;

#[path = "agent_store/turn_history.rs"]
mod turn_history;

#[path = "agent_store/selection.rs"]
mod selection;

const DISPLAY_HISTORY_SQL: &str =
    "SELECT id, conv_id, role, content, images_json, NULL, NULL, created_at
     FROM agent_messages WHERE conv_id=?1 AND role IN ('user', 'assistant')
     ORDER BY created_at ASC, id ASC";

impl Database {
    // ── Agent conversations / messages ──

    /// A voice submission becomes visible only with its complete first message.
    /// Failure rolls back the conversation as well as the message.
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    pub(crate) fn agent_create_voice_turn(&self, id: &str, text: &str) -> Result<i64, String> {
        self.store_voice_turn(id, text, false)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    pub(crate) fn agent_retry_voice_turn(&self, id: &str, text: &str) -> Result<i64, String> {
        self.store_voice_turn(id, text, true)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    fn store_voice_turn(&self, id: &str, text: &str, retry: bool) -> Result<i64, String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("DB voice transaction: {e}"))?;
        if retry {
            // The first worker can unwind after a successful commit. Accept
            // only this exact first user message; never duplicate or replace it.
            let matching: Option<i64> = tx.query_row(
                "SELECT m.id FROM agent_messages m JOIN agent_conversations c ON c.id=m.conv_id WHERE m.conv_id=?1 AND m.role='user' AND m.content=?2 AND m.images_json IS NULL AND m.tool_name IS NULL AND m.tool_result_json IS NULL AND m.id=(SELECT MIN(id) FROM agent_messages WHERE conv_id=?1)",
                params![id, text], |row| row.get(0),
            ).optional().map_err(|e| format!("DB voice retry: {e}"))?;
            if let Some(message_id) = matching {
                tx.commit()
                    .map_err(|e| format!("DB voice retry commit: {e}"))?;
                return Ok(message_id);
            }
        }
        let now = epoch_secs();
        tx.execute(
            "INSERT INTO agent_conversations (id, title, created_at, updated_at) VALUES (?1, 'Voice Shortcut', ?2, ?2)",
            params![id, now],
        ).map_err(|e| format!("DB voice conversation: {e}"))?;
        tx.execute(
            "INSERT INTO agent_messages (conv_id, role, content, created_at) VALUES (?1, 'user', ?2, ?3)",
            params![id, text, now],
        ).map_err(|e| format!("DB voice message: {e}"))?;
        let message_id = tx.last_insert_rowid();
        tx.commit().map_err(|e| format!("DB voice commit: {e}"))?;
        Ok(message_id)
    }

    pub fn agent_create_conversation(&self, id: &str, title: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        conn.execute(
            "INSERT INTO agent_conversations (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![id, title, now],
        ).map_err(|e| format!("DB agent_create_conversation: {}", e))?;
        Ok(())
    }

    pub fn agent_list_conversations(&self) -> Result<Vec<AgentConversationRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at FROM agent_conversations ORDER BY updated_at DESC"
        ).map_err(|e| format!("DB prepare: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AgentConversationRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(|e| format!("DB map: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("DB read conversation: {error}"))
    }

    /// One conditional write prevents automatic titles from overwriting a
    /// manual rename, even if that rename races with turn preparation.
    pub(crate) fn agent_autotitle_conversation(
        &self,
        id: &str,
        title: &str,
    ) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let changed = conn.execute(
            "UPDATE agent_conversations SET title=?2, updated_at=?3 WHERE id=?1 AND title IN ('', '新しい会話', 'エージェント') AND title<>?2",
            params![id, title, epoch_secs()],
        ).map_err(|e| format!("DB agent_autotitle: {}", e))?;
        Ok(changed != 0)
    }

    pub fn agent_rename_conversation(&self, id: &str, title: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute(
            "UPDATE agent_conversations SET title=?2, updated_at=?3 WHERE id=?1",
            params![id, title, epoch_secs()],
        )
        .map_err(|e| format!("DB agent_rename: {}", e))?;
        Ok(())
    }

    pub fn agent_delete_conversation(&self, id: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("DB delete transaction: {e}"))?;
        tx.execute("DELETE FROM agent_messages WHERE conv_id=?1", params![id])
            .map_err(|e| format!("DB delete msgs: {e}"))?;
        tx.execute("DELETE FROM agent_conversations WHERE id=?1", params![id])
            .map_err(|e| format!("DB delete conv: {e}"))?;
        tx.execute(
            "UPDATE data_cache SET data_json='',updated_at=?3 WHERE cache_key=?1 AND data_json=?2 AND data_json<>''",
            params![selection::ACTIVE_CONV_KEY, id, epoch_secs()],
        ).map_err(|e| format!("DB clear deleted selection: {e}"))?;
        tx.commit().map_err(|e| format!("DB delete commit: {e}"))
    }

    pub fn agent_append_message(
        &self,
        conv_id: &str,
        role: &str,
        content: &str,
        images_json: Option<&str>,
        tool_name: Option<&str>,
        tool_result_json: Option<&str>,
    ) -> Result<i64, String> {
        self.append_message_fields(
            conv_id,
            role,
            content,
            images_json,
            tool_name,
            tool_result_json,
            None,
        )
    }

    pub(crate) fn agent_append_document_message(
        &self,
        conv_id: &str,
        content: &str,
        images_json: Option<&str>,
        documents_json: &str,
    ) -> Result<i64, String> {
        self.append_message_fields(
            conv_id,
            "user",
            content,
            images_json,
            None,
            None,
            Some(documents_json),
        )
    }

    pub(crate) fn agent_load_message_documents(
        &self,
        conv_id: &str,
    ) -> Result<std::collections::HashMap<i64, String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let mut stmt = conn.prepare("SELECT id,documents_json FROM agent_messages WHERE conv_id=?1 AND role='user' AND documents_json IS NOT NULL")
            .map_err(|e| format!("DB attachments: {e}"))?;
        let rows = stmt
            .query_map([conv_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("DB attachments: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("DB attachments: {e}"))
    }

    fn append_message_fields(
        &self,
        conv_id: &str,
        role: &str,
        content: &str,
        images_json: Option<&str>,
        tool_name: Option<&str>,
        tool_result_json: Option<&str>,
        documents_json: Option<&str>,
    ) -> Result<i64, String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("DB append transaction: {e}"))?;
        let now = epoch_secs();
        // Guard admission inside the write statement, including when foreign
        // keys are disabled. A late result cannot recreate a deleted chat.
        let inserted = tx.prepare_cached(
            "INSERT INTO agent_messages (conv_id, role, content, images_json, tool_name, tool_result_json, created_at, documents_json)
             SELECT id, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM agent_conversations WHERE id=?1",
        ).map_err(|e| format!("DB append prepare: {e}"))?
            .execute(params![conv_id, role, content, images_json, tool_name, tool_result_json, now, documents_json])
            .map_err(|e| format!("DB append msg: {e}"))?;
        if inserted != 1 {
            return Err("会話が見つかりません。新しい会話を選んでください。".into());
        }
        let id = tx.last_insert_rowid();
        // Multiple tool results in one second need no redundant metadata write.
        // An actual update failure must roll back the message as well.
        tx.prepare_cached(
            "UPDATE agent_conversations SET updated_at=?2 WHERE id=?1 AND updated_at<>?2",
        )
        .map_err(|e| format!("DB append timestamp prepare: {e}"))?
        .execute(params![conv_id, now])
        .map_err(|e| format!("DB append conversation timestamp: {e}"))?;
        tx.commit().map_err(|e| format!("DB append commit: {e}"))?;
        Ok(id)
    }

    pub fn agent_load_messages(&self, conv_id: &str) -> Result<Vec<AgentMessageRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, conv_id, role, content, images_json, tool_name, tool_result_json, created_at
             FROM agent_messages WHERE conv_id = ?1 ORDER BY created_at ASC, id ASC"
        ).map_err(|e| format!("DB prepare: {}", e))?;
        let rows = stmt
            .query_map(params![conv_id], message_row)
            .map_err(|e| format!("DB map: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("DB read message: {}", e))
    }

    /// Full visible history, without reading hidden tool payloads into memory.
    /// Stored messages and the model's full/recent history APIs remain intact.
    pub(crate) fn agent_load_display_messages(
        &self,
        conv_id: &str,
    ) -> Result<Vec<AgentMessageRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let mut stmt = conn
            .prepare_cached(DISPLAY_HISTORY_SQL)
            .map_err(|e| format!("DB display history prepare: {e}"))?;
        let rows = stmt
            .query_map(params![conv_id], message_row)
            .map_err(|e| format!("DB display history map: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("DB display history message: {e}"))
    }

    /// Read the newest rows with the same chronological order as full history.
    /// LIMIT is applied before deserializing old text, attachments or tool JSON.
    #[cfg(test)]
    pub(crate) fn agent_load_recent_messages(
        &self,
        conv_id: &str,
        limit: usize,
    ) -> Result<Vec<AgentMessageRow>, String> {
        let limit = i64::try_from(limit).map_err(|_| "DB history limit is too large")?;
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, conv_id, role, content, images_json, tool_name, tool_result_json, created_at
             FROM agent_messages WHERE conv_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2"
        ).map_err(|e| format!("DB prepare: {}", e))?;
        let rows = stmt
            .query_map(params![conv_id, limit], message_row)
            .map_err(|e| format!("DB map: {}", e))?;
        let mut rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("DB read message: {}", e))?;
        rows.reverse();
        Ok(rows)
    }
}

fn message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentMessageRow> {
    Ok(AgentMessageRow {
        id: row.get(0)?,
        conv_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        images_json: row.get(4)?,
        tool_name: row.get(5)?,
        tool_result_json: row.get(6)?,
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
#[path = "agent_store/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_store/write_tests.rs"]
mod write_tests;

#[cfg(test)]
#[path = "agent_store/display_tests.rs"]
mod display_tests;
