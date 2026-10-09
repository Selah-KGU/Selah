//! A consistent planning snapshot without materializing the whole conversation.
use super::*;

const RECENT_PLANNING_SQL: &str =
    "SELECT id, conv_id, role, content, images_json, tool_name, tool_result_json, created_at
     FROM agent_messages WHERE conv_id=?1
       AND (?3 IS NULL OR id<?3 OR id IN (SELECT value FROM json_each(?4)))
     ORDER BY created_at DESC, id DESC LIMIT ?2";
const OLDER_PLANNING_TOOL_SQL: &str = "SELECT id, tool_result_json FROM agent_messages
     WHERE conv_id=?1 AND (created_at, id)<(?2, ?3)
       AND role='tool' AND tool_result_json IS NOT NULL
       AND (?4 IS NULL OR id<?4 OR id IN (SELECT value FROM json_each(?5)))
     ORDER BY created_at DESC, id DESC";

impl Database {
    #[cfg(test)]
    pub(crate) fn agent_load_planning_messages(
        &self,
        conv_id: &str,
        limit: usize,
        older_tool: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<Vec<AgentMessageRow>, String> {
        self.load_planning_messages(conv_id, limit, older_tool, None)
    }

    pub(crate) fn agent_load_turn_planning_messages(
        &self,
        conv_id: &str,
        limit: usize,
        older_tool: Option<&dyn Fn(&str) -> bool>,
        history: &crate::agent_turn_scope::History,
    ) -> Result<Vec<AgentMessageRow>, String> {
        self.load_planning_messages(conv_id, limit, older_tool, Some(history))
    }

    fn load_planning_messages(
        &self,
        conv_id: &str,
        limit: usize,
        older_tool: Option<&dyn Fn(&str) -> bool>,
        history: Option<&crate::agent_turn_scope::History>,
    ) -> Result<Vec<AgentMessageRow>, String> {
        let limit = i64::try_from(limit).map_err(|_| "DB history limit is too large")?;
        let input_id = history.map(|history| history.input_message);
        let own_ids = match history {
            Some(history) => std::borrow::Cow::Owned(
                serde_json::to_string(&history.messages)
                    .map_err(|error| format!("DB planning message IDs: {error}"))?,
            ),
            None => std::borrow::Cow::Borrowed("[]"),
        };
        let path = {
            let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
            conn.path()
                .map(PathBuf::from)
                .ok_or("DB planning requires a file-backed database")?
        };
        // Reading/decoding an older image must not hold the shared connection's
        // mutex. WAL permits writes while this private read transaction runs.
        let mut reader =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| format!("DB planning reader: {e}"))?;
        // Match the warehouse connection's read mapping. This is per-connection
        // state and does not modify the database or acquire a write transaction.
        reader
            .pragma_update(None, "mmap_size", 33_554_432_i64)
            .map_err(|e| format!("DB planning mmap: {e}"))?;
        let tx = reader
            .transaction()
            .map_err(|e| format!("DB planning snapshot: {e}"))?;
        if let Some(input_id) = input_id {
            super::turn_history::check_input(&tx, conv_id, input_id)?;
        }
        let mut recent = {
            let mut stmt = tx
                .prepare(RECENT_PLANNING_SQL)
                .map_err(|e| format!("DB planning prepare: {e}"))?;
            let rows = stmt
                .query_map(
                    params![conv_id, limit, input_id, own_ids.as_ref()],
                    message_row,
                )
                .map_err(|e| format!("DB planning query: {e}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("DB read message: {e}"))?
        };
        recent.reverse();
        if let Some(matches) = older_tool {
            let already_present = recent.iter().rev().any(|row| {
                row.role == "tool" && row.tool_result_json.as_deref().is_some_and(matches)
            });
            if !already_present && recent.len() == limit as usize && !recent.is_empty() {
                let first = &recent[0];
                // Seek before the text window using the existing chronological
                // index. Only candidate JSON is decoded, one row at a time.
                let mut stmt = tx
                    .prepare(OLDER_PLANNING_TOOL_SQL)
                    .map_err(|e| format!("DB planning image prepare: {e}"))?;
                let candidates = stmt
                    .query_map(
                        params![
                            conv_id,
                            first.created_at,
                            first.id,
                            input_id,
                            own_ids.as_ref()
                        ],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                    )
                    .map_err(|e| format!("DB planning image query: {e}"))?;
                for candidate in candidates {
                    let (id, json) = candidate.map_err(|e| format!("DB read tool result: {e}"))?;
                    if matches(&json) {
                        let mut image_row = tx.query_row(
                            "SELECT id, conv_id, role, content, images_json, tool_name, NULL, created_at
                             FROM agent_messages WHERE id=?1", params![id], message_row,
                        ).map_err(|e| format!("DB read image message: {e}"))?;
                        // The candidate JSON already belongs to this snapshot;
                        // reuse its allocation instead of reading it twice.
                        image_row.tool_result_json = Some(json);
                        // This is older than a full text window, so it is only
                        // visible to image lookup, not the planner's text slice.
                        recent.insert(0, image_row);
                        break;
                    }
                }
            }
        }
        tx.commit()
            .map_err(|e| format!("DB planning snapshot commit: {e}"))?;
        Ok(recent)
    }
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;
