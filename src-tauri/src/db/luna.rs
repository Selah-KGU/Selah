use super::*;

impl Database {
    // ── Luna courses ──

    pub fn get_luna_courses(&self) -> Result<Vec<LunaCourseRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        Self::query_luna_courses(&conn)
    }

    pub(super) fn query_luna_courses(conn: &Connection) -> Result<Vec<LunaCourseRow>, String> {
        let (year, term) = Self::effective_luna_scope(conn);
        let mut stmt = conn.prepare(
            "SELECT id, luna_id, name, teacher, day, period FROM luna_courses ORDER BY day, period"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LunaCourseRow {
                    id: row.get(0)?,
                    luna_id: row.get(1)?,
                    name: row.get(2)?,
                    teacher: row.get(3)?,
                    day: row.get(4)?,
                    period: row.get(5)?,
                })
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|row| luna_course_matches_snapshot(&row.luna_id, &year, &term))
            .collect())
    }

    /// Replace the current timetable rows only. Activity and count tables stay,
    /// because they are keyed by luna_id and are not foreign keys.
    pub fn replace_luna_courses(&self, courses: &[luna_parser::LunaCourse]) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let tx = conn.transaction().map_err(|e| format!("DB begin: {}", e))?;
        tx.execute("DELETE FROM luna_courses", [])
            .map_err(|e| format!("DB delete luna courses: {}", e))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO luna_courses (luna_id, name, teacher, day, period, updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                )
                .map_err(|e| format!("DB prepare: {}", e))?;
            let mut seen = std::collections::HashSet::new();
            for course in courses {
                let day = course.day as i32;
                let period = course.period as i32;
                if !seen.insert((course.idnumber.clone(), day, period)) {
                    continue;
                }
                stmt.execute(params![
                    course.idnumber,
                    course.name,
                    course.teacher,
                    day,
                    period,
                    now
                ])
                .map_err(|e| format!("DB insert luna course: {}", e))?;
            }
        }
        tx.commit().map_err(|e| format!("DB commit: {}", e))?;
        Ok(())
    }

    fn snapshot_luna_scope(conn: &Connection) -> (String, String) {
        conn.query_row(
            "SELECT luna_year, luna_term FROM schedule_snapshot_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_else(|_| (String::new(), String::new()))
    }

    /// Calendar period wins over a stale snapshot, so a new spring does not
    /// keep reading the previous fall just because Luna has not synced yet.
    pub(super) fn effective_luna_scope(conn: &Connection) -> (String, String) {
        let (year, term) = Self::snapshot_luna_scope(conn);
        if let Some(period) =
            crate::academic_period::calendar_academic_period(chrono::Local::now().date_naive())
        {
            return (period.year, period.term);
        }
        (year, term)
    }

    /// Get luna_ids that need count enrichment (never fetched, or >1h stale).
    pub fn luna_ids_needing_counts(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let (year, term) = Self::effective_luna_scope(&conn);
        let threshold = epoch_secs() - 3 * 3600; // 3 hours
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT luna_id FROM luna_courses
             WHERE luna_id NOT IN (SELECT luna_id FROM luna_counts WHERE updated_at > ?1)",
            )
            .map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map(params![threshold], |row| row.get::<_, String>(0))
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|luna_id| luna_course_matches_snapshot(luna_id, &year, &term))
            .collect())
    }
}
