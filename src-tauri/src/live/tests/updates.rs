use super::*;
use crate::live::tests::transcript::recording;
use crate::live::{LiveSummaryChunk, LiveTranscriptLine, SharedSummaryChunk};

fn chunk(body: &str) -> SharedSummaryChunk {
    LiveSummaryChunk {
        title: "summary".into(),
        range_label: "10:00-10:10".into(),
        body: body.into(),
        line_count: 3,
        terms: Vec::new(),
        whiteboard: None,
    }
    .into()
}

#[test]
fn snapshot_notification_and_speech_append_do_not_wait_for_config_io() {
    use std::sync::mpsc;
    use std::time::Duration;

    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(recording());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        crate::ai::hold_live_timing_io(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        });
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let worker_state = state.clone();
    let worker = std::thread::spawn(move || {
        worker_state
            .append_line_for_session(
                Some("recording-test"),
                LiveTranscriptLine {
                    at: "10:00:00".into(),
                    text: "設定 IO 待機中にも確定字幕を保持する。".into(),
                },
            )
            .unwrap()
            .unwrap();
        let snapshot = current_snapshot(&worker_state);
        let update = current_update(&worker_state, false).unwrap();
        done_tx.send((snapshot, update)).unwrap();
    });
    let result = done_rx.recv_timeout(Duration::from_secs(5));
    // Release and join before asserting: a future regression must fail cleanly
    // rather than leave a blocked thread holding the global configuration gate.
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    worker.join().unwrap();
    let (snapshot, update) = result.expect("LIVE operations waited for config IO");
    assert!(snapshot.active);
    assert_eq!(snapshot.transcript_lines.len(), 1);
    assert_eq!(snapshot.pending_lines.len(), 1);
    assert_eq!(
        snapshot.transcript_lines[0].text,
        "設定 IO 待機中にも確定字幕を保持する。"
    );
    assert!(snapshot.next_summary_at_ms.is_some());
    assert_eq!(snapshot.next_summary_at_ms, update.next_summary_at_ms);
    assert_eq!(update.transcript_line_count, 1);
    assert!(state.session.try_lock().is_ok());
    assert!(state.persistence.gate.try_lock().is_ok());
}

#[test]
fn notification_size_does_not_grow_with_transcript_or_old_summaries() {
    let state = LiveState::new();
    let mut session = recording();
    session.summaries = Arc::new(vec![chunk("previous summary"), chunk("latest summary")]);
    *state.session.lock().unwrap() = Some(session);
    let small = serde_json::to_vec(&current_update(&state, false).unwrap()).unwrap();
    let small_chunk = serde_json::to_vec(&current_update(&state, true).unwrap()).unwrap();
    {
        let mut guard = state.session.lock().unwrap();
        let session = guard.as_mut().unwrap();
        for i in 0..10_000 {
            session.append_line(LiveTranscriptLine {
                text: format!("講義の確定字幕 {i}：すべての文字起こしを保持する。"),
                at: "10:00:00".into(),
            });
        }
        let latest = session.summaries.last().unwrap().clone();
        let mut chunks = vec![chunk(&"earlier board and summary".repeat(100)); 1000];
        chunks.push(latest);
        session.summaries = Arc::new(chunks);
    }
    let metadata = current_update(&state, false).unwrap();
    let updated = current_update(&state, true).unwrap();
    let metadata_json = serde_json::to_value(&metadata).unwrap();
    let chunk_json = serde_json::to_value(&updated).unwrap();
    assert!(metadata_json.get("transcript_lines").is_none());
    assert!(metadata_json.get("pending_lines").is_none());
    assert!(metadata_json.get("summaries").is_none());
    assert!(metadata_json.get("latest_summary").is_none());
    assert_eq!(chunk_json["latest_summary"]["body"], "latest summary");
    assert_eq!(updated.transcript_line_count, 10_000);
    assert_eq!(updated.pending_line_count, 10_000);
    assert_eq!(updated.summary_count, 1001);
    let large = serde_json::to_vec(&metadata).unwrap();
    let large_chunk = serde_json::to_vec(&updated).unwrap();
    // Only the decimal counts/revision grow, not any history contents.
    assert!(large.len() <= small.len() + 20);
    assert!(large_chunk.len() <= small_chunk.len() + 20);
    let full = current_snapshot(&state);
    assert_eq!(full.transcript_lines.len(), 10_000);
    assert_eq!(full.summaries.len(), 1001);
    eprintln!("LIVE notification: {} bytes metadata / {} bytes latest chunk / {} bytes full synthetic history",
        large.len(), large_chunk.len(), serde_json::to_vec(&full).unwrap().len());
}

#[test]
fn full_reads_and_notifications_share_capture_order_across_recordings() {
    let state = LiveState::new();
    let idle = current_snapshot(&state);
    *state.session.lock().unwrap() = Some(recording());
    let first = current_update(&state, false).unwrap();
    let read = current_snapshot(&state.clone());
    let ended = {
        let mut guard = state.session.lock().unwrap();
        let ended = state.capture_completed_snapshot(guard.as_ref().unwrap());
        *guard = None;
        ended
    };
    let inactive = current_update(&state, false).unwrap();
    let mut replacement = recording();
    replacement.session_id = "replacement".into();
    *state.session.lock().unwrap() = Some(replacement);
    let latest = current_update(&state, false).unwrap();
    let revisions = [
        idle.update_revision,
        first.update_revision,
        read.update_revision,
        ended.update_revision,
        inactive.update_revision,
        latest.update_revision,
    ];
    assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(!inactive.active);
    assert!(inactive.session_id.is_none());
    assert_eq!(latest.session_id.as_deref(), Some("replacement"));
    assert_eq!(first.session_id.as_deref(), Some("recording-test"));
}

#[test]
fn notification_does_not_retain_speech_arrays_or_deep_clone_boards_under_lock() {
    let state = LiveState::new();
    let mut session = recording();
    session.summaries = Arc::new(vec![chunk("captured latest")]);
    let speech = Arc::clone(&session.transcript_lines);
    let pending = Arc::clone(&session.pending_lines);
    let chunks = Arc::clone(&session.summaries);
    *state.session.lock().unwrap() = Some(session);
    let metadata = current_update(&state, false).unwrap();
    assert_eq!(Arc::strong_count(&speech), 2);
    assert_eq!(Arc::strong_count(&pending), 2);
    assert_eq!(Arc::strong_count(&chunks), 2);
    let with_chunk = current_update(&state, true).unwrap();
    assert_eq!(Arc::strong_count(&chunks), 2);
    assert!(Arc::ptr_eq(
        with_chunk.latest_summary.as_ref().unwrap(),
        &chunks[0]
    ));
    assert_eq!(Arc::strong_count(&speech), 2);
    assert_eq!(Arc::strong_count(&pending), 2);
    {
        let mut guard = state.session.lock().unwrap();
        Arc::make_mut(&mut guard.as_mut().unwrap().summaries).push(chunk("new latest"));
    }
    // Serialization occurs after releasing the LIVE lock and after replacement.
    assert!(state.session.try_lock().is_ok());
    assert_eq!(
        serde_json::to_value(with_chunk).unwrap()["latest_summary"]["body"],
        "captured latest"
    );
    assert_eq!(metadata.summary_count, 1);
}

#[test]
fn latest_notification_keeps_one_chunk_and_releases_unrelated_history() {
    let state = LiveState::new();
    let mut session = recording();
    let old = chunk("old board and summary");
    let old_weak = Arc::downgrade(&old);
    session.summaries = Arc::new(vec![old, chunk("captured newest")]);
    let history_weak = Arc::downgrade(&session.summaries);
    *state.session.lock().unwrap() = Some(session);
    let update = current_update(&state, true).unwrap();
    assert_eq!(history_weak.strong_count(), 1);
    *state.session.lock().unwrap() = None;
    assert!(history_weak.upgrade().is_none());
    assert!(old_weak.upgrade().is_none());
    assert_eq!(update.summary_count, 2);
    assert_eq!(
        update.latest_summary.as_ref().unwrap().body,
        "captured newest"
    );
    assert_eq!(
        serde_json::to_value(update).unwrap()["latest_summary"]["body"],
        "captured newest"
    );
}

#[test]
fn pending_counts_describe_consumed_prefix_and_new_speech_tail() {
    let state = LiveState::new();
    let mut session = recording();
    for i in 0..5 {
        session.append_line(LiveTranscriptLine {
            text: format!("line {i}"),
            at: "10:00:00".into(),
        });
    }
    Arc::make_mut(&mut session.pending_lines).drain(0..3);
    session.summaries = Arc::new(vec![chunk("first three")]);
    *state.session.lock().unwrap() = Some(session);
    let update = current_update(&state, true).unwrap();
    assert_eq!(update.transcript_line_count - update.pending_line_count, 3);
    assert_eq!(update.pending_line_count, 2);
}

#[test]
fn legacy_full_record_defaults_to_unordered_revision() {
    let mut legacy = serde_json::to_value(recording().snapshot()).unwrap();
    legacy.as_object_mut().unwrap().remove("update_revision");
    let restored: LiveSessionSnapshot = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.update_revision, 0);
}

#[test]
fn native_overlay_parses_only_order_and_lifecycle_from_a_summary_update() {
    let state = LiveState::new();
    let mut session = recording();
    session.summaries = Arc::new(vec![chunk("large unused summary")]);
    *state.session.lock().unwrap() = Some(session);
    let update = current_update(&state, true).unwrap();
    let status: crate::live::LiveSessionStatus =
        serde_json::from_slice(&serde_json::to_vec(&update).unwrap()).unwrap();
    assert!(status.active);
    assert_eq!(status.update_revision, update.update_revision);
    // Missing/malformed lifecycle fields must not be treated as "inactive".
    assert!(serde_json::from_str::<crate::live::LiveSessionStatus>("{}").is_err());
    assert!(serde_json::from_str::<crate::live::LiveSessionStatus>(
        r#"{"active":false,"update_revision":"invalid"}"#
    )
    .is_err());
}
