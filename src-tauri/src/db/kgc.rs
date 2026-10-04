use super::*;

impl Database {
    // ── KGC courses ──

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_kgc_course(
        &self,
        kgc_code: &str,
        name: &str,
        day: i32,
        period: i32,
        room: &str,
        detail_path: &str,
        is_cancelled: bool,
        is_makeup: bool,
        is_room_changed: bool,
        week_label: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        conn.execute(
            "INSERT INTO kgc_courses (kgc_code, name, day, period, room, detail_path, is_cancelled, is_makeup, is_room_changed, week_label, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(kgc_code, day, period, week_label) DO UPDATE SET
               name=?2, room=?5, detail_path=?6, is_cancelled=?7, is_makeup=?8, is_room_changed=?9, updated_at=?11",
            params![kgc_code, name, day, period, room, detail_path, is_cancelled as i32, is_makeup as i32, is_room_changed as i32, week_label, now],
        ).map_err(|e| format!("DB upsert kgc: {}", e))?;
        Ok(conn.last_insert_rowid())
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
