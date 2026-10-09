use super::*;

#[derive(Debug, Serialize)]
pub struct CacheDeltaRow {
    pub key: String,
    pub updated_at: i64,
    pub revision: i64,
    pub unchanged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
}

impl Database {
    // ── Generic data cache ──

    /// Save a JSON blob to the data cache under the given key.
    ///
    /// Identical payloads only refresh `updated_at`. Freshness checks still
    /// advance, but the previous blob is not copied into the WAL again.
    pub fn save_data_cache(&self, key: &str, json: &str) -> Result<(), String> {
        self.store_data_cache(key, json, true).map(|_| ())
    }

    /// Like [`save_data_cache`], but returns whether the payload changed and
    /// does not touch `updated_at` when the JSON is unchanged.
    pub fn save_data_cache_if_changed(&self, key: &str, json: &str) -> Result<bool, String> {
        self.store_data_cache(key, json, false)
    }

    /// Returns true when the stored JSON changed.
    pub fn store_data_cache(
        &self,
        key: &str,
        json: &str,
        touch_if_same: bool,
    ) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let now = epoch_secs();
        let existing = conn.query_row(
            "SELECT data_json = ?2 FROM data_cache WHERE cache_key = ?1",
            params![key, json],
            |row| row.get::<_, bool>(0),
        );
        match existing {
            Ok(true) => {
                if touch_if_same {
                    conn.execute(
                        "UPDATE data_cache SET updated_at = ?1 WHERE cache_key = ?2",
                        params![now, key],
                    )
                    .map_err(|e| format!("DB touch cache: {}", e))?;
                }
                Ok(false)
            }
            Ok(false) | Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO data_cache (cache_key, data_json, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(cache_key) DO UPDATE SET data_json=?2, updated_at=?3",
                    params![key, json, now],
                )
                .map_err(|e| format!("DB save cache: {}", e))?;
                Ok(true)
            }
            Err(e) => Err(format!("DB get cache: {}", e)),
        }
    }

    pub fn cache_payload(&self, key: &str) -> Option<String> {
        self.get_data_cache(key)
            .ok()
            .flatten()
            .map(|(json, _)| json)
    }

    /// Timestamp only. Freshness checks must not pull multi-megabyte blobs.
    pub fn cache_updated_at(&self, key: &str) -> Option<i64> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT updated_at FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
    }

    pub fn touch_data_cache(&self, key: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute(
            "UPDATE data_cache SET updated_at = ?1 WHERE cache_key = ?2",
            params![epoch_secs(), key],
        )
        .map_err(|e| format!("DB touch cache: {}", e))?;
        Ok(())
    }

    pub fn cached_source_matches(&self, cache_key: &str, source_hash: &str) -> bool {
        let hash_key = source_hash_key(cache_key);
        self.cache_payload(&hash_key).as_deref() == Some(source_hash)
            && self.cache_exists(cache_key)
    }

    fn cache_exists(&self, key: &str) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return false;
        };
        conn.query_row(
            "SELECT 1 FROM data_cache WHERE cache_key = ?1",
            params![key],
            |_| Ok(()),
        )
        .is_ok()
    }

    pub fn store_source_hash(&self, cache_key: &str, source_hash: &str) {
        let _ = self.save_data_cache_if_changed(&source_hash_key(cache_key), source_hash);
    }

    pub fn checkpoint_passive(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        }
    }

    pub fn oversized_cache_rows(
        &self,
        prefix: &str,
        min_bytes: i64,
    ) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT cache_key, data_json FROM data_cache
                 WHERE cache_key LIKE ?1 AND length(data_json) > ?2",
            )
            .map_err(|e| format!("DB prepare oversized cache: {}", e))?;
        let rows = stmt
            .query_map(params![format!("{prefix}%"), min_bytes], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("DB query oversized cache: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("DB read oversized cache: {}", e))
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

    /// Load all cached JSON blobs whose keys begin with `prefix`.
    pub fn list_data_cache_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String, i64)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT cache_key, data_json, updated_at
                 FROM data_cache
                 WHERE cache_key LIKE ?1
                 ORDER BY cache_key",
            )
            .map_err(|e| format!("DB prepare cache prefix: {}", e))?;
        let pattern = format!("{}%", prefix);
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| format!("DB query cache prefix: {}", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("DB read cache prefix: {}", e))
    }

    /// Load several cache rows under one connection lock.
    pub fn get_data_cache_many(
        &self,
        keys: &[String],
    ) -> Result<std::collections::HashMap<String, (String, i64)>, String> {
        let mut out = std::collections::HashMap::new();
        if keys.is_empty() {
            return Ok(out);
        }
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT data_json, updated_at FROM data_cache WHERE cache_key = ?1")
            .map_err(|e| format!("DB prepare cache batch: {}", e))?;
        for key in keys {
            match stmt.query_row(params![key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }) {
                Ok(pair) => {
                    out.insert(key.clone(), pair);
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(format!("DB get cache batch: {}", e)),
            }
        }
        Ok(out)
    }

    /// Read small covering-index metadata first. Matching versions never load
    /// JSON into Rust; a read transaction keeps metadata and payload consistent.
    pub fn get_data_cache_deltas(
        &self,
        queries: &[(String, Option<i64>)],
    ) -> Result<Vec<CacheDeltaRow>, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("DB cache read: {e}"))?;
        let mut metadata = tx.prepare("SELECT updated_at, revision FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1").map_err(|e| e.to_string())?;
        let mut payload = tx
            .prepare("SELECT data_json FROM data_cache WHERE cache_key = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = Vec::with_capacity(queries.len());
        for (key, known_revision) in queries {
            let stamp = metadata.query_row(params![key], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            });
            let (updated_at, revision, exists) = match stamp {
                Ok((updated_at, revision)) => (updated_at, revision, true),
                Err(rusqlite::Error::QueryReturnedNoRows) => (0, 0, false),
                Err(error) => return Err(format!("DB cache metadata: {error}")),
            };
            let unchanged = *known_revision == Some(revision);
            let json = if exists && !unchanged {
                Some(
                    payload
                        .query_row(params![key], |row| row.get::<_, String>(0))
                        .map_err(|e| format!("DB cache payload: {e}"))?,
                )
            } else {
                None
            };
            rows.push(CacheDeltaRow {
                key: key.clone(),
                updated_at,
                revision,
                unchanged,
                json,
            });
        }
        Ok(rows)
    }

    /// Delete a cached entry by key (used to invalidate stale HTML cache).
    pub fn delete_data_cache(&self, key: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute("DELETE FROM data_cache WHERE cache_key = ?1", params![key])
            .map_err(|e| format!("DB delete cache: {}", e))?;
        Ok(())
    }
}

fn source_hash_key(cache_key: &str) -> String {
    format!("{cache_key}:source_hash")
}

/// Stable fingerprint of one or more fetched payloads.
pub(crate) fn source_hash(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("selah-cache-{}", uuid::Uuid::new_v4()));
        let db = Database::open(&dir).expect("open temp db");
        (db, dir)
    }

    #[test]
    fn identical_payload_does_not_count_as_changed() {
        let (db, dir) = temp_db();
        assert!(db.store_data_cache("k", "hello", true).unwrap());
        assert!(!db.store_data_cache("k", "hello", true).unwrap());
        assert!(!db.save_data_cache_if_changed("k", "hello").unwrap());
        assert!(db.save_data_cache_if_changed("k", "world").unwrap());
        assert_eq!(db.cache_payload("k").as_deref(), Some("world"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn content_versions_survive_touches_same_second_changes_and_recreation() {
        let (db, dir) = temp_db();
        db.save_data_cache("k", "first").unwrap();
        let first = db.cache_revision("k").unwrap();
        db.save_data_cache("k", "first").unwrap();
        db.touch_data_cache("k").unwrap();
        assert_eq!(db.cache_revision("k"), Some(first));
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE data_cache SET updated_at = 42 WHERE cache_key = 'k'",
                [],
            )
            .unwrap();
        db.save_data_cache("k", "second").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE data_cache SET updated_at = 42 WHERE cache_key = 'k'",
                [],
            )
            .unwrap();
        let changed = db
            .get_data_cache_deltas(&[("k".into(), Some(first))])
            .unwrap()
            .remove(0);
        assert_eq!(changed.updated_at, 42);
        assert!(!changed.unchanged);
        assert_eq!(changed.json.as_deref(), Some("second"));
        assert!(changed.revision > first);
        db.delete_data_cache("k").unwrap();
        let absent = db
            .get_data_cache_deltas(&[("k".into(), Some(changed.revision))])
            .unwrap()
            .remove(0);
        assert_eq!(absent.revision, 0);
        assert!(!absent.unchanged);
        db.save_data_cache("k", "third").unwrap();
        assert!(db.cache_revision("k").unwrap() > changed.revision);
        let version = db.cache_revision("k");
        drop(db);
        let reopened = Database::open(&dir).unwrap();
        assert_eq!(reopened.cache_revision("k"), version);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn matching_versions_use_covering_metadata_and_omit_large_json() {
        let (db, dir) = temp_db();
        let large = "x".repeat(2 * 1024 * 1024);
        db.save_data_cache("large", &large).unwrap();
        let revision = db.cache_revision("large").unwrap();
        let rows = db
            .get_data_cache_deltas(&[("large".into(), Some(revision)), ("absent".into(), Some(0))])
            .unwrap();
        assert!(rows.iter().all(|row| row.unchanged && row.json.is_none()));
        assert!(serde_json::to_string(&rows).unwrap().len() < 250);
        let plan: String = db.conn.lock().unwrap().query_row(
            "EXPLAIN QUERY PLAN SELECT updated_at, revision FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1",
            params!["large"], |row| row.get(3)).unwrap();
        assert!(plan.contains("COVERING INDEX"), "{plan}");
        assert_eq!(db.cache_updated_at("large"), Some(rows[0].updated_at));
        let timestamp_plan: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
            "EXPLAIN QUERY PLAN SELECT updated_at FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1",
                params!["large"],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            timestamp_plan.contains("COVERING INDEX"),
            "{timestamp_plan}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schedule_revision_tracks_counts_activities_and_ai_without_timestamp_changes() {
        let (db, dir) = temp_db();
        let initial = db.schedule_snapshot_version().unwrap();
        let statements = [
            "INSERT INTO luna_counts (luna_id, reports) VALUES ('course', 1)",
            "UPDATE luna_counts SET reports = 2 WHERE luna_id = 'course'",
            "INSERT INTO luna_activities (luna_id, activity_type, title) VALUES ('course', 'report', 'new')",
            "INSERT INTO ai_schedule_cache (id, result_json) VALUES (1, '{}')",
            "DELETE FROM luna_counts WHERE luna_id = 'course'",
            "INSERT INTO data_cache (cache_key, data_json) VALUES ('schedule_kgc_warning', 'warning')",
            "UPDATE data_cache SET data_json = 'changed warning' WHERE cache_key = 'schedule_kgc_warning'",
            "DELETE FROM data_cache WHERE cache_key = 'schedule_kgc_warning'",
        ];
        let mut previous = initial.1;
        for statement in statements {
            db.conn.lock().unwrap().execute(statement, []).unwrap();
            let (timestamp, revision) = db.schedule_snapshot_version().unwrap();
            assert_eq!(timestamp, initial.0);
            assert!(revision > previous);
            previous = revision;
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn revision_migration_preserves_legacy_payloads_and_schedule_rows() {
        let dir =
            std::env::temp_dir().join(format!("selah-cache-migration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("courses.db")).unwrap();
        conn.execute_batch("PRAGMA user_version = 6;
            CREATE TABLE data_cache (cache_key TEXT PRIMARY KEY, data_json TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);
            INSERT INTO data_cache VALUES ('user-memory', 'durable', 42);
            CREATE TABLE luna_counts (luna_id TEXT PRIMARY KEY, announcements INTEGER NOT NULL DEFAULT 0, new_announcements INTEGER NOT NULL DEFAULT 0, reports INTEGER NOT NULL DEFAULT 0, exams INTEGER NOT NULL DEFAULT 0, discussions INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0);
            INSERT INTO luna_counts (luna_id, reports) VALUES ('old-course', 3);").unwrap();
        drop(conn);
        let db = Database::open(&dir).unwrap();
        assert_eq!(
            db.get_data_cache("user-memory").unwrap(),
            Some(("durable".into(), 42))
        );
        assert!(db.cache_revision("user-memory").unwrap() > 0);
        let reports: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT reports FROM luna_counts WHERE luna_id = 'old-course'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reports, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[ignore = "manual cache synchronization benchmark"]
    fn benchmark_unchanged_cache_sync() {
        let (db, dir) = temp_db();
        let large = "x".repeat(1024 * 1024);
        let keys: Vec<String> = (0..16).map(|i| format!("payload-{i}")).collect();
        for key in &keys {
            db.save_data_cache(key, &large).unwrap();
        }
        let queries: Vec<_> = keys
            .iter()
            .map(|key| (key.clone(), db.cache_revision(key)))
            .collect();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            assert_eq!(db.get_data_cache_many(&keys).unwrap().len(), 16);
        }
        let baseline = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            assert!(db
                .get_data_cache_deltas(&queries)
                .unwrap()
                .iter()
                .all(|row| row.json.is_none()));
        }
        println!("unchanged cache batch: legacy={baseline:?}, metadata={:?}, 16 MiB per batch, 100 batches", start.elapsed());
        let _ = std::fs::remove_dir_all(dir);
    }
}
