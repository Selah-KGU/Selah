use super::super::{history::retain_download_records, history_store::HistoryStore};
use super::*;
use std::cell::Cell;
use std::path::{Path, PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("selah-duplicates-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn record(&self, id: &str, content: &[u8], source: &str, timestamp: i64) -> DownloadRecord {
        let path = self.0.join(format!("{id}.md"));
        std::fs::write(&path, content).unwrap();
        DownloadRecord {
            id: id.into(),
            filename: format!("{id}.md"),
            path: path.to_string_lossy().into_owned(),
            course_name: "歴史".into(),
            source: source.into(),
            size_bytes: content.len() as u64,
            downloaded_at: timestamp,
            file_exists: true,
            subfolder: String::new(),
        }
    }
    fn validate(&self, path: &str) -> Result<PathBuf, String> {
        if path == "denied" {
            return Err("fixture validation rejected".into());
        }
        let path = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
        if !path.starts_with(&self.0) {
            return Err("outside fixture".into());
        }
        Ok(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn duplicate_hashes_keep_full_items_and_recommendations_and_sort_by_waste() {
    let f = Fixture::new();
    let a = f.record("a", b"identical file", "mail", 100);
    let b = f.record("b", b"identical file", "luna", 1);
    let c = f.record("c", b"identical file", "live", 200);
    let d = f.record("d", b"xyz", "luna", 1);
    let e = f.record("e", b"xyz", "luna", 2);
    let different = f.record("different", b"different file", "luna", 1);
    let empty = f.record("empty", b"", "luna", 1);
    let groups = find_duplicate_downloads(
        vec![a.clone(), b.clone(), c.clone(), d, e, different, empty],
        |p| f.validate(p),
    )
    .unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].size_bytes, a.size_bytes);
    assert_eq!(
        groups[0].content_hash,
        format!("{:x}", Sha256::digest(b"identical file"))
    );
    assert_eq!(
        groups[0]
            .items
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["c", "a", "b"]
    );
    let json = serde_json::to_value(&groups).unwrap();
    for (i, expected) in [c, a, b].iter().enumerate() {
        assert_eq!(
            json[0]["items"][i],
            serde_json::json!({
                "id": expected.id, "filename": expected.filename, "path": expected.path,
                "course_name": expected.course_name, "source": expected.source,
                "size_bytes": expected.size_bytes, "downloaded_at": expected.downloaded_at,
                "file_exists": true, "is_recommended": expected.id == "b",
            })
        );
    }
    assert!(
        groups[1]
            .items
            .iter()
            .find(|i| i.id == "e")
            .unwrap()
            .is_recommended
    );
}

#[test]
fn batch_deletion_updates_history_once_and_only_removes_successful_paths() {
    let f = Fixture::new();
    let store = HistoryStore::new(f.0.join("history.json"));
    let a = f.record("a", b"first", "luna", 1);
    let b = f.record("b", b"second", "live", 2);
    let keep = f.record("keep", b"keep", "mail", 3);
    let blocked = f.0.join("directory");
    std::fs::create_dir(&blocked).unwrap();
    let mut blocked_row = keep.clone();
    blocked_row.id = "blocked".into();
    blocked_row.path = blocked.to_string_lossy().into_owned();
    store
        .update(|r| {
            r.extend([a.clone(), b.clone(), keep.clone(), blocked_row.clone()]);
            true
        })
        .unwrap();
    let writes = Cell::new(0);
    let result = delete_files_and_history(
        vec![
            " ".into(),
            format!(" {} ", a.path),
            b.path.clone(),
            a.path.clone(),
            blocked_row.path.clone(),
            "denied".into(),
        ],
        |p| f.validate(p),
        |paths| {
            writes.set(writes.get() + 1);
            assert_eq!(paths, &HashSet::from([a.path.clone(), b.path.clone()]));
            // A download completed after the delete request was prepared.
            let completed = f.record("concurrent", b"concurrent", "luna", 4);
            store
                .update(|r| {
                    r.push(completed);
                    true
                })
                .unwrap();
            store
                .update(|r| retain_download_records(r, |row| !paths.contains(&row.path)))
                .map(|_| ())
        },
    )
    .unwrap();
    assert_eq!(writes.get(), 1);
    assert_eq!(result.deleted_count, 2);
    assert_eq!(result.failed_count, 3); // repeated/missing file, directory, rejected path
    assert_eq!(result.errors.len(), 3);
    let rows = store.read().unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["keep", "blocked", "concurrent"]
    );
    assert_eq!(rows[0], keep);
    assert!(Path::new(&rows[0].path).exists());
    assert!(blocked.exists());
    assert!(!Path::new(&a.path).exists());
    assert!(!Path::new(&b.path).exists());
}

#[test]
fn no_successful_deletions_skip_history_and_metadata_errors_keep_accurate_counts() {
    let f = Fixture::new();
    let no_files = delete_files_and_history(
        vec![" ".into(), "denied".into()],
        |p| f.validate(p),
        |_| panic!("nothing to persist"),
    )
    .unwrap();
    assert_eq!(no_files.deleted_count, 0);
    assert_eq!(no_files.failed_count, 1);
    let row = f.record("delete", b"content", "luna", 1);
    let result = delete_files_and_history(
        vec![row.path.clone()],
        |p| f.validate(p),
        |_| Err("corrupt fixture history".into()),
    )
    .unwrap();
    assert_eq!(result.deleted_count, 1);
    assert_eq!(result.failed_count, 0);
    assert_eq!(
        result.errors,
        ["Download history cleanup failed: corrupt fixture history"]
    );
    assert!(!Path::new(&row.path).exists());
    assert_eq!(serde_json::to_value(result).unwrap()["deleted_count"], 1);
}
