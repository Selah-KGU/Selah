use super::super::*;

pub(in crate::live) fn recording() -> LiveSession {
    let now = Local::now();
    LiveSession {
        session_id: "recording-test".into(),
        course: LiveCourseInfo {
            course_name: "test".into(),
            course_code: String::new(),
            room: String::new(),
            teacher: String::new(),
            day: 1,
            period: 1,
            time_label: String::new(),
            is_free_note: false,
        },
        started_at: now,
        transcript_lines: Arc::new(Vec::new()),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(Vec::new()),
        batch_started_at: now,
        flush_in_flight: false,
        is_fresh_start: true,
        cache_progress: CacheProgress::restored(0, 0),
        finish_phase: None,
        finish_revision: 0,
    }
}

#[test]
fn recording_retains_speech_without_a_frontend_subscriber() {
    let mut session = recording();
    let before = session.snapshot();
    for i in 1..=1000 {
        let update = session.append_line(LiveTranscriptLine {
            at: "10:00:00".into(),
            text: format!("line {i}"),
        });
        assert_eq!(update.line_count, i);
        assert_eq!(update.session_id, "recording-test");
    }
    // A WebView returning after all these lines reads the complete session.
    let recovered = session.snapshot();
    assert_eq!(recovered.session_id.as_deref(), Some("recording-test"));
    assert_eq!(recovered.transcript_lines.len(), 1000);
    assert_eq!(recovered.pending_lines.len(), 1000);
    assert_eq!(recovered.transcript_lines[999].text, "line 1000");
    assert!(before.transcript_lines.is_empty());
}

#[test]
fn transcript_delta_does_not_serialize_the_recording_history() {
    let mut session = recording();
    for _ in 0..1000 {
        session.append_line(LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "speech".into(),
        });
    }
    let mut update = session.append_line(LiveTranscriptLine {
        at: "10:00:00".into(),
        text: "latest".into(),
    });
    let json = serde_json::to_string(&update).unwrap();
    assert!(json.len() < 200);
    assert!(!json.contains("transcript_lines"));
    assert!(!json.contains("speech"));
    assert!(!json.contains("\"seq\""));
    update.seq = Some(42);
    let committed = serde_json::to_value(&update).unwrap();
    assert_eq!(committed["seq"], 42);
    assert_eq!(committed["session_id"], "recording-test");
    assert_eq!(committed["line_count"], 1001);
    assert_eq!(committed["line"]["text"], "latest");
}

#[test]
fn delayed_decoder_cannot_append_to_a_replacement_recording() {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(recording());
    let old_id = state.active_session_id().unwrap();
    let mut next = recording();
    next.session_id = "replacement".into();
    *state.session.lock().unwrap() = Some(next);
    assert!(!state.is_session_current(&old_id));
    let stale = state
        .append_line_for_session(
            Some(&old_id),
            LiveTranscriptLine {
                at: "10:00:00".into(),
                text: "old tail".into(),
            },
        )
        .unwrap();
    assert!(stale.is_none());
    let tail = state
        .append_line_for_session(
            Some("replacement"),
            LiveTranscriptLine {
                at: "10:01:00".into(),
                text: "new tail".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(tail.line_count, 1);
    let guard = state.session.lock().unwrap();
    assert_eq!(guard.as_ref().unwrap().transcript_lines[0].text, "new tail");
    assert_eq!(guard.as_ref().unwrap().pending_lines.len(), 1);
}

#[test]
fn canceled_recording_ignores_delayed_speech_but_manual_append_reports_missing_session() {
    let state = LiveState::new();
    let line = LiveTranscriptLine {
        at: "10:00:00".into(),
        text: "late tail".into(),
    };
    assert!(state
        .append_line_for_session(Some("canceled"), line.clone())
        .unwrap()
        .is_none());
    assert!(state.append_line_for_session(None, line).is_err());
    assert!(state.active_session_id().is_none());
}

#[test]
fn caption_admission_observes_the_owner_while_the_live_lock_is_held() {
    let state = LiveState::new();
    assert_eq!(
        state.with_active_session_id(|owner| {
            assert!(state.session.try_lock().is_err());
            owner.map(str::to_owned)
        }),
        Some(None)
    );
    let mut session = recording();
    session.finish_phase = Some(LiveFinishPhase::Stopping);
    *state.session.lock().unwrap() = Some(session);
    // Draining finals may still display during finish, with unchanged ownership.
    assert_eq!(
        state.with_active_session_id(|owner| {
            assert!(state.session.try_lock().is_err());
            owner.map(str::to_owned)
        }),
        Some(Some("recording-test".to_owned()))
    );
}

#[test]
#[ignore = "manual retained-snapshot append benchmark; no app or microphone"]
fn benchmark_append_after_retained_live_snapshot() {
    use std::hint::black_box;
    use std::time::Instant;
    for (label, count, repetitions) in [
        ("10000 short lines", 10_000, 6),
        ("1000 long lines", 1_000, 256),
    ] {
        let text = "日本語の授業と引用 \"example\" 🌕\n".repeat(repetitions);
        let mut times = Vec::new();
        for _ in 0..7 {
            let mut session = recording();
            for _ in 0..count {
                session.append_line(LiveTranscriptLine {
                    text: text.clone(),
                    at: "10:00:00".into(),
                });
            }
            let retained = session.snapshot();
            let state = LiveState::new();
            *state.session.lock().unwrap() = Some(session);
            let next = LiveTranscriptLine {
                text: "末尾の完全な発話 👩🏽‍💻".into(),
                at: "10:01:00".into(),
            };
            let start = Instant::now();
            let update = black_box(
                state
                    .append_line_for_session(Some("recording-test"), next)
                    .unwrap()
                    .unwrap(),
            );
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(retained.transcript_lines.len(), count);
            assert_eq!(retained.pending_lines.len(), count);
            assert_eq!(retained.transcript_lines[0].text, text);
            assert_eq!(update.line_count, count + 1);
            let current = state.session.lock().unwrap();
            let current = current.as_ref().unwrap();
            assert_eq!(current.transcript_lines[count].text, "末尾の完全な発話 👩🏽‍💻");
            assert_eq!(current.pending_lines.len(), count + 1);
        }
        times.sort_by(f64::total_cmp);
        println!("retained snapshot / {label}: {:.3} ms median / seven trials; {} text bytes in each history; production append + ownership lock only, excludes setup, serialization, disk, model, UI, CPU/RSS and GPU", times[3], count * text.len());
    }
}

#[test]
fn accepted_line_reuses_owned_text_and_is_shared_by_history_pending_and_delta() {
    let text = "第一段落。\nsecond \"quoted\"\t👩🏽‍💻 末尾".repeat(512);
    let at = "23:59:59".to_owned();
    let text_pointer = text.as_ptr();
    let at_pointer = at.as_ptr();
    let mut session = recording();
    let update = session.append_line(LiveTranscriptLine { text, at });
    let stored = &session.transcript_lines[0];
    assert!(Arc::ptr_eq(stored, &session.pending_lines[0]));
    assert!(Arc::ptr_eq(stored, &update.line));
    assert_eq!(
        stored.text.as_ptr(),
        text_pointer,
        "accepted full text was copied"
    );
    assert_eq!(
        stored.at.as_ptr(),
        at_pointer,
        "accepted timestamp was copied"
    );
    let expected = serde_json::json!({"text": "第一段落。\nsecond \"quoted\"\t👩🏽‍💻 末尾".repeat(512), "at": "23:59:59"});
    assert_eq!(serde_json::to_value(&update.line).unwrap(), expected);
    let snapshot = session.snapshot();
    let wire = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        wire["transcript_lines"],
        serde_json::json!([expected.clone()])
    );
    assert_eq!(wire["pending_lines"], serde_json::json!([expected]));
    let recovered: LiveSessionSnapshot = serde_json::from_value(wire).unwrap();
    assert_eq!(recovered.transcript_lines[0].text, stored.text);
    assert_eq!(recovered.pending_lines[0].at, stored.at);
    // Releasing pending summary ownership cannot erase the retained record.
    session.pending_lines = Arc::new(Vec::new());
    drop(update);
    assert_eq!(
        session.transcript_lines[0].text,
        recovered.transcript_lines[0].text
    );
    assert_eq!(snapshot.pending_lines.len(), 1);
}

#[test]
fn appending_during_retained_snapshots_copies_only_the_index_and_preserves_every_line() {
    let mut session = recording();
    for index in 0..128 {
        session.append_line(LiveTranscriptLine {
            text: format!("{index} 日本語\\n\"quoted\" 👩🏽‍💻"),
            at: format!("10:{:02}:00", index % 60),
        });
    }
    let before = session.snapshot();
    let before_wire = serde_json::to_value(&before).unwrap();
    let update = session.append_line(LiveTranscriptLine {
        text: "new complete tail".into(),
        at: "11:00:00".into(),
    });
    assert!(!Arc::ptr_eq(
        &before.transcript_lines,
        &session.transcript_lines
    ));
    assert!(!Arc::ptr_eq(&before.pending_lines, &session.pending_lines));
    for index in 0..128 {
        assert!(Arc::ptr_eq(
            &before.transcript_lines[index],
            &session.transcript_lines[index]
        ));
        assert!(Arc::ptr_eq(
            &before.pending_lines[index],
            &session.pending_lines[index]
        ));
        assert!(Arc::ptr_eq(
            &session.transcript_lines[index],
            &session.pending_lines[index]
        ));
    }
    assert!(Arc::ptr_eq(&update.line, &session.transcript_lines[128]));
    assert_eq!(serde_json::to_value(&before).unwrap(), before_wire);
    let second = session.snapshot();
    session.append_line(LiveTranscriptLine {
        text: "second tail".into(),
        at: "11:00:01".into(),
    });
    assert_eq!(before.transcript_lines.len(), 128);
    assert_eq!(second.transcript_lines.len(), 129);
    assert_eq!(session.transcript_lines.len(), 130);
    assert!(Arc::ptr_eq(
        &before.transcript_lines[0],
        &session.transcript_lines[0]
    ));
    assert!(Arc::ptr_eq(
        &second.transcript_lines[128],
        &session.transcript_lines[128]
    ));
}
