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

fn refined<'a>(
    translation: Option<&'a str>,
    show: bool,
    ready: bool,
    clear: bool,
) -> RefinedCaption<'a> {
    RefinedCaption {
        session_id: "a",
        segment_id: 1,
        source: String::new(),
        translation: translation.map(str::to_string),
        ready,
        show_overlay: show,
        clear,
    }
}

#[test]
fn translation_hold_blocks_original_captions_without_losing_order() {
    let now = Instant::now();
    let mut gate = CaptionGate::default();
    let original = gate
        .partial(Some("a"), partial(10, "原文"), true, now)
        .unwrap();
    let held = gate
        .refined(
            Some("a"),
            refined(Some("translation"), true, true, false),
            true,
        )
        .unwrap();
    assert!(!held.is_final);
    assert_eq!(held.text, "translation");
    assert!(!original.is_current());
    assert!(gate
        .partial(
            Some("a"),
            partial(11, "next original"),
            true,
            now + PARTIAL_INTERVAL
        )
        .is_none());
    assert!(gate
        .committed(Some("a"), committed(Some(12), 1, "final original"), true)
        .is_none());
    assert!(held.is_current());
    gate.release_hold();
    assert!(!held.is_current());
    let resumed = gate
        .partial(
            Some("a"),
            partial(13, "after release"),
            true,
            now + PARTIAL_INTERVAL + PARTIAL_INTERVAL,
        )
        .unwrap();
    assert_eq!(resumed.text, "after release");
    assert!(gate
        .committed(Some("a"), committed(Some(12), 2, "late final"), true)
        .is_none());
}

#[test]
fn only_a_ready_overlay_translation_holds_the_caption() {
    let mut gate = CaptionGate::default();
    assert!(gate
        .refined(Some("a"), refined(Some("訳"), false, true, false), true)
        .is_none());
    assert!(gate
        .refined(Some("a"), refined(Some("訳"), true, false, false), true)
        .is_none());
    assert!(gate
        .refined(Some("a"), refined(Some("  "), true, true, false), true)
        .is_none());
    assert!(gate
        .refined(Some("a"), refined(None, true, true, false), true)
        .is_none());
    let held = gate
        .refined(Some("a"), refined(Some("訳"), true, true, false), true)
        .unwrap();
    assert!(!held.is_final);
    gate.refined(Some("a"), refined(None, false, false, true), true);
    assert!(!held.is_current());
}

#[test]
fn switching_recordings_releases_a_translation_hold() {
    let mut gate = CaptionGate::default();
    let held = gate
        .refined(Some("a"), refined(Some("訳"), true, true, false), true)
        .unwrap();
    assert!(gate.status(Some("b"), &status(2, Some("b"))));
    assert!(!held.is_current());
    let mut next = partial(1, "new");
    next.live_session_id = Some("b");
    assert_eq!(
        gate.partial(Some("b"), next, true, Instant::now())
            .unwrap()
            .text,
        "new"
    );
}

#[test]
fn reopening_the_overlay_accepts_new_speech_and_invalidates_old_pages() {
    let mut gate = CaptionGate::default();
    let held = gate
        .refined(
            Some("a"),
            refined(Some("old translation"), true, true, false),
            true,
        )
        .unwrap();
    gate.partial(Some("a"), partial(10, "before close"), true, Instant::now());
    gate.surface_closed();
    assert!(!held.is_current());
    let first = gate
        .partial(
            Some("a"),
            partial(20, "after reopening"),
            true,
            Instant::now(),
        )
        .unwrap();
    assert_eq!(first.text, "after reopening");
    assert!(gate
        .committed(Some("a"), committed(Some(10), 1, "old final"), true)
        .is_none());
    assert!(first.is_current());
    let final_caption = gate
        .committed(Some("a"), committed(Some(30), 2, "new final"), true)
        .unwrap();
    assert_eq!(final_caption.text, "new final");
}

#[test]
fn closing_also_cancels_a_queued_raw_caption_and_resets_its_throttle() {
    let mut gate = CaptionGate::default();
    let now = Instant::now();
    let old = gate
        .partial(Some("a"), partial(1, "old"), true, now)
        .unwrap();
    gate.surface_closed();
    assert!(!old.is_current());
    assert!(gate
        .partial(Some("a"), partial(2, "new"), true, now)
        .is_some());
}

#[test]
fn refined_event_defaults_keep_a_clear_payload_small() {
    let payload: RefinedCaption<'_> = serde_json::from_str(
        r#"{"session_id":"a","ready":true,"show_overlay":true,"translation":"訳"}"#,
    )
    .unwrap();
    assert!(!payload.clear);
    assert_eq!(payload.translation.as_deref(), Some("訳"));
    assert_eq!(payload.segment_id, 0);
}

#[test]
fn escaped_translations_reach_the_native_caption() {
    for text in ["say \"hello\"", r"C:\data", "日本語\n続き", "🌙"] {
        let wire = serde_json::json!({
            "session_id": "a", "segment_id": 3, "ready": true,
            "show_overlay": true, "translation": text,
        })
        .to_string();
        let payload: RefinedCaption<'_> = serde_json::from_str(&wire).unwrap();
        let mut gate = CaptionGate::default();
        assert_eq!(gate.refined(Some("a"), payload, true).unwrap().text, text);
    }
}

#[test]
fn a_failed_translation_immediately_restores_the_latest_original_and_releases_hold() {
    let now = Instant::now();
    let mut gate = CaptionGate::default();
    let held = gate
        .refined(
            Some("a"),
            refined(Some("old translation"), true, true, false),
            true,
        )
        .unwrap();
    assert!(gate
        .partial(Some("a"), partial(11, "new speech"), true, now)
        .is_none());
    assert!(gate
        .committed(Some("a"), committed(Some(12), 1, "new final"), true)
        .is_none());
    let fallback = RefinedCaption {
        segment_id: 2,
        source: "earlier batch source".into(),
        ..refined(None, false, false, false)
    };
    let restored = gate.refined(Some("a"), fallback, true).unwrap();
    assert!(!held.is_current());
    assert_eq!(restored.text, "new final");
    assert!(restored.is_final);
    assert_eq!(
        gate.partial(Some("a"), partial(13, "continuing speech"), true, now)
            .unwrap()
            .text,
        "continuing speech"
    );
    assert!(gate
        .committed(Some("a"), committed(Some(12), 2, "late final"), true)
        .is_none());
}

#[test]
fn an_untranslated_ready_caption_uses_its_source_when_no_original_is_waiting() {
    let mut gate = CaptionGate::default();
    gate.refined(
        Some("a"),
        refined(Some("old translation"), true, true, false),
        true,
    );
    let same_language = RefinedCaption {
        segment_id: 2,
        source: "current source".into(),
        ..refined(None, false, true, false)
    };
    let restored = gate.refined(Some("a"), same_language, true).unwrap();
    assert_eq!(restored.text, "current source");
    assert!(restored.is_final);
}

#[test]
fn a_stale_or_unowned_failure_cannot_release_a_newer_translation() {
    let mut gate = CaptionGate::default();
    let held = gate
        .refined(
            Some("a"),
            RefinedCaption {
                segment_id: 10,
                ..refined(Some("new translation"), true, true, false)
            },
            true,
        )
        .unwrap();
    assert!(gate
        .refined(
            Some("a"),
            RefinedCaption {
                segment_id: 9,
                ..refined(None, false, false, false)
            },
            true
        )
        .is_none());
    assert!(gate
        .refined(
            Some("a"),
            RefinedCaption {
                session_id: "old recording",
                segment_id: 20,
                ..refined(None, false, false, true)
            },
            true
        )
        .is_none());
    assert!(held.is_current());
}

#[test]
fn disabling_translation_restores_a_waiting_original_or_clears_the_label() {
    for original in [None, Some("waiting original")] {
        let mut gate = CaptionGate::default();
        gate.refined(
            Some("a"),
            refined(Some("old translation"), true, true, false),
            true,
        );
        if let Some(text) = original {
            gate.partial(Some("a"), partial(1, text), true, Instant::now());
        }
        let cleared = gate
            .refined(Some("a"), refined(None, false, false, true), true)
            .unwrap();
        assert_eq!(cleared.text, original.unwrap_or(""));
        assert!(!gate.holding_translation);
    }
}
