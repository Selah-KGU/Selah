use super::*;

impl Database {
    /// Unscoped database for explicit local stores and isolated tests.
    pub fn open(data_dir: &PathBuf) -> Result<Self, String> {
        Ok(Self {
            conn: super::account::AccountConnection::local(data_dir.clone())?,
        })
    }

    /// Application business data is partitioned by university account. Legacy
    /// unowned courses.db remains on disk; it is never assigned to a new login.
    pub fn open_accounts(data_dir: &PathBuf) -> Result<Self, String> {
        Ok(Self {
            conn: super::account::AccountConnection::accounts(data_dir.clone())?,
        })
    }

    pub(super) fn open_connection(data_dir: &std::path::Path) -> Result<Connection, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let conn = Connection::open(data_dir.join("courses.db")).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=33554432;")
            .map_err(|e| e.to_string())?;
        Self::init_connection(&conn)?;
        Ok(conn)
    }

    fn init_connection(conn: &Connection) -> Result<(), String> {
        // Migration: refresh the schedule-derived tables and keep generic
        // data_cache intact. data_cache now contains durable user/app state
        // (SenseA memory, generated todos, preferences) that cannot be safely
        // re-fetched from KGC/Luna.
        let user_version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);

        const CURRENT_VERSION: i32 = 6;
        if user_version != CURRENT_VERSION {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS session_plans;
                DROP TABLE IF EXISTS luna_counts;
                DROP TABLE IF EXISTS luna_activities;
                DROP TABLE IF EXISTS kgc_courses;
                DROP TABLE IF EXISTS luna_courses;
                DROP TABLE IF EXISTS kgc_course_details;
                DROP TABLE IF EXISTS ai_schedule_cache;
                DROP TABLE IF EXISTS schedule_snapshot_state;
            ",
            )
            .map_err(|e| format!("Migration failed: {}", e))?;
            conn.execute_batch(&format!("PRAGMA user_version = {}", CURRENT_VERSION))
                .map_err(|e| format!("Set version failed: {}", e))?;
        }

        conn.execute_batch(
            "
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
                detail_path     TEXT NOT NULL DEFAULT '',
                updated_at      INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS kgc_course_details (
                kgc_code        TEXT PRIMARY KEY,
                fields_json     TEXT NOT NULL DEFAULT '[]',
                delivery_mode   TEXT NOT NULL DEFAULT '',
                textbooks_json  TEXT NOT NULL DEFAULT '[]',
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
            CREATE TABLE IF NOT EXISTS agent_conversations (
                id              TEXT PRIMARY KEY,
                title           TEXT NOT NULL,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_messages (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                conv_id         TEXT NOT NULL REFERENCES agent_conversations(id) ON DELETE CASCADE,
                role            TEXT NOT NULL,
                content         TEXT NOT NULL,
                images_json     TEXT,
                documents_json  TEXT,
                tool_name       TEXT,
                tool_result_json TEXT,
                created_at      INTEGER NOT NULL
            );
        ",
        )
        .map_err(|e| format!("DB init: {}", e))?;

        // Additive migration: keep all cached/user data and the schedule tables.
        let has_documents: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('agent_messages') WHERE name='documents_json')",
            [], |row| row.get(0),
        ).map_err(|e| format!("DB attachment migration: {e}"))?;
        if !has_documents {
            conn.execute(
                "ALTER TABLE agent_messages ADD COLUMN documents_json TEXT",
                [],
            )
            .map_err(|e| format!("DB attachment migration: {e}"))?;
        }
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_agent_message_documents ON agent_messages(conv_id,id) WHERE documents_json IS NOT NULL AND role='user'")
            .map_err(|e| format!("DB attachment index: {e}"))?;
        super::revisions::init(&conn)?;

        // Indexes for frequent queries
        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_kgc_week ON kgc_courses(week_label);
            CREATE INDEX IF NOT EXISTS idx_kgc_code ON kgc_courses(kgc_code);
            CREATE INDEX IF NOT EXISTS idx_luna_id ON luna_courses(luna_id);
            CREATE INDEX IF NOT EXISTS idx_sp_kgc_code ON session_plans(kgc_code);
            CREATE INDEX IF NOT EXISTS idx_la_luna_id ON luna_activities(luna_id);
            CREATE INDEX IF NOT EXISTS idx_lc_updated ON luna_counts(updated_at);
            CREATE INDEX IF NOT EXISTS idx_agent_messages_conv ON agent_messages(conv_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_agent_messages_display ON agent_messages(conv_id, created_at)
                WHERE role IN ('user', 'assistant');
            CREATE INDEX IF NOT EXISTS idx_agent_conv_updated ON agent_conversations(updated_at DESC);
        ").map_err(|e| format!("DB index: {}", e))?;

        // Drop old merged tables if they exist (migration from old schema)
        let _ = conn.execute_batch(
            "
            DROP TABLE IF EXISTS courses;
        ",
        );

        // Migration: add textbooks_json column to existing kgc_course_details tables
        let _ = conn.execute_batch(
            "ALTER TABLE kgc_course_details ADD COLUMN textbooks_json TEXT NOT NULL DEFAULT '[]'",
        );

        // Migration: add detail_path column to existing luna_activities tables
        let _ = conn.execute_batch(
            "ALTER TABLE luna_activities ADD COLUMN detail_path TEXT NOT NULL DEFAULT ''",
        );

        // Force re-fetch for rows that still have empty textbooks (parser was updated)
        let _ = conn.execute_batch(
            "UPDATE kgc_course_details SET updated_at = 0 WHERE textbooks_json = '[]'",
        );

        Ok(())
    }
}
