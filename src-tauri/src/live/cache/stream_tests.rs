use super::*;
use crate::live::{
    LiveSummaryChunk, LiveTermExplanation, LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode,
};
use serde::ser::{Error as _, SerializeStruct};
use serde::Serialize;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("selah-cache-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn no_staging_files(&self) {
        for entry in std::fs::read_dir(&self.0).unwrap() {
            assert!(!entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".selah-write-"));
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[path = "../markdown/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

fn snapshot<'a>(
    lines: &'a [SharedTranscriptLine],
    summaries: &'a [SharedSummaryChunk],
) -> LiveDayCacheRef<'a> {
    LiveDayCacheRef {
        date: "2026-10-08".into(),
        course_name: "講義 中文 👩🏽‍💻",
        started_at: "2026-10-08 10:00:00".into(),
        transcript_lines: lines,
        summaries,
    }
}
fn original_deltas(lines: &[SharedTranscriptLine], start: usize) -> Vec<u8> {
    // The removed batch encoder, independent of the buffered writer.
    let mut bytes = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(start) {
        serde_json::to_writer(
            &mut bytes,
            &LiveLineDeltaRef {
                i,
                t: &line.text,
                a: &line.at,
            },
        )
        .unwrap();
        bytes.push(b'\n');
    }
    bytes
}
fn valid_record(bytes: &[u8]) -> bool {
    serde_json::from_slice::<LiveLineDeltaOwned>(bytes).is_ok()
}

#[test]
fn actual_cache_and_journal_bytes_match_prior_encoders_and_recover_all_summary_fields_and_lines() {
    let dir = Directory::new();
    let path = dir.path("day.cache.json");
    let journal = dir.path("day.ndjson");
    for count in [0, 1, 119, 120, 121, 10_000] {
        let lines = fixtures::lines(count);
        let summaries = fixtures::summaries(3);
        let full = snapshot(&lines, &summaries);
        let expected = serde_json::to_vec(&full).unwrap();
        std::fs::write(
            &journal,
            b"stale log that must be removed only after commit",
        )
        .unwrap();
        persist_day_cache_to(&path, &journal, &full, 0, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert!(!journal.exists());
        let restored: LiveDayCache = serde_json::from_slice(&expected).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&full).unwrap()
        );
        let base = count.min(2);
        let initial = snapshot(&lines[..base], &summaries);
        persist_day_cache_to(&path, &journal, &initial, 0, true).unwrap();
        let original = std::fs::read(&path).unwrap();
        persist_day_cache_to(&path, &journal, &full, base, false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let delta_bytes = original_deltas(&lines, base);
        if delta_bytes.is_empty() {
            assert!(!journal.exists());
        } else {
            assert_eq!(std::fs::read(&journal).unwrap(), delta_bytes);
        }
        let mut recovered: LiveDayCache = serde_json::from_slice(&original).unwrap();
        replay_deltas_into(&mut recovered, std::str::from_utf8(&delta_bytes).unwrap());
        assert_eq!(
            serde_json::to_value(recovered).unwrap(),
            serde_json::to_value(&full).unwrap()
        );
        dir.no_staging_files();
    }
}

#[test]
fn very_long_lines_cross_buffers_and_empty_or_out_of_range_batches_do_not_touch_a_torn_journal() {
    let dir = Directory::new();
    let path = dir.path("cache.json");
    let journal = dir.path("lines.ndjson");
    let mut lines = fixtures::lines(3);
    std::sync::Arc::make_mut(&mut lines[1]).text = "全文 👩🏽‍💻\n\r\0\"引用\"".repeat(20_000);
    let full = snapshot(&lines, &[]);
    persist_day_cache_to(&path, &journal, &snapshot(&lines[..1], &[]), 0, true).unwrap();
    persist_day_cache_to(&path, &journal, &full, 1, false).unwrap();
    assert_eq!(std::fs::read(&journal).unwrap(), original_deltas(&lines, 1));
    let torn = b"{\"i\":99,\"t\":\"partial";
    std::fs::write(&journal, torn).unwrap();
    for start in [3, 4, usize::MAX] {
        persist_day_cache_to(&path, &journal, &full, start, false).unwrap();
        assert_eq!(std::fs::read(&journal).unwrap(), torn);
    }
    std::fs::remove_file(&path).unwrap();
    // Missing base always folds the full snapshot, even for an empty batch.
    persist_day_cache_to(&path, &journal, &full, usize::MAX, false).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec(&full).unwrap()
    );
    assert!(!journal.exists());
}

struct FailingWriter<'a> {
    file: &'a mut File,
    remaining: usize,
    flush_error: bool,
}
impl Write for FailingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected write failure",
            ));
        }
        let amount = self.remaining.min(bytes.len());
        let written = self.file.write(&bytes[..amount])?;
        self.remaining -= written;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.flush_error {
            Err(io::Error::other("injected flush failure"))
        } else {
            self.file.flush()
        }
    }
}

#[test]
fn streaming_write_and_flush_errors_preserve_old_cache_and_roll_back_the_entire_new_journal_batch()
{
    let dir = Directory::new();
    let path = dir.path("cache.json");
    let journal = dir.path("lines.ndjson");
    let lines = fixtures::lines(2_000);
    let full = snapshot(&lines, &[]);
    let previous = b"original complete cache";
    let prior_log = original_deltas(&lines[..1], 0);
    for (remaining, flush_error) in [(1024, false), (usize::MAX, true)] {
        std::fs::write(&path, previous).unwrap();
        std::fs::write(&journal, &prior_log).unwrap();
        let error = atomic_write_with(&path, |file| {
            encoding::cache(
                FailingWriter {
                    file,
                    remaining,
                    flush_error,
                },
                &full,
            )
        })
        .unwrap_err();
        let message = write_error(error, "conversion", "storage");
        assert!(message.starts_with("storage:"));
        assert!(message.contains("injected"));
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        dir.no_staging_files();
        let error = append_ndjson(
            &journal,
            |file| {
                encoding::deltas(
                    FailingWriter {
                        file,
                        remaining,
                        flush_error,
                    },
                    &lines,
                    1,
                )
            },
            valid_record,
        )
        .unwrap_err();
        assert!(write_error(error, "conversion", "storage").starts_with("storage:"));
        assert_eq!(std::fs::read(&journal).unwrap(), prior_log);
        // Retrying a rolled-back batch appends once, including all lines that
        // had reached the file before the injected error.
        append_ndjson(
            &journal,
            |file| encoding::deltas(file, &lines, 1),
            valid_record,
        )
        .unwrap();
        assert_eq!(std::fs::read(&journal).unwrap(), original_deltas(&lines, 0));
    }
}

struct BrokenValue {
    panic: bool,
    body: String,
}
impl Serialize for BrokenValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_struct("BrokenValue", 2)?;
        object.serialize_field("complete_first_field", &self.body)?;
        if self.panic {
            panic!("injected serializer panic");
        }
        Err(S::Error::custom("original serialization error"))
    }
}
#[test]
fn serialization_errors_and_panics_close_and_clean_staging_files_and_roll_back_buffered_appends() {
    let dir = Directory::new();
    let path = dir.path("cache.json");
    let journal = dir.path("lines.ndjson");
    let old = b"complete old file";
    let original = original_deltas(&fixtures::lines(1), 0);
    for panic in [false, true] {
        let bad = BrokenValue {
            panic,
            body: "large already-written field 🌕".repeat(10_000),
        };
        std::fs::write(&path, old).unwrap();
        std::fs::write(&journal, &original).unwrap();
        let result = std::panic::catch_unwind(|| {
            atomic_write_with(&path, |file| encoding::cache(file, &bad))
        });
        if panic {
            assert!(result.is_err());
        } else {
            assert_eq!(
                write_error(result.unwrap().unwrap_err(), "conversion", "storage"),
                "conversion: original serialization error"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), old);
        dir.no_staging_files();
        let result = std::panic::catch_unwind(|| {
            append_ndjson(&journal, |file| encoding::cache(file, &bad), valid_record)
        });
        if panic {
            assert!(result.is_err());
        } else {
            assert!(matches!(result.unwrap(), Err(WriteError::Content(_))));
        }
        assert_eq!(std::fs::read(&journal).unwrap(), original);
    }
}

#[test]
fn failed_cache_replacement_keeps_the_journal_until_commit_and_can_retry_without_losing_rows() {
    let dir = Directory::new();
    let path = dir.path("blocked");
    let journal = dir.path("lines.ndjson");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("old"), b"keep").unwrap();
    let lines = fixtures::lines(3);
    let full = snapshot(&lines, &[]);
    let journal_bytes = original_deltas(&lines, 0);
    std::fs::write(&journal, &journal_bytes).unwrap();
    let error = persist_day_cache_to(&path, &journal, &full, 0, true).unwrap_err();
    assert!(error.starts_with("LIVEキャッシュの保存失敗:"));
    assert_eq!(std::fs::read(&journal).unwrap(), journal_bytes);
    assert_eq!(std::fs::read(path.join("old")).unwrap(), b"keep");
    dir.no_staging_files();
    let missing = dir.path("missing/cache.json");
    assert!(persist_day_cache_to(&missing, &journal, &full, 0, true)
        .unwrap_err()
        .starts_with("LIVEキャッシュの保存失敗:"));
    assert_eq!(std::fs::read(&journal).unwrap(), journal_bytes);
    std::fs::remove_dir_all(&path).unwrap();
    persist_day_cache_to(&path, &journal, &full, 0, true).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec(&full).unwrap()
    );
    assert!(!journal.exists());
}

#[test]
fn torn_and_missing_newline_tails_are_repaired_before_streaming_and_failed_batches_keep_the_repaired_prefix(
) {
    let dir = Directory::new();
    let path = dir.path("lines.ndjson");
    let lines = fixtures::lines(3);
    let complete = original_deltas(&lines[..1], 0);
    for tail in [
        &b"{\"i\":1,\"t\":\"torn"[..],
        &complete[..complete.len() - 1],
    ] {
        let prefix = if tail == &complete[..complete.len() - 1] {
            Vec::new()
        } else {
            complete.clone()
        };
        let mut raw = prefix;
        raw.extend_from_slice(tail);
        std::fs::write(&path, &raw).unwrap();
        let error = append_ndjson(
            &path,
            |file| {
                file.write_all(b"new partial")?;
                Err::<(), _>(io::Error::other("failed batch"))
            },
            valid_record,
        );
        assert!(error.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), complete);
        append_ndjson(
            &path,
            |file| encoding::deltas(file, &lines, 1),
            valid_record,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original_deltas(&lines, 0));
    }
}
