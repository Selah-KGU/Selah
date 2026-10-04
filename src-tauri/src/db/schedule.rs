use super::*;

impl Database {
    // ── AI schedule cache ──

    pub fn get_ai_schedule_cache(&self) -> Result<Option<(AiScheduleResult, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let result: Option<(String, i64)> = conn
            .query_row(
                "SELECT result_json, updated_at FROM ai_schedule_cache WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        match result {
            Some((json, ts)) => {
                let parsed: AiScheduleResult =
                    serde_json::from_str(&json).map_err(|e| format!("AI cache parse: {}", e))?;
                Ok(Some((parsed, ts)))
            }
            None => Ok(None),
        }
    }

    pub fn save_ai_schedule_cache(&self, result: &AiScheduleResult) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let json =
            serde_json::to_string(result).map_err(|e| format!("AI cache serialize: {}", e))?;
        conn.execute(
            "INSERT INTO ai_schedule_cache (id, result_json, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET result_json=?1, updated_at=?2",
            params![json, now],
        )
        .map_err(|e| format!("DB save ai cache: {}", e))?;
        Ok(())
    }

    // ── Snapshot state ──

    pub fn save_snapshot_state(&self, state: &SnapshotState) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let communities_json = serde_json::to_string(&state.luna_communities)
            .map_err(|e| format!("serialize communities: {}", e))?;
        let year_options_json = serde_json::to_string(&state.luna_year_options)
            .map_err(|e| format!("serialize year_options: {}", e))?;
        let term_options_json = serde_json::to_string(&state.luna_term_options)
            .map_err(|e| format!("serialize term_options: {}", e))?;
        conn.execute(
            "INSERT INTO schedule_snapshot_state (id, current_week_label, next_week_label, luna_year, luna_term, luna_communities_json, luna_year_options_json, luna_term_options_json, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET current_week_label=?1, next_week_label=?2, luna_year=?3, luna_term=?4, luna_communities_json=?5, luna_year_options_json=?6, luna_term_options_json=?7, updated_at=?8",
            params![state.current_week_label, state.next_week_label, state.luna_year, state.luna_term, communities_json, year_options_json, term_options_json, now],
        ).map_err(|e| format!("DB save snapshot state: {}", e))?;
        Ok(())
    }

    /// Timestamp only. Used to skip rebuilding the timetable snapshot on focus.
    pub fn schedule_snapshot_updated_at(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        match conn.query_row(
            "SELECT updated_at FROM schedule_snapshot_state WHERE id = 1",
            [],
            |row| row.get(0),
        ) {
            Ok(updated_at) => Ok(updated_at),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(format!("DB snapshot stamp: {}", e)),
        }
    }

    pub fn get_snapshot_state(&self) -> Result<Option<SnapshotState>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        #[allow(clippy::type_complexity)]
        let result: Option<(String, String, String, String, String, String, String, i64)> = conn.query_row(
            "SELECT current_week_label, next_week_label, luna_year, luna_term, luna_communities_json, luna_year_options_json, luna_term_options_json, updated_at FROM schedule_snapshot_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        ).ok();
        match result {
            Some((cwl, nwl, ly, lt, comm_json, yo_json, to_json, updated_at)) => {
                let luna_communities: Vec<luna_parser::LunaCommunity> =
                    serde_json::from_str(&comm_json).unwrap_or_default();
                let luna_year_options: Vec<luna_parser::SelectOption> =
                    serde_json::from_str(&yo_json).unwrap_or_default();
                let luna_term_options: Vec<luna_parser::SelectOption> =
                    serde_json::from_str(&to_json).unwrap_or_default();
                Ok(Some(SnapshotState {
                    current_week_label: cwl,
                    next_week_label: nwl,
                    luna_year: ly,
                    luna_term: lt,
                    luna_communities,
                    luna_year_options,
                    luna_term_options,
                    updated_at,
                }))
            }
            None => Ok(None),
        }
    }

    /// Build the raw data snapshot for AI analysis.
    /// Acquires the lock once and runs all queries in a single scope for consistency.
    pub fn build_raw_data(
        &self,
        current_week_label: &str,
        next_week_label: &str,
        luna_communities: Vec<crate::luna_parser::LunaCommunity>,
    ) -> Result<ScheduleRawData, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let kgc_current = Self::query_kgc_courses(&conn, current_week_label)?;
        let kgc_next = Self::query_kgc_courses(&conn, next_week_label)?;
        let luna_courses = Self::query_luna_courses(&conn)?;
        let (year, term) = Self::effective_luna_scope(&conn);
        let visible_codes: std::collections::HashSet<&str> = kgc_current
            .iter()
            .chain(kgc_next.iter())
            .map(|row| row.kgc_code.as_str())
            .filter(|code| !code.is_empty())
            .collect();
        let session_plans = Self::query_all_session_plans(&conn)?
            .into_iter()
            .filter(|(code, _)| visible_codes.contains(code.as_str()))
            .collect();
        let luna_counts = Self::query_all_luna_counts(&conn)?
            .into_iter()
            .filter(|(id, _)| luna_course_matches_snapshot(id, &year, &term))
            .collect();
        let luna_activities = Self::query_all_luna_activities(&conn)?
            .into_iter()
            .filter(|row| luna_course_matches_snapshot(&row.luna_id, &year, &term))
            .collect();
        let kgc_course_details = Self::query_all_kgc_course_details(&conn)?
            .into_iter()
            .filter(|detail| visible_codes.contains(detail.kgc_code.as_str()))
            .collect();
        Ok(ScheduleRawData {
            kgc_entries_current: kgc_current,
            kgc_entries_next: kgc_next,
            luna_courses,
            session_plans,
            luna_counts,
            luna_activities,
            kgc_course_details,
            current_week_label: current_week_label.to_string(),
            next_week_label: next_week_label.to_string(),
            luna_communities,
        })
    }
}
