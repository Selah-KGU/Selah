use super::*;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CacheTimestampRow {
    pub key: String,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CacheTimestampBatch {
    pub rows: Vec<CacheTimestampRow>,
    pub schedule_updated_at: Option<i64>,
}

impl Database {
    /// One read transaction for refresh indicators. Never select cache JSON or
    /// build the schedule merely to obtain its last successful sync timestamp.
    pub fn cache_timestamps(
        &self,
        keys: &[String],
        include_schedule: bool,
    ) -> Result<CacheTimestampBatch, String> {
        let conn = self.conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("DB timestamp read: {e}"))?;
        let mut metadata = tx.prepare(
            "SELECT updated_at FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1",
        ).map_err(|e| format!("DB timestamp query: {e}"))?;
        let rows = keys
            .iter()
            .map(|key| CacheTimestampRow {
                key: key.clone(),
                // Preserve the single-key timestamp reader's per-row null fallback.
                updated_at: metadata.query_row(params![key], |row| row.get(0)).ok(),
            })
            .collect();
        let schedule_updated_at = if include_schedule {
            tx.query_row(
                "SELECT updated_at FROM schedule_snapshot_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .ok()
        } else {
            None
        };
        Ok(CacheTimestampBatch {
            rows,
            schedule_updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Database, PathBuf) {
        let dir = std::env::temp_dir().join(format!("selah-timestamps-{}", uuid::Uuid::new_v4()));
        (Database::open(&dir).unwrap(), dir)
    }

    #[test]
    fn batch_preserves_requested_keys_missing_zero_negative_and_duplicate_stamps() {
        let (db, dir) = temp_db();
        for (key, stamp) in [("course", 42), ("zero", 0), ("negative", -1), ("曜日'", 77)] {
            db.save_data_cache(key, "payload").unwrap();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE data_cache SET updated_at = ?1 WHERE cache_key = ?2",
                    params![stamp, key],
                )
                .unwrap();
        }
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO schedule_snapshot_state (id, updated_at) VALUES (1, 101)",
                [],
            )
            .unwrap();
        let keys: Vec<String> = ["曜日'", "absent", "course", "zero", "negative", "course"]
            .into_iter()
            .map(String::from)
            .collect();
        let batch = db.cache_timestamps(&keys, true).unwrap();
        assert_eq!(batch.schedule_updated_at, Some(101));
        assert_eq!(
            batch
                .rows
                .iter()
                .map(|row| row.key.clone())
                .collect::<Vec<_>>(),
            keys
        );
        assert_eq!(
            batch
                .rows
                .iter()
                .map(|row| row.updated_at)
                .collect::<Vec<_>>(),
            vec![Some(77), None, Some(42), Some(0), Some(-1), Some(42)]
        );
        for row in &batch.rows {
            assert_eq!(row.updated_at, db.cache_updated_at(&row.key));
        }
        assert_eq!(
            db.cache_timestamps(&[], false).unwrap(),
            CacheTimestampBatch {
                rows: vec![],
                schedule_updated_at: None
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn timestamp_batch_uses_covering_index_without_decoding_large_payload_or_schedule_json() {
        let (db, dir) = temp_db();
        let large = vec![0xffu8; 2 * 1024 * 1024];
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO data_cache (cache_key, data_json, updated_at) VALUES ('large', ?1, 42)", params![large]).unwrap();
            conn.execute("INSERT INTO schedule_snapshot_state (id, luna_communities_json, updated_at) VALUES (1, ?1, 101)", params![large]).unwrap();
            let plan: String = conn.query_row("EXPLAIN QUERY PLAN SELECT updated_at FROM data_cache INDEXED BY idx_data_cache_metadata WHERE cache_key = ?1", params!["large"], |row| row.get(3)).unwrap();
            assert!(plan.contains("COVERING INDEX"), "{plan}");
        }
        assert!(db.get_data_cache("large").is_err());
        assert!(db.get_snapshot_state().is_err());
        let batch = db.cache_timestamps(&["large".into()], true).unwrap();
        assert_eq!(batch.rows[0].updated_at, Some(42));
        assert_eq!(batch.schedule_updated_at, Some(101));
        assert_eq!(
            serde_json::to_string(&batch).unwrap(),
            r#"{"rows":[{"key":"large","updated_at":42}],"schedule_updated_at":101}"#
        );
        let remaining: Vec<u8> = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT data_json FROM data_cache WHERE cache_key = 'large'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, large);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_timestamp_is_local_and_schedule_can_be_skipped_or_missing() {
        let (db, dir) = temp_db();
        db.conn.lock().unwrap().execute_batch("INSERT INTO data_cache (cache_key, data_json, updated_at) VALUES ('broken', '{}', 'bad'), ('valid', '{}', 42);").unwrap();
        let batch = db
            .cache_timestamps(&["broken".into(), "valid".into()], true)
            .unwrap();
        assert_eq!(batch.rows[0].updated_at, None);
        assert_eq!(batch.rows[1].updated_at, Some(42));
        assert_eq!(batch.schedule_updated_at, None);
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE schedule_snapshot_state;")
            .unwrap();
        assert_eq!(
            db.cache_timestamps(&["valid".into()], false).unwrap().rows[0].updated_at,
            Some(42)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
