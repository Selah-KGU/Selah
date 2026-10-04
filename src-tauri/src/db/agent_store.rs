use super::*;

impl Database {
    // ── Agent conversations / messages ──

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
        Ok(rows.filter_map(|r| r.ok()).collect())
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
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute("DELETE FROM agent_messages WHERE conv_id=?1", params![id])
            .map_err(|e| format!("DB delete msgs: {}", e))?;
        conn.execute("DELETE FROM agent_conversations WHERE id=?1", params![id])
            .map_err(|e| format!("DB delete conv: {}", e))?;
        Ok(())
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
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute(
            "INSERT INTO agent_messages (conv_id, role, content, images_json, tool_name, tool_result_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![conv_id, role, content, images_json, tool_name, tool_result_json, epoch_secs()],
        ).map_err(|e| format!("DB append msg: {}", e))?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE agent_conversations SET updated_at=?2 WHERE id=?1",
            params![conv_id, epoch_secs()],
        )
        .ok();
        Ok(id)
    }

    pub fn agent_load_messages(&self, conv_id: &str) -> Result<Vec<AgentMessageRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, conv_id, role, content, images_json, tool_name, tool_result_json, created_at
             FROM agent_messages WHERE conv_id = ?1 ORDER BY created_at ASC, id ASC"
        ).map_err(|e| format!("DB prepare: {}", e))?;
        let rows = stmt
            .query_map(params![conv_id], |row| {
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
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
