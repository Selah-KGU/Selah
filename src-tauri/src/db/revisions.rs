//! Persistent content versions are separate from cache freshness timestamps.
//! SQLite triggers cover every writer, including older application versions.

use super::*;

pub(super) fn init(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("DB revision migration: {e}"))?;
    let has_revision = {
        let mut columns = tx
            .prepare("PRAGMA table_info(data_cache)")
            .map_err(|e| e.to_string())?;
        let names = columns
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?;
        names
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|name| name == "revision")
    };
    if !has_revision {
        tx.execute_batch("ALTER TABLE data_cache ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;")
            .map_err(|e| format!("DB cache revision column: {e}"))?;
    }
    tx.execute_batch("CREATE TABLE IF NOT EXISTS cache_revision_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        revision INTEGER NOT NULL DEFAULT 0,
        schedule_revision INTEGER NOT NULL DEFAULT 0
    );
    INSERT OR IGNORE INTO cache_revision_state (id) VALUES (1);
    UPDATE data_cache SET revision = rowid WHERE revision = 0;
    UPDATE cache_revision_state SET revision = max(revision, COALESCE((SELECT max(revision) FROM data_cache), 0));
    UPDATE cache_revision_state SET revision = revision + 1, schedule_revision = revision + 1 WHERE schedule_revision = 0;
    CREATE INDEX IF NOT EXISTS idx_data_cache_metadata ON data_cache(cache_key, updated_at, revision);
    CREATE TRIGGER IF NOT EXISTS data_cache_revision_insert AFTER INSERT ON data_cache BEGIN
        UPDATE cache_revision_state SET revision = revision + 1;
        UPDATE data_cache SET revision = (SELECT revision FROM cache_revision_state) WHERE cache_key = NEW.cache_key;
    END;
    CREATE TRIGGER IF NOT EXISTS data_cache_revision_update AFTER UPDATE OF data_json ON data_cache
    WHEN OLD.data_json IS NOT NEW.data_json BEGIN
        UPDATE cache_revision_state SET revision = revision + 1;
        UPDATE data_cache SET revision = (SELECT revision FROM cache_revision_state) WHERE cache_key = NEW.cache_key;
    END;")
    .map_err(|e| format!("DB cache revisions: {e}"))?;

    // Counts, activities and AI results can change without save_snapshot_state.
    // Invalidate the derived schedule whenever any of its SQLite inputs changes.
    for table in [
        "kgc_courses",
        "luna_courses",
        "session_plans",
        "luna_counts",
        "luna_activities",
        "kgc_course_details",
        "ai_schedule_cache",
        "schedule_snapshot_state",
    ] {
        for event in ["INSERT", "UPDATE", "DELETE"] {
            tx.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS schedule_revision_{table}_{event}
                AFTER {event} ON {table} BEGIN
                    UPDATE cache_revision_state SET revision = revision + 1, schedule_revision = revision + 1;
                END;"))
                .map_err(|e| format!("DB schedule revisions: {e}"))?;
        }
    }
    // This raw cache row is also an input to ScheduleResponse.
    for (event, row) in [
        ("INSERT", "NEW"),
        ("UPDATE OF data_json", "NEW"),
        ("DELETE", "OLD"),
    ] {
        let suffix = event.split_whitespace().next().unwrap();
        tx.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS schedule_revision_warning_{suffix}
            AFTER {event} ON data_cache WHEN {row}.cache_key = 'schedule_kgc_warning' BEGIN
                UPDATE cache_revision_state SET revision = revision + 1, schedule_revision = revision + 1;
            END;"))
            .map_err(|e| format!("DB schedule warning revision: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("DB revision migration commit: {e}"))
}

impl Database {
    pub fn cache_revision(&self, key: &str) -> Option<i64> {
        let conn = self.conn.lock().ok()?;
        conn.query_row("SELECT revision FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1", params![key], |row| row.get(0)).ok()
    }

    pub fn schedule_snapshot_version(&self) -> Result<(i64, i64), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        conn.query_row("SELECT COALESCE((SELECT updated_at FROM schedule_snapshot_state WHERE id = 1), 0), schedule_revision FROM cache_revision_state WHERE id = 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("DB schedule version: {e}"))
    }

    /// Expiration is a derived input even when the AI content never changes.
    pub fn schedule_ai_cache_updated_at(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        conn.query_row(
            "SELECT COALESCE((SELECT updated_at FROM ai_schedule_cache WHERE id = 1), 0)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("DB schedule AI timestamp: {e}"))
    }
}
