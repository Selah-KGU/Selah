use super::*;

fn activity_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<LunaActivityRow> {
    Ok(LunaActivityRow {
        luna_id: row.get(offset)?,
        activity_type: row.get(offset + 1)?,
        title: row.get(offset + 2)?,
        period: row.get(offset + 3)?,
        status: row.get(offset + 4)?,
        detail_path: row.get(offset + 5)?,
    })
}

impl Database {
    // ── Luna activities (detailed items) ──

    /// Replace all activities for a given luna_id with fresh data.
    pub fn replace_luna_activities(
        &self,
        luna_id: &str,
        activities: &[LunaActivityRow],
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let tx = conn.transaction().map_err(|e| format!("DB begin: {}", e))?;
        tx.execute(
            "DELETE FROM luna_activities WHERE luna_id = ?1",
            params![luna_id],
        )
        .map_err(|e| format!("DB delete activities: {}", e))?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO luna_activities (luna_id, activity_type, title, period, status, detail_path, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)"
            ).map_err(|e| format!("DB prepare: {}", e))?;
            for a in activities {
                stmt.execute(params![
                    luna_id,
                    a.activity_type,
                    a.title,
                    a.period,
                    a.status,
                    a.detail_path,
                    now
                ])
                .map_err(|e| format!("DB insert activity: {}", e))?;
            }
        }
        tx.commit().map_err(|e| format!("DB commit: {}", e))?;
        Ok(())
    }

    pub fn get_all_luna_activities(&self) -> Result<Vec<LunaActivityRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        Self::query_all_luna_activities(&conn)
    }

    pub(super) fn query_all_luna_activities(
        conn: &Connection,
    ) -> Result<Vec<LunaActivityRow>, String> {
        let mut stmt = conn.prepare(
            "SELECT luna_id, activity_type, title, period, status, detail_path FROM luna_activities ORDER BY luna_id, activity_type"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], |row| activity_row(row, 0))
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub(super) fn query_visible_luna_activities(
        conn: &Connection,
        year: &str,
        term: &str,
    ) -> Result<Vec<LunaActivityRow>, String> {
        let ids = super::scoped_rows::visible_luna_ids(conn, "luna_activities", year, term)?;
        let keys: Vec<_> = ids.iter().map(String::as_str).collect();
        let mut rows: Vec<(i64, LunaActivityRow)> = super::scoped_rows::query_selected(
            conn, &keys, "SELECT rowid, luna_id, activity_type, title, period, status, detail_path FROM luna_activities",
            "luna_id", "", |row| Ok((row.get(0)?, activity_row(row, 1)?)))?;
        rows.sort_unstable_by(|(id_a, a), (id_b, b)| {
            a.luna_id
                .cmp(&b.luna_id)
                .then_with(|| a.activity_type.cmp(&b.activity_type))
                .then_with(|| id_a.cmp(id_b))
        });
        Ok(rows.into_iter().map(|(_, activity)| activity).collect())
    }
}
