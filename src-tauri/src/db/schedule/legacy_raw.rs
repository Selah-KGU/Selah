// Frozen raw-data assembly before filtering body queries by visible keys. Test only.
use super::*;

impl Database {
    pub(super) fn build_raw_data_before(
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
