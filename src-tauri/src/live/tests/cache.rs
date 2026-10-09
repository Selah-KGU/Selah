use super::super::*;
use chrono::TimeZone;

fn fixture_cache(transcript: Vec<(&str, &str)>) -> LiveDayCache {
    LiveDayCache {
        date: "2026-05-13".to_string(),
        course_name: "テスト".to_string(),
        started_at: "2026-05-13 10:00:00".to_string(),
        transcript_lines: transcript
            .into_iter()
            .map(|(text, at)| {
                LiveTranscriptLine {
                    text: text.to_string(),
                    at: at.to_string(),
                }
                .into()
            })
            .collect(),
        summaries: Vec::new(),
    }
}

fn delta_line(i: usize, text: &str, at: &str) -> String {
    serde_json::to_string(&LiveLineDeltaRef { i, t: text, a: at }).unwrap()
}

#[test]
fn replay_appends_new_deltas_in_order() {
    let mut cache = fixture_cache(vec![("hello", "10:00:01")]);
    let deltas = format!(
        "{}\n{}\n",
        delta_line(1, "world", "10:00:02"),
        delta_line(2, "again", "10:00:03"),
    );
    replay_deltas_into(&mut cache, &deltas);
    assert_eq!(cache.transcript_lines.len(), 3);
    assert_eq!(cache.transcript_lines[1].text, "world");
    assert_eq!(cache.transcript_lines[2].at, "10:00:03");
}

#[test]
fn replay_skips_stale_entries_already_in_snapshot() {
    // Snapshot already has 2 lines (e.g. last flush wrote both into cache.json),
    // but deltas still contains those entries because the truncation didn't run.
    let mut cache = fixture_cache(vec![("a", "10:00:01"), ("b", "10:00:02")]);
    let deltas = format!(
        "{}\n{}\n{}\n",
        delta_line(0, "a", "10:00:01"), // stale
        delta_line(1, "b", "10:00:02"), // stale
        delta_line(2, "c", "10:00:03"), // new
    );
    replay_deltas_into(&mut cache, &deltas);
    assert_eq!(cache.transcript_lines.len(), 3);
    assert_eq!(cache.transcript_lines[2].text, "c");
}

#[test]
fn replay_stops_on_gap_to_avoid_reorder() {
    let mut cache = fixture_cache(vec![("a", "10:00:01")]);
    // Missing index 1; should stop before applying index 2.
    let deltas = format!(
        "{}\n{}\n",
        delta_line(2, "c", "10:00:03"),
        delta_line(3, "d", "10:00:04"),
    );
    replay_deltas_into(&mut cache, &deltas);
    assert_eq!(cache.transcript_lines.len(), 1);
}

#[test]
fn replay_tolerates_blank_and_corrupt_lines() {
    let mut cache = fixture_cache(vec![("a", "10:00:01")]);
    let deltas = format!(
        "\n{}\nnot-json\n{}\n",
        delta_line(1, "b", "10:00:02"),
        delta_line(2, "c", "10:00:03"),
    );
    replay_deltas_into(&mut cache, &deltas);
    // The "not-json" between two valid entries is skipped (`continue`), and
    // replay keeps going — `b` at index 1 lands, then `c` at index 2 lands.
    assert_eq!(cache.transcript_lines.len(), 3);
    assert_eq!(cache.transcript_lines[2].text, "c");
}

#[test]
fn replay_noop_on_empty_deltas() {
    let mut cache = fixture_cache(vec![("a", "10:00:01")]);
    replay_deltas_into(&mut cache, "");
    assert_eq!(cache.transcript_lines.len(), 1);
}

#[test]
fn delta_roundtrips_preserve_escapes() {
    // Newlines / quotes in transcript text must survive NDJSON encoding so a
    // single delta entry stays on one line.
    let line = LiveTranscriptLine {
        text: "first\nsecond \"quoted\"".to_string(),
        at: "10:00:01".to_string(),
    };
    let serialized = serde_json::to_string(&LiveLineDeltaRef {
        i: 0,
        t: &line.text,
        a: &line.at,
    })
    .unwrap();
    // Must not contain a raw newline; deltas file splits by '\n'.
    assert!(!serialized.contains('\n'));
    // Roundtrip
    let parsed: LiveLineDeltaOwned = serde_json::from_str(&serialized).unwrap();
    assert_eq!(parsed.t, line.text);
    assert_eq!(parsed.a, line.at);
}

#[test]
fn formal_filename_anchors_to_started_at_date() {
    // started_at on 2026-05-12 23:50; "now" doesn't matter — filename uses
    // the start date so partial mid-session and final on the next calendar
    // day land on the same path.
    let course = LiveCourseInfo {
        course_name: "高等数学".into(),
        course_code: "M101".into(),
        room: "".into(),
        teacher: "".into(),
        day: 1,
        period: 1,
        time_label: "".into(),
        is_free_note: false,
    };
    let dt = Local
        .with_ymd_and_hms(2026, 5, 12, 23, 50, 0)
        .single()
        .unwrap();
    let name = formal_markdown_filename(&course, dt);
    assert!(name.starts_with("20260512_"));
    assert!(name.ends_with("_live.md"));
}

#[test]
fn free_note_formal_filename_uses_started_at_time() {
    let course = LiveCourseInfo {
        course_name: FREE_NOTE_FOLDER_NAME.into(),
        course_code: "".into(),
        room: "".into(),
        teacher: "".into(),
        day: 0,
        period: 0,
        time_label: "".into(),
        is_free_note: true,
    };
    let dt = Local
        .with_ymd_and_hms(2026, 5, 13, 14, 30, 45)
        .single()
        .unwrap();
    let name = formal_markdown_filename(&course, dt);
    assert_eq!(name, "20260513_143045_live.md");
}

#[test]
fn snapshot_serialization_does_not_clone_vec() {
    // The serialized JSON must round-trip back into a LiveDayCache with the
    // original transcript_lines/summaries. This ensures LiveDayCacheRef
    // (the borrow-only serializer) is wire-compatible with LiveDayCache
    // (the owned deserializer).
    let lines = vec![
        LiveTranscriptLine {
            text: "one".into(),
            at: "10:00:01".into(),
        }
        .into(),
        LiveTranscriptLine {
            text: "two".into(),
            at: "10:00:02".into(),
        }
        .into(),
    ];
    let summaries: Vec<SharedSummaryChunk> = vec![];
    let cache_ref = LiveDayCacheRef {
        date: "2026-05-13".into(),
        course_name: "テスト",
        started_at: "2026-05-13 10:00:00".into(),
        transcript_lines: &lines,
        summaries: &summaries,
    };
    let json = serde_json::to_string(&cache_ref).unwrap();
    let parsed: LiveDayCache = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.transcript_lines.len(), 2);
    assert_eq!(parsed.transcript_lines[1].text, "two");
    assert_eq!(parsed.course_name, "テスト");
}
