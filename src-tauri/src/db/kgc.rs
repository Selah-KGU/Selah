use super::*;

impl Database {
    // ── KGC courses ──

    /// Replace one verified week atomically so removed secondary rows disappear.
    pub(crate) fn replace_kgc_week(
        &self,
        label: &str,
        entries: &[KgcCourseRow],
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn.transaction().map_err(|e| format!("DB begin: {e}"))?;
        tx.execute("DELETE FROM kgc_courses WHERE week_label = ?1", [label])
            .map_err(|e| format!("DB clear KGC week: {e}"))?;
        for entry in entries {
            tx.execute(
                "INSERT OR REPLACE INTO kgc_courses (kgc_code, name, day, period, room, detail_path, is_cancelled, is_makeup, is_room_changed, week_label, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![entry.kgc_code, entry.name, entry.day, entry.period, entry.room, entry.detail_path,
                    entry.is_cancelled, entry.is_makeup, entry.is_room_changed, label, epoch_secs()],
            ).map_err(|e| format!("DB save KGC week: {e}"))?;
        }
        tx.commit().map_err(|e| format!("DB commit: {e}"))
    }

    pub fn get_kgc_courses(&self, week_label: &str) -> Result<Vec<KgcCourseRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        Self::query_kgc_courses(&conn, week_label)
    }

    pub(super) fn query_kgc_courses(
        conn: &Connection,
        week_label: &str,
    ) -> Result<Vec<KgcCourseRow>, String> {
        if week_label.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT id, kgc_code, name, day, period, room, detail_path, is_cancelled, is_makeup, is_room_changed, week_label
             FROM kgc_courses WHERE week_label = ?1 ORDER BY day, period"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map(params![week_label], |row| {
                Ok(KgcCourseRow {
                    id: row.get(0)?,
                    kgc_code: row.get(1)?,
                    name: row.get(2)?,
                    day: row.get(3)?,
                    period: row.get(4)?,
                    room: row.get(5)?,
                    detail_path: row.get(6)?,
                    is_cancelled: row.get::<_, i32>(7)? != 0,
                    is_makeup: row.get::<_, i32>(8)? != 0,
                    is_room_changed: row.get::<_, i32>(9)? != 0,
                    week_label: row.get(10)?,
                })
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get distinct kgc_codes that need session plan enrichment.
    pub fn kgc_codes_needing_plans(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let threshold = epoch_secs() - 24 * 3600;
        // A course needs plans if:
        //   1. its detail was never fetched (no row in kgc_course_details within 24h), OR
        //   2. it has fewer than 5 session_plans rows (previous parse bug / incomplete data)
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT c.kgc_code FROM kgc_courses c
             WHERE c.kgc_code != ''
               AND (
                 c.kgc_code NOT IN (SELECT kgc_code FROM kgc_course_details WHERE updated_at > ?1)
                 OR (SELECT COUNT(*) FROM session_plans sp WHERE sp.kgc_code = c.kgc_code) < 5
               )",
            )
            .map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map(params![threshold], |row| row.get::<_, String>(0))
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
