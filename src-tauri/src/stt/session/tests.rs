use super::super::super::*;
use super::{enqueue_stt_decode_job, SttDecodeInbox, SttDecodeJob};

#[test]
fn graceful_worker_teardown_keeps_every_final_in_shared_and_split_lanes() {
    for split in [false, true] {
        let failed = Arc::new(AtomicBool::new(false));
        let partials = Arc::new(SttDecodeInbox::new(false, failed.clone()));
        let finals = if split {
            Arc::new(SttDecodeInbox::new(false, failed))
        } else {
            partials.clone()
        };
        for seq in 1..=3 {
            finals.push(SttDecodeJob::Final {
                seq,
                samples: vec![seq as f32],
            });
        }
        partials.push(SttDecodeJob::Partial {
            seq: 4,
            version: 1,
            samples: vec![4.0],
        });
        super::super::worker::finish_decode_lanes(&partials, &finals, false);
        for expected in 1..=3 {
            let SttDecodeJob::Final { seq, samples } = finals.pop() else {
                panic!("lost final during graceful shutdown");
            };
            assert_eq!(seq, expected);
            assert_eq!(samples, vec![expected as f32]);
        }
        assert!(matches!(finals.pop(), SttDecodeJob::Shutdown));
        if split {
            assert!(matches!(partials.pop(), SttDecodeJob::Shutdown));
        }
    }
}

#[test]
fn failed_worker_teardown_discards_undecodable_audio_and_wakes_both_lanes() {
    let failed = Arc::new(AtomicBool::new(true));
    let partials = SttDecodeInbox::new(false, failed.clone());
    let finals = SttDecodeInbox::new(false, failed);
    finals.push(SttDecodeJob::Final {
        seq: 1,
        samples: vec![1.0],
    });
    partials.push(SttDecodeJob::Partial {
        seq: 2,
        version: 1,
        samples: vec![2.0],
    });
    super::super::worker::finish_decode_lanes(&partials, &finals, true);
    assert!(matches!(finals.pop(), SttDecodeJob::Shutdown));
    assert!(matches!(partials.pop(), SttDecodeJob::Shutdown));
}

#[test]
fn newer_partial_replaces_queued_partial_without_passing_finals() {
    let mut jobs = VecDeque::new();
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Final {
            seq: 1,
            samples: vec![1.0],
        },
        false,
    );
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Partial {
            version: 1,
            seq: 2,
            samples: vec![2.0],
        },
        false,
    );
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Partial {
            version: 1,
            seq: 3,
            samples: vec![3.0],
        },
        false,
    );
    assert_eq!(jobs.len(), 2);
    match &jobs[0] {
        SttDecodeJob::Final { samples, .. } => assert_eq!(samples, &vec![1.0]),
        other => panic!("expected final, got {:?}", other),
    }
    match &jobs[1] {
        SttDecodeJob::Partial { seq, samples, .. } => {
            assert_eq!(*seq, 3);
            assert_eq!(samples, &vec![3.0]);
        }
        other => panic!("expected partial, got {:?}", other),
    }
}

#[test]
fn live_partial_jumps_ahead_of_queued_finals() {
    let mut jobs = VecDeque::new();
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Final {
            seq: 1,
            samples: vec![1.0],
        },
        true,
    );
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Partial {
            version: 1,
            seq: 2,
            samples: vec![2.0],
        },
        true,
    );
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Final {
            seq: 3,
            samples: vec![3.0],
        },
        true,
    );
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Partial {
            version: 1,
            seq: 4,
            samples: vec![4.0],
        },
        true,
    );
    assert_eq!(jobs.len(), 3);
    match &jobs[0] {
        SttDecodeJob::Partial { seq, samples, .. } => {
            assert_eq!(*seq, 4);
            assert_eq!(samples, &vec![4.0]);
        }
        other => panic!("expected partial, got {:?}", other),
    }
    match &jobs[1] {
        SttDecodeJob::Final { seq, .. } => assert_eq!(*seq, 1),
        other => panic!("expected first final, got {:?}", other),
    }
    match &jobs[2] {
        SttDecodeJob::Final { seq, .. } => assert_eq!(*seq, 3),
        other => panic!("expected second final, got {:?}", other),
    }
}

#[test]
fn live_partial_lane_is_not_blocked_by_queued_finals() {
    let failed = Arc::new(AtomicBool::new(false));
    let partial_inbox = SttDecodeInbox::new(false, Arc::clone(&failed));
    let final_inbox = SttDecodeInbox::new(false, failed);
    final_inbox.push(SttDecodeJob::Final {
        seq: 1,
        samples: vec![1.0],
    });
    partial_inbox.push(SttDecodeJob::Partial {
        version: 1,
        seq: 2,
        samples: vec![2.0],
    });
    final_inbox.push(SttDecodeJob::Final {
        seq: 3,
        samples: vec![3.0],
    });
    partial_inbox.push(SttDecodeJob::Partial {
        version: 1,
        seq: 4,
        samples: vec![4.0],
    });
    assert_eq!(partial_inbox.len(), 1);
    assert_eq!(final_inbox.len(), 2);

    match partial_inbox.pop() {
        SttDecodeJob::Partial { seq, samples, .. } => {
            assert_eq!(seq, 4);
            assert_eq!(samples, vec![4.0]);
        }
        other => panic!("expected partial, got {:?}", other),
    }
    match final_inbox.pop() {
        SttDecodeJob::Final { seq, .. } => assert_eq!(seq, 1),
        other => panic!("expected first final, got {:?}", other),
    }
    match final_inbox.pop() {
        SttDecodeJob::Final { seq, .. } => assert_eq!(seq, 3),
        other => panic!("expected second final, got {:?}", other),
    }
}

#[test]
fn normal_stop_keeps_every_final_and_discards_obsolete_captions() {
    let inbox = SttDecodeInbox::new(false, Arc::new(AtomicBool::new(false)));
    for seq in 1..=3 {
        inbox.push(SttDecodeJob::Final {
            seq,
            samples: vec![seq as f32],
        });
        inbox.push(SttDecodeJob::Partial {
            version: 1,
            seq: seq + 10,
            samples: vec![0.0],
        });
    }
    inbox.discard_partials();
    inbox.push(SttDecodeJob::Shutdown);
    assert_eq!(inbox.len(), 4);
    for expected in 1..=3 {
        match inbox.pop() {
            SttDecodeJob::Final { seq, .. } => assert_eq!(seq, expected),
            other => panic!("lost final {expected}: {other:?}"),
        }
    }
    assert!(matches!(inbox.pop(), SttDecodeJob::Shutdown));
}

#[test]
fn setting_changes_coalesce_and_clear_old_partial_audio() {
    let inbox = SttDecodeInbox::new(false, Arc::new(AtomicBool::new(false)));
    inbox.push(SttDecodeJob::Partial {
        version: 1,
        seq: 1,
        samples: vec![1.0],
    });
    inbox.push(SttDecodeJob::ConfigurePartial);
    inbox.push(SttDecodeJob::Partial {
        version: 1,
        seq: 2,
        samples: vec![2.0],
    });
    inbox.push(SttDecodeJob::ConfigurePartial);
    inbox.push(SttDecodeJob::Partial {
        version: 1,
        seq: 3,
        samples: vec![3.0],
    });
    assert_eq!(inbox.len(), 2);
    assert!(matches!(inbox.pop(), SttDecodeJob::ConfigurePartial));
    assert!(matches!(inbox.pop(), SttDecodeJob::Partial { seq: 3, .. }));
}
