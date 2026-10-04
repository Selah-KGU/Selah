use super::*;

impl Database {
    // ── Session plans ──

    pub fn upsert_session_plans(
        &self,
        kgc_code: &str,
        plans: &[SessionPlanRow],
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let tx = conn.transaction().map_err(|e| format!("DB begin: {}", e))?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO session_plans (kgc_code, session_num, th_header, topic, delivery_mode, study_outside, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(kgc_code, session_num) DO UPDATE SET th_header=?3, topic=?4, delivery_mode=?5, study_outside=?6, updated_at=?7"
            ).map_err(|e| format!("DB prepare: {}", e))?;
            for p in plans {
                stmt.execute(params![
                    kgc_code,
                    p.session_num,
                    p.th_header,
                    p.topic,
                    p.delivery_mode,
                    p.study_outside,
                    now
                ])
                .map_err(|e| format!("DB upsert plan: {}", e))?;
            }
        }
        tx.commit().map_err(|e| format!("DB commit: {}", e))?;
        Ok(())
    }

    pub fn get_all_session_plans(&self) -> Result<Vec<(String, Vec<SessionPlanRow>)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        Self::query_all_session_plans(&conn)
    }

    pub(super) fn query_all_session_plans(
        conn: &Connection,
    ) -> Result<Vec<(String, Vec<SessionPlanRow>)>, String> {
        let mut stmt = conn.prepare(
            "SELECT kgc_code, session_num, th_header, topic, delivery_mode, study_outside FROM session_plans ORDER BY kgc_code, session_num"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    SessionPlanRow {
                        session_num: row.get(1)?,
                        th_header: row.get(2)?,
                        topic: row.get(3)?,
                        delivery_mode: row.get(4)?,
                        study_outside: row.get(5)?,
                    },
                ))
            })
            .map_err(|e| format!("DB map: {}", e))?;
        let mut map: std::collections::HashMap<String, Vec<SessionPlanRow>> = Default::default();
        for r in rows.flatten() {
            map.entry(r.0).or_default().push(r.1);
        }
        Ok(map.into_iter().collect())
    }

    /// The full 授業計画 per course name: `(name, [(session_num, topic,
    /// delivery_mode)])`, session-ordered. Used to align a Live note to its
    /// 真の第N回 by content, and to tell offline (Live-bearing) 回 from online
    /// ones that never produce a recording.
    pub fn get_planned_sessions_by_name(&self) -> Result<PlannedSessionsByName, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT c.name, sp.session_num, sp.topic, sp.delivery_mode
                 FROM session_plans sp
                 JOIN (SELECT DISTINCT kgc_code, name FROM kgc_courses) c
                   ON c.kgc_code = sp.kgc_code
                 ORDER BY c.name, sp.session_num",
            )
            .map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| format!("DB map: {}", e))?;
        let mut map: std::collections::HashMap<String, Vec<PlannedSession>> = Default::default();
        for (name, num, topic, mode) in rows.flatten() {
            map.entry(name).or_default().push((num, topic, mode));
        }
        Ok(map.into_iter().collect())
    }
}
