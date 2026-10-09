use super::*;
use crate::live::{
    LiveCourseInfo, LiveSummaryChunk, LiveTermExplanation, LiveWhiteboard, LiveWhiteboardEdge,
    LiveWhiteboardNode, SharedSummaryChunk,
};
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;

#[path = "before.rs"]
mod before;
#[allow(dead_code)]
#[path = "../../markdown/fixtures.rs"]
mod fixtures;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("selah-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn base(count: usize) -> LiveDayCache {
    LiveDayCache {
        date: "2026-10-08".into(),
        course_name: "全文 中文 👩🏽‍💻".into(),
        started_at: "2026-10-08 10:00:00".into(),
        transcript_lines: fixtures::lines(count),
        summaries: fixtures::summaries(3),
    }
}
fn record(i: usize) -> String {
    serde_json::json!({"i": i, "t": format!("完整 {i} 👩🏽‍💻\n\r\0\"引用\""), "a": "10:00:02"})
        .to_string()
}
fn value(cache: &LiveDayCache) -> serde_json::Value {
    serde_json::to_value(cache).unwrap()
}

#[test]
fn fragmented_reads_match_whole_log_recovery_for_stale_gap_corrupt_crlf_and_final_records() {
    for count in [0, 1, 119, 120, 121] {
        let original = base(count);
        for suffix in ["\n", "\r\n", "", "\r", "\n\u{a0}\u{feff}\n"] {
            for gap in [false, true] {
                let log = format!(
                    "\n\u{2003}\r\nnot-json\n{}\r\n{}\n{{\"i\":\"bad\"}}\n{}\n{}{}",
                    record(count.saturating_sub(1)),
                    record(count),
                    record(count + 1 + usize::from(gap)),
                    record(count + 2),
                    suffix
                );
                let mut expected = original.clone();
                before::replay_reader(&mut expected, Cursor::new(&log)).unwrap();
                for capacity in [1, 2, 3, 7, 8192] {
                    let mut actual = original.clone();
                    replay(
                        &mut actual,
                        BufReader::with_capacity(capacity, Cursor::new(&log)),
                    )
                    .unwrap();
                    assert_eq!(value(&actual), value(&expected));
                    for i in 0..count {
                        assert!(Arc::ptr_eq(
                            &actual.transcript_lines[i],
                            &original.transcript_lines[i]
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn very_long_lines_and_unknown_fields_preserve_full_text_without_changing_gap_rules() {
    let original = base(1);
    let long = "全文 👩🏽‍💻\n\r\0\"引用\"".repeat(20_000);
    let log = format!(
        "{}\n{}\n{{\"i\":2,\"t\":\"bad\",\"a\":null}}\n{}",
        serde_json::json!({"i":1,"t":long,"a":"\r\n\0","unknown":{"keep":"ignored"}}),
        record(1), // duplicate of the just-restored row
        record(2)
    );
    let mut expected = original.clone();
    before::replay_reader(&mut expected, Cursor::new(&log)).unwrap();
    let mut actual = original.clone();
    replay(&mut actual, BufReader::with_capacity(17, Cursor::new(&log))).unwrap();
    assert_eq!(value(&actual), value(&expected));
    assert_eq!(actual.transcript_lines[1].text, long);
    assert_eq!(actual.transcript_lines.len(), 3);
}

struct FaultReader<'a> {
    input: Cursor<&'a [u8]>,
    fail_at: u64,
    panic: bool,
}
impl Read for FaultReader<'_> {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let remaining = self.fail_at.saturating_sub(self.input.position());
        if remaining == 0 {
            if self.panic {
                panic!("injected recovery read panic");
            }
            return Err(io::Error::other("injected recovery read failure"));
        }
        let length = target.len().min(remaining as usize);
        self.input.read(&mut target[..length])
    }
}

#[test]
fn read_utf8_errors_and_panics_roll_back_valid_prefixes_even_after_a_gap() {
    let original = base(1);
    for gap in [false, true] {
        let log = format!(
            "{}\n{}\n{}\n",
            record(1),
            record(if gap { 4 } else { 2 }),
            record(3)
        );
        let mut invalid = log.as_bytes().to_vec();
        invalid.extend_from_slice(b"invalid utf8: \xff\n");
        for capacity in [1, 3, 8192] {
            let mut expected = original.clone();
            assert!(before::replay_reader(&mut expected, Cursor::new(&invalid)).is_err());
            let mut actual = original.clone();
            let error = replay(
                &mut actual,
                BufReader::with_capacity(capacity, Cursor::new(&invalid)),
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(value(&actual), value(&expected));
            assert_eq!(
                actual.transcript_lines.capacity(),
                expected.transcript_lines.capacity()
            );
            for fail_at in [record(1).len() as u64 + 1, log.len() as u64] {
                let reader = || FaultReader {
                    input: Cursor::new(log.as_bytes()),
                    fail_at,
                    panic: false,
                };
                let mut expected = original.clone();
                assert!(before::replay_reader(&mut expected, reader()).is_err());
                let mut actual = original.clone();
                assert!(replay(&mut actual, BufReader::with_capacity(capacity, reader())).is_err());
                assert_eq!(value(&actual), value(&expected));
                assert_eq!(
                    actual.transcript_lines.capacity(),
                    expected.transcript_lines.capacity()
                );
                assert!(Arc::ptr_eq(
                    &actual.transcript_lines[0],
                    &original.transcript_lines[0]
                ));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    replay(
                        &mut actual,
                        BufReader::with_capacity(
                            capacity,
                            FaultReader {
                                input: Cursor::new(log.as_bytes()),
                                fail_at,
                                panic: true,
                            },
                        ),
                    )
                }));
                assert!(result.is_err());
                assert_eq!(value(&actual), value(&original));
                assert_eq!(
                    actual.transcript_lines.capacity(),
                    original.transcript_lines.capacity()
                );
            }
        }
    }
}

#[test]
fn rejecting_a_large_journal_releases_its_rows_and_expanded_index() {
    let original = base(1);
    let mut log = String::new();
    for i in 1..10_000 {
        log.push_str(&record(i));
        log.push('\n');
    }
    let mut invalid = log.into_bytes();
    invalid.push(0xff);
    let mut actual = original.clone();
    assert!(replay(&mut actual, BufReader::new(Cursor::new(invalid))).is_err());
    assert_eq!(value(&actual), value(&original));
    assert_eq!(
        actual.transcript_lines.capacity(),
        original.transcript_lines.capacity()
    );
    assert!(Arc::ptr_eq(
        &actual.transcript_lines[0],
        &original.transcript_lines[0]
    ));
}

#[test]
fn actual_file_loader_matches_original_identity_cleanup_and_corrupt_file_behavior() {
    let dir = Directory::new();
    for (case, json, log, today, course) in [
        (
            "valid",
            serde_json::to_vec(&base(1)).unwrap(),
            format!("{}\n{}", record(1), record(2)).into_bytes(),
            "2026-10-08",
            "全文 中文 👩🏽‍💻",
        ),
        (
            "stale-date",
            serde_json::to_vec(&base(1)).unwrap(),
            b"keep".to_vec(),
            "2026-10-09",
            "全文 中文 👩🏽‍💻",
        ),
        (
            "stale-course",
            serde_json::to_vec(&base(1)).unwrap(),
            b"keep".to_vec(),
            "2026-10-08",
            "different",
        ),
        (
            "bad-json",
            b"{\"date\":\"incomplete".to_vec(),
            b"keep".to_vec(),
            "2026-10-08",
            "全文 中文 👩🏽‍💻",
        ),
        (
            "bad-json-utf8",
            b"{\"date\":\"\xff\"}".to_vec(),
            b"keep".to_vec(),
            "2026-10-08",
            "全文 中文 👩🏽‍💻",
        ),
        (
            "bad-journal-utf8",
            serde_json::to_vec(&base(1)).unwrap(),
            [format!("{}\n", record(1)).into_bytes(), b"\xff".to_vec()].concat(),
            "2026-10-08",
            "全文 中文 👩🏽‍💻",
        ),
        (
            "torn",
            serde_json::to_vec(&base(1)).unwrap(),
            format!("{}\n{{\"i\":2,\"t\":\"torn", record(1)).into_bytes(),
            "2026-10-08",
            "全文 中文 👩🏽‍💻",
        ),
    ] {
        let old_path = dir.path(&format!("{case}-old.json"));
        let old_log = dir.path(&format!("{case}-old.ndjson"));
        let new_path = dir.path(&format!("{case}-new.json"));
        let new_log = dir.path(&format!("{case}-new.ndjson"));
        for (path, journal) in [(&old_path, &old_log), (&new_path, &new_log)] {
            std::fs::write(path, &json).unwrap();
            std::fs::write(journal, &log).unwrap();
        }
        let expected = before::load(&old_path, &old_log, today, course);
        let actual = load(&new_path, &new_log, today, course);
        assert_eq!(
            actual.as_ref().map(value),
            expected.as_ref().map(value),
            "{case}"
        );
        assert_eq!(new_path.exists(), old_path.exists(), "{case}");
        assert_eq!(new_log.exists(), old_log.exists(), "{case}");
        if new_path.exists() {
            assert_eq!(std::fs::read(&new_path).unwrap(), json);
        }
        if new_log.exists() {
            assert_eq!(std::fs::read(&new_log).unwrap(), log);
        }
    }
    let path = dir.path("missing.json");
    let journal = dir.path("missing.ndjson");
    assert!(load(&path, &journal, "2026-10-08", "全文 中文 👩🏽‍💻").is_none());
    std::fs::write(&path, serde_json::to_vec(&base(0)).unwrap()).unwrap();
    assert_eq!(
        value(&load(&path, &journal, "2026-10-08", "全文 中文 👩🏽‍💻").unwrap()),
        value(&base(0))
    );
    std::fs::create_dir(&journal).unwrap(); // journal open/read failure keeps base
    assert_eq!(
        value(&load(&path, &journal, "2026-10-08", "全文 中文 👩🏽‍💻").unwrap()),
        value(&base(0))
    );
}
