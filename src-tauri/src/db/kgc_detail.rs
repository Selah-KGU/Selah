use super::*;

impl Database {
    // ── KGC course details ──

    pub fn upsert_kgc_course_detail(&self, detail: &KgcCourseDetailRow) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let fields_json = serde_json::to_string(&detail.fields)
            .map_err(|e| format!("serialize fields: {}", e))?;
        let textbooks_json = serde_json::to_string(&detail.textbooks)
            .map_err(|e| format!("serialize textbooks: {}", e))?;
        conn.execute(
            "INSERT INTO kgc_course_details (kgc_code, fields_json, delivery_mode, textbooks_json, updated_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(kgc_code) DO UPDATE SET fields_json=?2, delivery_mode=?3, textbooks_json=?4, updated_at=?5",
            params![detail.kgc_code, fields_json, detail.delivery_mode, textbooks_json, now],
        ).map_err(|e| format!("DB upsert detail: {}", e))?;
        Ok(())
    }

    pub fn get_kgc_course_detail(
        &self,
        kgc_code: &str,
    ) -> Result<Option<KgcCourseDetailRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kgc_code, fields_json, delivery_mode, COALESCE(textbooks_json, '[]') FROM kgc_course_details WHERE kgc_code = ?1"
        ).map_err(|e| format!("DB query: {}", e))?;
        let mut rows = stmt
            .query_map(params![kgc_code], |row| {
                let kgc_code: String = row.get(0)?;
                let fields_json: String = row.get(1)?;
                let delivery_mode: String = row.get(2)?;
                let textbooks_json: String = row.get(3)?;
                let fields: Vec<(String, String)> =
                    serde_json::from_str(&fields_json).unwrap_or_default();
                let textbooks = serde_json::from_str(&textbooks_json).unwrap_or_default();
                Ok(KgcCourseDetailRow {
                    kgc_code,
                    fields,
                    delivery_mode,
                    textbooks,
                })
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    pub(super) fn query_all_kgc_course_details(
        conn: &Connection,
    ) -> Result<Vec<KgcCourseDetailRow>, String> {
        let mut stmt = conn.prepare(
            "SELECT kgc_code, fields_json, delivery_mode, COALESCE(textbooks_json, '[]') FROM kgc_course_details"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                let kgc_code: String = row.get(0)?;
                let fields_json: String = row.get(1)?;
                let delivery_mode: String = row.get(2)?;
                let textbooks_json: String = row.get(3)?;
                let fields: Vec<(String, String)> =
                    serde_json::from_str(&fields_json).unwrap_or_default();
                let textbooks = serde_json::from_str(&textbooks_json).unwrap_or_default();
                Ok(KgcCourseDetailRow {
                    kgc_code,
                    fields,
                    delivery_mode,
                    textbooks,
                })
            })
            .map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
