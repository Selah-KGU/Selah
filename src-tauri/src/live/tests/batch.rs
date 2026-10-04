use super::super::*;
use chrono::TimeZone;

#[test]
fn skip_ai_summarization_for_sessions_under_two_minutes() {
    let now = Local::now();
    assert!(should_skip_ai_summarization(
        now - chrono::Duration::seconds(119),
        now
    ));
    assert!(!should_skip_ai_summarization(
        now - chrono::Duration::seconds(120),
        now
    ));
}

#[test]
fn transcript_line_datetime_uses_session_date() {
    let started_at = Local
        .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
        .single()
        .unwrap();
    let line = LiveTranscriptLine {
        text: "topic".into(),
        at: "10:05:30".into(),
    };

    let parsed = transcript_line_datetime(started_at, &line).unwrap();

    assert_eq!(
        parsed.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-05-13 10:05:30"
    );
}

#[test]
fn transcript_line_datetime_handles_midnight_rollover() {
    let started_at = Local
        .with_ymd_and_hms(2026, 5, 13, 23, 50, 0)
        .single()
        .unwrap();
    let line = LiveTranscriptLine {
        text: "after midnight".into(),
        at: "00:05:00".into(),
    };

    let parsed = transcript_line_datetime(started_at, &line).unwrap();

    assert_eq!(
        parsed.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-05-14 00:05:00"
    );
}

#[test]
fn effective_batch_start_uses_first_pending_line_for_first_chunk() {
    let started_at = Local
        .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
        .single()
        .unwrap();
    let pending = vec![
        LiveTranscriptLine {
            text: "first".into(),
            at: "10:03:00".into(),
        },
        LiveTranscriptLine {
            text: "second".into(),
            at: "10:04:00".into(),
        },
    ];
    let session = LiveSession {
        session_id: "test".into(),
        course: LiveCourseInfo {
            course_name: "テスト".into(),
            course_code: String::new(),
            room: String::new(),
            teacher: String::new(),
            day: 1,
            period: 1,
            time_label: String::new(),
            is_free_note: false,
        },
        started_at,
        transcript_lines: Arc::new(pending.clone()),
        pending_lines: Arc::new(pending),
        summaries: Arc::new(Vec::new()),
        batch_started_at: started_at,
        flush_in_flight: false,
        is_fresh_start: true,
        persisted_line_count: 0,
    };

    let effective = effective_batch_started_at(&session);

    assert_eq!(
        effective.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-05-13 10:03:00"
    );
}

#[test]
fn effective_batch_start_uses_latest_summary_end_after_resume() {
    let started_at = Local
        .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
        .single()
        .unwrap();
    let resumed_batch_started_at = latest_summary_end_datetime(
        started_at,
        &[LiveSummaryChunk {
            title: "Chunk 01 | 10:03-10:10".into(),
            range_label: "10:03-10:10".into(),
            body: "summary".into(),
            line_count: 3,
            terms: Vec::new(),
            whiteboard: None,
        }],
    )
    .unwrap();
    let session = LiveSession {
        session_id: "test".into(),
        course: LiveCourseInfo {
            course_name: "テスト".into(),
            course_code: String::new(),
            room: String::new(),
            teacher: String::new(),
            day: 1,
            period: 1,
            time_label: String::new(),
            is_free_note: false,
        },
        started_at,
        transcript_lines: Arc::new(vec![LiveTranscriptLine {
            text: "covered".into(),
            at: "10:09:30".into(),
        }]),
        pending_lines: Arc::new(vec![LiveTranscriptLine {
            text: "new".into(),
            at: "10:20:00".into(),
        }]),
        summaries: Arc::new(vec![LiveSummaryChunk {
            title: "Chunk 01 | 10:03-10:10".into(),
            range_label: "10:03-10:10".into(),
            body: "summary".into(),
            line_count: 3,
            terms: Vec::new(),
            whiteboard: None,
        }]),
        batch_started_at: resumed_batch_started_at,
        flush_in_flight: false,
        is_fresh_start: false,
        persisted_line_count: 1,
    };

    let effective = effective_batch_started_at(&session);

    assert_eq!(
        effective.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-05-13 10:10:00"
    );
}

#[test]
fn effective_batch_start_keeps_second_precision_after_current_flush() {
    let started_at = Local
        .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
        .single()
        .unwrap();
    let last_subtitle_at = Local
        .with_ymd_and_hms(2026, 5, 13, 10, 10, 45)
        .single()
        .unwrap();
    let session = LiveSession {
        session_id: "test".into(),
        course: LiveCourseInfo {
            course_name: "テスト".into(),
            course_code: String::new(),
            room: String::new(),
            teacher: String::new(),
            day: 1,
            period: 1,
            time_label: String::new(),
            is_free_note: false,
        },
        started_at,
        transcript_lines: Arc::new(Vec::new()),
        pending_lines: Arc::new(vec![LiveTranscriptLine {
            text: "new".into(),
            at: "10:20:00".into(),
        }]),
        summaries: Arc::new(vec![LiveSummaryChunk {
            title: "Chunk 01 | 10:03-10:10".into(),
            range_label: "10:03-10:10".into(),
            body: "summary".into(),
            line_count: 3,
            terms: Vec::new(),
            whiteboard: None,
        }]),
        batch_started_at: last_subtitle_at,
        flush_in_flight: false,
        is_fresh_start: true,
        persisted_line_count: 0,
    };

    let effective = effective_batch_started_at(&session);

    assert_eq!(
        effective.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-05-13 10:10:45"
    );
}
