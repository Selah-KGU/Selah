use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::luna_parser;

/// SQLite database — raw data warehouse for KGC / Luna + AI schedule cache.
///
/// Tables:
///   kgc_courses     – raw KGC timetable entries (per week_label)
///   luna_courses     – raw Luna timetable entries
///   session_plans    – parsed 授業計画 keyed by kgc_code
///   luna_counts      – Luna LMS activity counts keyed by luna_id
///   ai_schedule_cache – cached AI-generated schedule JSON
pub struct Database {
    conn: Mutex<Connection>,
}

// ── Row types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPlanRow {
    pub session_num: i32,
    pub th_header: String,
    pub topic: String,
    pub delivery_mode: String,
    pub study_outside: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCountsRow {
    pub announcements: i32,
    pub new_announcements: i32,
    pub reports: i32,
    pub exams: i32,
    pub discussions: i32,
}

/// Individual Luna activity item (announcement, report, exam, discussion, material).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaActivityRow {
    pub luna_id: String,
    pub activity_type: String,  // "announcement", "report", "exam", "discussion", "material"
    pub title: String,
    pub period: String,         // deadline / date range
    pub status: String,         // e.g. "未提出", "提出済", "未回答", "new"
}

/// KGC course detail fields (授業概要, 成績評価 etc.) extracted from detail page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgcCourseDetailRow {
    pub kgc_code: String,
    pub fields: Vec<(String, String)>,  // (label, value) pairs
    pub delivery_mode: String,          // detected from detail page
}

/// Raw KGC course entry stored in DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgcCourseRow {
    pub id: i64,
    pub kgc_code: String,
    pub name: String,
    pub day: i32,
    pub period: i32,
    pub room: String,
    pub detail_path: String,
    pub is_cancelled: bool,
    pub is_makeup: bool,
    pub is_room_changed: bool,
    pub week_label: String,
}

/// Raw Luna course entry stored in DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseRow {
    pub id: i64,
    pub luna_id: String,
    pub name: String,
    pub teacher: String,
    pub day: i32,
    pub period: i32,
}

/// AI-generated schedule item for a single class session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiScheduleItem {
    #[serde(default)]
    pub day: i32,
    #[serde(default)]
    pub period: i32,
    #[serde(default)]
    pub course_name: String,
    #[serde(default)]
    pub delivery_mode: String,
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub session_topic: String,
    #[serde(default)]
    pub is_cancelled: bool,
    #[serde(default)]
    pub notifications: Vec<String>,
    #[serde(default)]
    pub assignments: Vec<String>,
    #[serde(default)]
    pub exams: Vec<String>,
}

/// Full AI schedule result for two weeks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiScheduleResult {
    #[serde(default)]
    pub current_week_label: String,
    #[serde(default)]
    pub next_week_label: String,
    #[serde(default)]
    pub current_week: Vec<AiScheduleItem>,
    #[serde(default)]
    pub next_week: Vec<AiScheduleItem>,
    #[serde(default)]
    pub weekly_summary: String,
    #[serde(default)]
    pub cross_week_insights: String,
}

/// Raw data collected from both platforms, passed to AI for analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRawData {
    pub kgc_entries_current: Vec<KgcCourseRow>,
    pub kgc_entries_next: Vec<KgcCourseRow>,
    pub luna_courses: Vec<LunaCourseRow>,
    pub session_plans: Vec<(String, Vec<SessionPlanRow>)>,  // (kgc_code, plans)
    pub luna_counts: Vec<(String, LunaCountsRow)>,          // (luna_id, counts)
    pub luna_activities: Vec<LunaActivityRow>,               // detailed activity items
    pub kgc_course_details: Vec<KgcCourseDetailRow>,         // KGC course detail fields
    pub current_week_label: String,
    pub next_week_label: String,
    pub luna_communities: Vec<crate::luna_parser::LunaCommunity>,
}

/// Persisted snapshot metadata: week labels + Luna selector state.
/// Allows rebuilding ScheduleResponse from DB without network.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotState {
    pub current_week_label: String,
    pub next_week_label: String,
    pub luna_year: String,
    pub luna_term: String,
    pub luna_communities: Vec<luna_parser::LunaCommunity>,
    pub luna_year_options: Vec<luna_parser::SelectOption>,
    pub luna_term_options: Vec<luna_parser::SelectOption>,
    pub updated_at: i64,
}

impl Database {
    pub fn open(data_dir: &PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("Failed to create data dir: {}", e))?;
        let db_path = data_dir.join("courses.db");
        let conn = Connection::open(&db_path).map_err(|e| format!("Failed to open DB: {}", e))?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_tables()?;
        Ok(db)
    }

    fn init_tables(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;

        // Migration: always drop everything and recreate from scratch.
        // Data is re-fetched from KGC/Luna on next sync, so no user data is lost.
        let user_version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);

        const CURRENT_VERSION: i32 = 5;
        if user_version != CURRENT_VERSION {
            conn.execute_batch("
                DROP TABLE IF EXISTS session_plans;
                DROP TABLE IF EXISTS luna_counts;
                DROP TABLE IF EXISTS luna_activities;
                DROP TABLE IF EXISTS kgc_courses;
                DROP TABLE IF EXISTS luna_courses;
                DROP TABLE IF EXISTS kgc_course_details;
                DROP TABLE IF EXISTS ai_schedule_cache;
                DROP TABLE IF EXISTS schedule_snapshot_state;
                DROP TABLE IF EXISTS data_cache;
            ").map_err(|e| format!("Migration failed: {}", e))?;
            conn.execute_batch(&format!("PRAGMA user_version = {}", CURRENT_VERSION))
                .map_err(|e| format!("Set version failed: {}", e))?;
        }

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS kgc_courses (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                kgc_code        TEXT NOT NULL,
                name            TEXT NOT NULL,
                day             INTEGER NOT NULL,
                period          INTEGER NOT NULL,
                room            TEXT NOT NULL DEFAULT '',
                detail_path     TEXT NOT NULL DEFAULT '',
                is_cancelled    INTEGER NOT NULL DEFAULT 0,
                is_makeup       INTEGER NOT NULL DEFAULT 0,
                is_room_changed INTEGER NOT NULL DEFAULT 0,
                week_label      TEXT NOT NULL DEFAULT '',
                updated_at      INTEGER NOT NULL DEFAULT 0,
                UNIQUE(kgc_code, day, period, week_label)
            );
            CREATE TABLE IF NOT EXISTS luna_courses (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                luna_id         TEXT NOT NULL,
                name            TEXT NOT NULL,
                teacher         TEXT NOT NULL DEFAULT '',
                day             INTEGER NOT NULL,
                period          INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL DEFAULT 0,
                UNIQUE(luna_id, day, period)
            );
            CREATE TABLE IF NOT EXISTS session_plans (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                kgc_code        TEXT NOT NULL,
                session_num     INTEGER NOT NULL,
                th_header       TEXT NOT NULL DEFAULT '',
                topic           TEXT NOT NULL DEFAULT '',
                delivery_mode   TEXT NOT NULL DEFAULT '',
                study_outside   TEXT NOT NULL DEFAULT '',
                updated_at      INTEGER NOT NULL DEFAULT 0,
                UNIQUE(kgc_code, session_num)
            );
            CREATE TABLE IF NOT EXISTS luna_counts (
                luna_id          TEXT PRIMARY KEY,
                announcements    INTEGER NOT NULL DEFAULT 0,
                new_announcements INTEGER NOT NULL DEFAULT 0,
                reports          INTEGER NOT NULL DEFAULT 0,
                exams            INTEGER NOT NULL DEFAULT 0,
                discussions      INTEGER NOT NULL DEFAULT 0,
                updated_at       INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS luna_activities (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                luna_id         TEXT NOT NULL,
                activity_type   TEXT NOT NULL,
                title           TEXT NOT NULL DEFAULT '',
                period          TEXT NOT NULL DEFAULT '',
                status          TEXT NOT NULL DEFAULT '',
                updated_at      INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS kgc_course_details (
                kgc_code        TEXT PRIMARY KEY,
                fields_json     TEXT NOT NULL DEFAULT '[]',
                delivery_mode   TEXT NOT NULL DEFAULT '',
                updated_at      INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS ai_schedule_cache (
                id              INTEGER PRIMARY KEY CHECK (id = 1),
                result_json     TEXT NOT NULL,
                updated_at      INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS schedule_snapshot_state (
                id                      INTEGER PRIMARY KEY CHECK (id = 1),
                current_week_label      TEXT NOT NULL DEFAULT '',
                next_week_label         TEXT NOT NULL DEFAULT '',
                luna_year               TEXT NOT NULL DEFAULT '',
                luna_term               TEXT NOT NULL DEFAULT '',
                luna_communities_json   TEXT NOT NULL DEFAULT '[]',
                luna_year_options_json   TEXT NOT NULL DEFAULT '[]',
                luna_term_options_json   TEXT NOT NULL DEFAULT '[]',
                updated_at              INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS data_cache (
                cache_key       TEXT PRIMARY KEY,
                data_json       TEXT NOT NULL,
                updated_at      INTEGER NOT NULL DEFAULT 0
            );
        ").map_err(|e| format!("DB init: {}", e))?;
        // Drop old merged tables if they exist (migration from old schema)
        let _ = conn.execute_batch("
            DROP TABLE IF EXISTS courses;
        ");
        Ok(())
    }

    // ── KGC courses ──

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
        let mut stmt = conn.prepare(
            "SELECT id, kgc_code, name, day, period, room, detail_path, is_cancelled, is_makeup, is_room_changed, week_label
             FROM kgc_courses WHERE week_label = ?1 ORDER BY day, period"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map(params![week_label], |row| {
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
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get distinct kgc_codes that need session plan enrichment.
    pub fn kgc_codes_needing_plans(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let threshold = epoch_secs() - 24 * 3600;
        // A course needs plans if:
        //   1. its detail was never fetched (no row in kgc_course_details within 24h), OR
        //   2. it has fewer than 5 session_plans rows (previous parse bug / incomplete data)
        let mut stmt = conn.prepare(
            "SELECT DISTINCT c.kgc_code FROM kgc_courses c
             WHERE c.kgc_code != ''
               AND (
                 c.kgc_code NOT IN (SELECT kgc_code FROM kgc_course_details WHERE updated_at > ?1)
                 OR (SELECT COUNT(*) FROM session_plans sp WHERE sp.kgc_code = c.kgc_code) < 5
               )"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map(params![threshold], |row| {
            row.get::<_, String>(0)
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ── Luna courses ──

    pub fn upsert_luna_course(
        &self,
        luna_id: &str,
        name: &str,
        teacher: &str,
        day: i32,
        period: i32,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        conn.execute(
            "INSERT INTO luna_courses (luna_id, name, teacher, day, period, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(luna_id, day, period) DO UPDATE SET name=?2, teacher=?3, updated_at=?6",
            params![luna_id, name, teacher, day, period, now],
        ).map_err(|e| format!("DB upsert luna: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_luna_courses(&self) -> Result<Vec<LunaCourseRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, luna_id, name, teacher, day, period FROM luna_courses ORDER BY day, period"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map([], |row| {
            Ok(LunaCourseRow {
                id: row.get(0)?,
                luna_id: row.get(1)?,
                name: row.get(2)?,
                teacher: row.get(3)?,
                day: row.get(4)?,
                period: row.get(5)?,
            })
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get luna_ids that need count enrichment (never fetched, or >1h stale).
    pub fn luna_ids_needing_counts(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let threshold = epoch_secs() - 3600;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT luna_id FROM luna_courses
             WHERE luna_id NOT IN (SELECT luna_id FROM luna_counts WHERE updated_at > ?1)"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map(params![threshold], |row| {
            row.get::<_, String>(0)
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ── Session plans ──

    pub fn upsert_session_plans(&self, kgc_code: &str, plans: &[SessionPlanRow]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        for p in plans {
            conn.execute(
                "INSERT INTO session_plans (kgc_code, session_num, th_header, topic, delivery_mode, study_outside, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(kgc_code, session_num) DO UPDATE SET th_header=?3, topic=?4, delivery_mode=?5, study_outside=?6, updated_at=?7",
                params![kgc_code, p.session_num, p.th_header, p.topic, p.delivery_mode, p.study_outside, now],
            ).map_err(|e| format!("DB upsert plan: {}", e))?;
        }
        Ok(())
    }

    pub fn get_all_session_plans(&self) -> Result<Vec<(String, Vec<SessionPlanRow>)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kgc_code, session_num, th_header, topic, delivery_mode, study_outside FROM session_plans ORDER BY kgc_code, session_num"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, SessionPlanRow {
                session_num: row.get(1)?,
                th_header: row.get(2)?,
                topic: row.get(3)?,
                delivery_mode: row.get(4)?,
                study_outside: row.get(5)?,
            }))
        }).map_err(|e| format!("DB map: {}", e))?;
        let mut map: std::collections::HashMap<String, Vec<SessionPlanRow>> = Default::default();
        for r in rows.flatten() {
            map.entry(r.0).or_default().push(r.1);
        }
        Ok(map.into_iter().collect())
    }

    // ── KGC course details ──

    pub fn upsert_kgc_course_detail(&self, detail: &KgcCourseDetailRow) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let fields_json = serde_json::to_string(&detail.fields)
            .map_err(|e| format!("serialize fields: {}", e))?;
        conn.execute(
            "INSERT INTO kgc_course_details (kgc_code, fields_json, delivery_mode, updated_at)
             VALUES (?1,?2,?3,?4)
             ON CONFLICT(kgc_code) DO UPDATE SET fields_json=?2, delivery_mode=?3, updated_at=?4",
            params![detail.kgc_code, fields_json, detail.delivery_mode, now],
        ).map_err(|e| format!("DB upsert detail: {}", e))?;
        Ok(())
    }

    pub fn get_all_kgc_course_details(&self) -> Result<Vec<KgcCourseDetailRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kgc_code, fields_json, delivery_mode FROM kgc_course_details"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map([], |row| {
            let kgc_code: String = row.get(0)?;
            let fields_json: String = row.get(1)?;
            let delivery_mode: String = row.get(2)?;
            let fields: Vec<(String, String)> = serde_json::from_str(&fields_json).unwrap_or_default();
            Ok(KgcCourseDetailRow { kgc_code, fields, delivery_mode })
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

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
        let mut stmt = conn.prepare(
            "SELECT luna_id, announcements, new_announcements, reports, exams, discussions FROM luna_counts"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, LunaCountsRow {
                announcements: row.get(1)?,
                new_announcements: row.get(2)?,
                reports: row.get(3)?,
                exams: row.get(4)?,
                discussions: row.get(5)?,
            }))
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ── Luna activities (detailed items) ──

    /// Replace all activities for a given luna_id with fresh data.
    pub fn replace_luna_activities(&self, luna_id: &str, activities: &[LunaActivityRow]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        conn.execute_batch("BEGIN").map_err(|e| format!("DB begin: {}", e))?;
        let result = (|| -> Result<(), String> {
            conn.execute("DELETE FROM luna_activities WHERE luna_id = ?1", params![luna_id])
                .map_err(|e| format!("DB delete activities: {}", e))?;
            for a in activities {
                conn.execute(
                    "INSERT INTO luna_activities (luna_id, activity_type, title, period, status, updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                    params![luna_id, a.activity_type, a.title, a.period, a.status, now],
                ).map_err(|e| format!("DB insert activity: {}", e))?;
            }
            Ok(())
        })();
        if result.is_ok() {
            conn.execute_batch("COMMIT").map_err(|e| format!("DB commit: {}", e))?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }

    pub fn get_all_luna_activities(&self) -> Result<Vec<LunaActivityRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT luna_id, activity_type, title, period, status FROM luna_activities ORDER BY luna_id, activity_type"
        ).map_err(|e| format!("DB query: {}", e))?;
        let rows = stmt.query_map([], |row| {
            Ok(LunaActivityRow {
                luna_id: row.get(0)?,
                activity_type: row.get(1)?,
                title: row.get(2)?,
                period: row.get(3)?,
                status: row.get(4)?,
            })
        }).map_err(|e| format!("DB map: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ── AI schedule cache ──

    pub fn get_ai_schedule_cache(&self) -> Result<Option<(AiScheduleResult, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let result: Option<(String, i64)> = conn.query_row(
            "SELECT result_json, updated_at FROM ai_schedule_cache WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).ok();
        match result {
            Some((json, ts)) => {
                let parsed: AiScheduleResult = serde_json::from_str(&json)
                    .map_err(|e| format!("AI cache parse: {}", e))?;
                Ok(Some((parsed, ts)))
            }
            None => Ok(None),
        }
    }

    pub fn save_ai_schedule_cache(&self, result: &AiScheduleResult) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let json = serde_json::to_string(result).map_err(|e| format!("AI cache serialize: {}", e))?;
        conn.execute(
            "INSERT INTO ai_schedule_cache (id, result_json, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET result_json=?1, updated_at=?2",
            params![json, now],
        ).map_err(|e| format!("DB save ai cache: {}", e))?;
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

    pub fn get_snapshot_state(&self) -> Result<Option<SnapshotState>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let result: Option<(String, String, String, String, String, String, String, i64)> = conn.query_row(
            "SELECT current_week_label, next_week_label, luna_year, luna_term, luna_communities_json, luna_year_options_json, luna_term_options_json, updated_at FROM schedule_snapshot_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        ).ok();
        match result {
            Some((cwl, nwl, ly, lt, comm_json, yo_json, to_json, updated_at)) => {
                let luna_communities: Vec<luna_parser::LunaCommunity> = serde_json::from_str(&comm_json).unwrap_or_default();
                let luna_year_options: Vec<luna_parser::SelectOption> = serde_json::from_str(&yo_json).unwrap_or_default();
                let luna_term_options: Vec<luna_parser::SelectOption> = serde_json::from_str(&to_json).unwrap_or_default();
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
    pub fn build_raw_data(
        &self,
        current_week_label: &str,
        next_week_label: &str,
        luna_communities: Vec<crate::luna_parser::LunaCommunity>,
    ) -> Result<ScheduleRawData, String> {
        let kgc_current = self.get_kgc_courses(current_week_label)?;
        let kgc_next = self.get_kgc_courses(next_week_label)?;
        let luna_courses = self.get_luna_courses()?;
        let session_plans = self.get_all_session_plans()?;
        let luna_counts = self.get_all_luna_counts()?;
        let luna_activities = self.get_all_luna_activities()?;
        let kgc_course_details = self.get_all_kgc_course_details()?;
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

    // ── Generic data cache ──

    /// Save a JSON blob to the data cache under the given key.
    pub fn save_data_cache(&self, key: &str, json: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        conn.execute(
            "INSERT INTO data_cache (cache_key, data_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(cache_key) DO UPDATE SET data_json=?2, updated_at=?3",
            params![key, json, now],
        ).map_err(|e| format!("DB save cache: {}", e))?;
        Ok(())
    }

    /// Load a cached JSON blob by key. Returns (json, updated_at) if found.
    pub fn get_data_cache(&self, key: &str) -> Result<Option<(String, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let result = conn.query_row(
            "SELECT data_json, updated_at FROM data_cache WHERE cache_key = ?1",
            params![key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        );
        match result {
            Ok(pair) => Ok(Some(pair)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("DB get cache: {}", e)),
        }
    }
}

pub fn epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
