use super::*;

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
            "SELECT data_json FROM data_cache WHERE cache_key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        );
        match existing {
            Ok(prev) if prev == json => {
                if touch_if_same {
                    conn.execute(
                        "UPDATE data_cache SET updated_at = ?1 WHERE cache_key = ?2",
                        params![now, key],
                    )
                    .map_err(|e| format!("DB touch cache: {}", e))?;
                }
                Ok(false)
            }
            Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => {
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
            "SELECT updated_at FROM data_cache WHERE cache_key = ?1",
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
        let dir = std::env::temp_dir().join(format!(
            "selah-cache-{}-{}",
            std::process::id(),
            epoch_secs()
        ));
        let _ = std::fs::remove_dir_all(&dir);
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
}
