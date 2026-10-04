use super::*;

impl Database {
    // ── Luna counts ──

    pub fn upsert_luna_counts(&self, luna_id: &str, counts: &LunaCountsRow) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        conn.execute(
            "INSERT INTO luna_counts (luna_id, announcements, new_announcements, reports, exams, discussions, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(luna_id) DO UPDATE SET announcements=?2, new_announcements=?3, reports=?4, exams=?5, discussions=?6, updated_at=?7",
            params![luna_id, counts.announcements, counts.new_announcements, counts.reports, counts.exams, counts.discussions, now],
        ).map_err(|e| format!("DB upsert counts: {}", e))?;
        Ok(())
    }

    pub fn get_all_luna_counts(&self) -> Result<Vec<(String, LunaCountsRow)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        Self::query_all_luna_counts(&conn)
    }

    pub(super) fn query_all_luna_counts(
        conn: &Connection,
    ) -> Result<Vec<(String, LunaCountsRow)>, String> {
        let mut stmt = conn.prepare(
            "SELECT luna_id, announcements, new_announcements, reports, exams, discussions FROM luna_counts"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    LunaCountsRow {
                        announcements: row.get(1)?,
                        new_announcements: row.get(2)?,
                        reports: row.get(3)?,
                        exams: row.get(4)?,
                        discussions: row.get(5)?,
                    },
                ))
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
