use super::*;
use std::sync::Barrier;

fn partial(seq: u64, text: &str) -> PartialCaption<'_> {
    PartialCaption {
        text: Cow::Borrowed(text),
        caller: "live",
        live_session_id: Some("a"),
        seq,
    }
}

fn committed(seq: Option<u64>, line_count: usize, text: &str) -> CommittedCaption<'_> {
    CommittedCaption {
        session_id: "a",
        line_count,
        seq,
        line: CaptionLine {
            text: Cow::Borrowed(text),
        },
    }
}

fn status(revision: u64, id: Option<&str>) -> LiveSessionStatus {
    LiveSessionStatus {
        update_revision: revision,
        active: id.is_some(),
        session_id: id.map(str::to_owned),
    }
}

#[test]
fn old_and_unowned_captions_do_not_poison_the_current_recordings_order() {
    let mut gate = CaptionGate::default();
    assert!(gate
        .partial(Some("b"), partial(1000, "old"), true, Instant::now())
        .is_none());
    let mut own = partial(1, "current");
    own.live_session_id = Some("b");
    let accepted = gate.partial(Some("b"), own, true, Instant::now()).unwrap();
    assert!(accepted.is_current());
    assert_eq!(accepted.text, "current");
    let mut agent = partial(2000, "agent");
    agent.caller = "agent";
    agent.live_session_id = Some("b");
    assert!(gate
        .partial(Some("b"), agent, true, Instant::now())
        .is_none());
    let missing = PartialCaption {
        live_session_id: None,
        ..partial(3000, "missing owner")
    };
    assert!(gate
        .partial(Some("b"), missing, true, Instant::now())
        .is_none());
    assert!(accepted.is_current());
    assert!(gate
        .partial(None, partial(4000, "ended"), true, Instant::now())
        .is_none());
    assert!(!accepted.is_current());
}

#[test]
fn replacing_the_recording_invalidates_queued_captions_and_resets_order() {
    let mut gate = CaptionGate::default();
    let old = gate
        .partial(Some("a"), partial(500, "old"), true, Instant::now())
        .unwrap();
    assert!(gate.status(Some("b"), &status(2, Some("b"))));
    assert!(!old.is_current());
    let mut next = committed(None, 1, "new manual line");
    next.session_id = "b";
    let new = gate.committed(Some("b"), next, true).unwrap();
    assert_eq!(new.text, "new manual line");
}

#[test]
fn throttled_and_hidden_partials_still_block_delayed_older_finals() {
    let now = Instant::now();
    let mut gate = CaptionGate::default();
    let visible = gate
        .partial(Some("a"), partial(10, "first"), true, now)
        .unwrap();
    assert!(gate
        .partial(
            Some("a"),
            partial(20, "throttled newer"),
            true,
            now + Duration::from_millis(1)
        )
        .is_none());
    assert!(visible.is_current());
    assert!(gate
        .committed(Some("a"), committed(Some(19), 1, "old final"), true)
        .is_none());
    assert!(gate
        .partial(
            Some("a"),
            partial(30, "hidden newer"),
            false,
            now + PARTIAL_INTERVAL
        )
        .is_none());
    assert!(gate
        .committed(Some("a"), committed(Some(29), 2, "old while hidden"), true)
        .is_none());
    let final_caption = gate
        .committed(Some("a"), committed(Some(31), 3, "current final"), true)
        .unwrap();
    assert!(final_caption.is_final);
    assert!(!visible.is_current());
}

#[test]
fn monotonic_partial_throttle_and_final_bypass_keep_caption_progress() {
    let now = Instant::now();
    let mut gate = CaptionGate::default();
    gate.partial(Some("a"), partial(1, "one"), true, now)
        .unwrap();
    assert!(gate
        .partial(
            Some("a"),
            partial(2, "two"),
            true,
            now + Duration::from_millis(119)
        )
        .is_none());
    gate.partial(Some("a"), partial(3, "three"), true, now + PARTIAL_INTERVAL)
        .unwrap();
    let final_caption = gate
        .committed(Some("a"), committed(Some(4), 1, "final"), true)
        .unwrap();
    // The first partial of a new segment follows the final without a delay.
    assert!(gate
        .partial(Some("a"), partial(5, "next"), true, now + PARTIAL_INTERVAL)
        .is_some());
    assert!(!final_caption.is_current());
}

#[test]
fn manual_and_duplicate_final_events_do_not_cover_sequenced_captions() {
    let mut gate = CaptionGate::default();
    gate.committed(Some("a"), committed(None, 1, "manual"), true)
        .unwrap();
    assert!(gate
        .committed(Some("a"), committed(None, 1, "duplicate"), true)
        .is_none());
    let final_caption = gate
        .committed(Some("a"), committed(Some(10), 2, "final"), true)
        .unwrap();
    assert!(gate
        .committed(Some("a"), committed(Some(10), 2, "duplicate final"), true)
        .is_none());
    assert!(gate
        .committed(Some("a"), committed(None, 3, "unordered echo"), true)
        .is_none());
    assert!(final_caption.is_current());
}

#[test]
fn capture_order_outweighs_out_of_order_commit_notifications() {
    let mut gate = CaptionGate::default();
    let older_capture = gate
        .committed(
            Some("a"),
            committed(Some(10), 2, "older capture delivered first"),
            true,
        )
        .unwrap();
    let newer_capture = gate
        .committed(
            Some("a"),
            committed(Some(20), 1, "newer capture delivered late"),
            true,
        )
        .unwrap();
    assert!(!older_capture.is_current());
    assert!(newer_capture.is_current());
    assert_eq!(gate.last_line_count, 2);
}

#[test]
fn late_inactive_status_cannot_schedule_hiding_a_replacement() {
    let mut gate = CaptionGate::default();
    assert!(gate.status(Some("a"), &status(1, Some("a"))));
    assert!(gate.status(Some("b"), &status(3, Some("b"))));
    assert!(!gate.status(Some("b"), &status(2, None)));
    // Even a newer status captured before B starts must match current ownership.
    assert!(!gate.status(Some("b"), &status(4, None)));
    assert!(gate.status(None, &status(5, None)));
    assert!(!gate.status(None, &status(3, Some("b"))));
}

#[test]
fn mailbox_merges_a_thousand_delayed_ui_updates_into_one_latest_value() {
    let mailbox = CaptionMailbox::default();
    let mut gate = CaptionGate::default();
    let mut scheduled = Vec::new();
    for i in 1..=1000 {
        let caption = gate
            .committed(
                Some("a"),
                committed(Some(i), i as usize, &format!("line {i}")),
                true,
            )
            .unwrap();
        if let Some(ticket) = mailbox.push(caption) {
            scheduled.push(ticket);
        }
    }
    assert_eq!(scheduled.len(), 1);
    let last = mailbox.take(scheduled[0]).unwrap();
    assert_eq!(last.text, "line 1000");
    assert!(mailbox.take(scheduled[0]).is_none());
}

#[test]
fn accepted_old_callback_cannot_replace_newer_mailbox_value() {
    let mailbox = CaptionMailbox::default();
    let mut gate = CaptionGate::default();
    let delayed = gate
        .committed(Some("a"), committed(Some(1), 1, "old"), true)
        .unwrap();
    let latest = gate
        .committed(Some("a"), committed(Some(2), 2, "new"), true)
        .unwrap();
    let ticket = mailbox.push(latest).unwrap();
    assert!(mailbox.push(delayed).is_none());
    assert_eq!(mailbox.take(ticket).unwrap().text, "new");
}

#[test]
fn close_and_failed_dispatch_do_not_consume_reopened_or_retried_updates() {
    let mailbox = CaptionMailbox::default();
    let mut gate = CaptionGate::default();
    let old = mailbox
        .push(
            gate.committed(Some("a"), committed(Some(1), 1, "old"), true)
                .unwrap(),
        )
        .unwrap();
    assert!(mailbox.is_scheduled(old));
    mailbox.clear();
    assert!(!mailbox.is_scheduled(old));
    let fresh = mailbox
        .push(
            gate.committed(Some("a"), committed(Some(2), 2, "reopened"), true)
                .unwrap(),
        )
        .unwrap();
    assert!(mailbox.take(old).is_none());
    mailbox.cancel(old);
    assert!(mailbox.is_scheduled(fresh));
    assert_eq!(mailbox.take(fresh).unwrap().text, "reopened");
    let failed = mailbox
        .push(
            gate.committed(Some("a"), committed(Some(3), 3, "failed"), true)
                .unwrap(),
        )
        .unwrap();
    mailbox.cancel(failed);
    let retry = mailbox
        .push(
            gate.committed(Some("a"), committed(Some(4), 4, "retry"), true)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(mailbox.take(retry).unwrap().text, "retry");
}

#[test]
fn concurrent_admission_and_ui_submission_keep_the_newest_capture() {
    let mailbox = Arc::new(CaptionMailbox::default());
    let gate = Arc::new(Mutex::new(CaptionGate::default()));
    let sequence = Arc::new(AtomicU64::new(1));
    let dispatches = Arc::new(Mutex::new(Vec::new()));
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let (mailbox, gate, sequence, dispatches, barrier) = (
                mailbox.clone(),
                gate.clone(),
                sequence.clone(),
                dispatches.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..200 {
                    let seq = sequence.fetch_add(1, Ordering::Relaxed);
                    let caption = gate.lock().unwrap().committed(
                        Some("a"),
                        committed(Some(seq), seq as usize, &format!("line {seq}")),
                        true,
                    );
                    if let Some(caption) = caption {
                        std::thread::yield_now();
                        if let Some(ticket) = mailbox.push(caption) {
                            dispatches.lock().unwrap().push(ticket);
                        }
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let tickets = dispatches.lock().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(mailbox.take(tickets[0]).unwrap().text, "line 1600");
}

#[test]
fn typed_payloads_borrow_plain_text_and_decode_escaped_text_correctly() {
    let plain: PartialCaption<'_> = serde_json::from_str(r#"{"caller":"live","live_session_id":"a","seq":1,"text":"日本語","input_session_id":"ignored"}"#).unwrap();
    assert!(matches!(plain.text, Cow::Borrowed(_)));
    let escaped: PartialCaption<'_> = serde_json::from_str(
        r#"{"caller":"live","live_session_id":"a","seq":2,"text":"日本語\n続き"}"#,
    )
    .unwrap();
    assert_eq!(escaped.text, "日本語\n続き");
    assert!(matches!(escaped.text, Cow::Owned(_)));
    let final_payload: CommittedCaption<'_> = serde_json::from_str(
        r#"{"session_id":"a","seq":3,"line_count":1,"line":{"text":"final","at":"ignored"}}"#,
    )
    .unwrap();
    assert_eq!(final_payload.seq, Some(3));
    assert!(serde_json::from_str::<PartialCaption<'_>>(
        r#"{"text":"missing caller and sequence"}"#
    )
    .is_err());
}
