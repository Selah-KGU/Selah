use super::*;

const COURSE_PLANS: &str = "SELECT session_num, th_header, topic, delivery_mode, study_outside
     FROM session_plans WHERE kgc_code = ?1 ORDER BY session_num";
const PLAN_CODES: &str = "SELECT DISTINCT kgc_code FROM session_plans ORDER BY kgc_code";

fn plan_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, SessionPlanRow)> {
    Ok((
        row.get(0)?,
        SessionPlanRow {
            session_num: row.get(1)?,
            th_header: row.get(2)?,
            topic: row.get(3)?,
            delivery_mode: row.get(4)?,
            study_outside: row.get(5)?,
        },
    ))
}

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

    /// Read only one course's complete plans. LIVE historically matched codes
    /// after Rust's Unicode trim; retain that fallback without loading every
    /// course's plan text. Prefer the exact code, then the first sorted alias.
    pub(crate) fn get_session_plans_for_course(
        &self,
        kgc_code: &str,
    ) -> Result<Option<Vec<SessionPlanRow>>, String> {
        let code = kgc_code.trim();
        if code.is_empty() {
            return Ok(None);
        }
        let mut conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        // Exact lookup, alias selection and its rows see one SQLite snapshot.
        let tx = conn.transaction().map_err(|e| format!("DB begin: {e}"))?;
        let plans = Self::query_course_session_plans(&tx, code)?;
        let result = if !plans.is_empty() {
            Some(plans)
        } else {
            let alias = {
                let mut stmt = tx
                    .prepare_cached(PLAN_CODES)
                    .map_err(|e| format!("DB query plan codes: {e}"))?;
                let codes = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("DB map plan codes: {e}"))?;
                let mut alias = None;
                for value in codes {
                    let value = value.map_err(|e| format!("DB read plan code: {e}"))?;
                    if value.trim() == code {
                        alias = Some(value);
                        break;
                    }
                }
                alias
            };
            alias
                .map(|alias| Self::query_course_session_plans(&tx, &alias))
                .transpose()?
        };
        tx.commit().map_err(|e| format!("DB commit: {e}"))?;
        Ok(result)
    }

    fn query_course_session_plans(
        conn: &Connection,
        code: &str,
    ) -> Result<Vec<SessionPlanRow>, String> {
        let mut stmt = conn
            .prepare_cached(COURSE_PLANS)
            .map_err(|e| format!("DB query course plans: {e}"))?;
        let rows = stmt
            .query_map(params![code], |row| {
                Ok(SessionPlanRow {
                    session_num: row.get(0)?,
                    th_header: row.get(1)?,
                    topic: row.get(2)?,
                    delivery_mode: row.get(3)?,
                    study_outside: row.get(4)?,
                })
            })
            .map_err(|e| format!("DB map course plans: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("DB read course plan: {e}"))
    }

    pub(super) fn query_all_session_plans(
        conn: &Connection,
    ) -> Result<Vec<(String, Vec<SessionPlanRow>)>, String> {
        let mut stmt = conn.prepare(
            "SELECT kgc_code, session_num, th_header, topic, delivery_mode, study_outside FROM session_plans ORDER BY kgc_code, session_num"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt
            .query_map([], plan_row)
            .map_err(|e| format!("DB map: {}", e))?;
        let mut map: std::collections::HashMap<String, Vec<SessionPlanRow>> = Default::default();
        for r in rows.flatten() {
            map.entry(r.0).or_default().push(r.1);
        }
        Ok(map.into_iter().collect())
    }

    pub(super) fn query_visible_session_plans(
        conn: &Connection,
        codes: &[&str],
    ) -> Result<Vec<(String, Vec<SessionPlanRow>)>, String> {
        let rows = super::scoped_rows::query_selected(conn, codes,
            "SELECT kgc_code, session_num, th_header, topic, delivery_mode, study_outside FROM session_plans",
            "kgc_code", "ORDER BY kgc_code, session_num", plan_row)?;
        let mut map: std::collections::HashMap<String, Vec<SessionPlanRow>> = Default::default();
        for (code, plan) in rows {
            map.entry(code).or_default().push(plan);
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

#[cfg(test)]
#[path = "session_plans/tests.rs"]
mod tests;
