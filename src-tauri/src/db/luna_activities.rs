use super::*;

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
            .query_map([], |row| {
                Ok(LunaActivityRow {
                    luna_id: row.get(0)?,
                    activity_type: row.get(1)?,
                    title: row.get(2)?,
                    period: row.get(3)?,
                    status: row.get(4)?,
                    detail_path: row.get(5)?,
                })
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
