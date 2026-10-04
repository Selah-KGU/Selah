use super::super::super::*;
use super::{enqueue_stt_decode_job, SttDecodeInbox, SttDecodeJob};

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
            seq: 2,
            samples: vec![2.0],
        },
        false,
    );
    enqueue_stt_decode_job(
        &mut jobs,
        SttDecodeJob::Partial {
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
        SttDecodeJob::Partial { seq, samples } => {
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
            seq: 4,
            samples: vec![4.0],
        },
        true,
    );
    assert_eq!(jobs.len(), 3);
    match &jobs[0] {
        SttDecodeJob::Partial { seq, samples } => {
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
        seq: 2,
        samples: vec![2.0],
    });
    final_inbox.push(SttDecodeJob::Final {
        seq: 3,
        samples: vec![3.0],
    });
    partial_inbox.push(SttDecodeJob::Partial {
        seq: 4,
        samples: vec![4.0],
    });
    assert_eq!(partial_inbox.len(), 1);
    assert_eq!(final_inbox.len(), 2);

    match partial_inbox.pop() {
        SttDecodeJob::Partial { seq, samples } => {
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
