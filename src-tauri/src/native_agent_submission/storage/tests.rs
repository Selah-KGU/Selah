use super::*;

#[test]
fn failed_speech_is_retained_and_only_one_retry_can_own_it() {
    let storage = Arc::new(VoiceStorage::default());
    let attempt = storage.accept("voice".into(), "complete final lines and tail".into());
    assert!(
        storage.retry_failed().is_empty(),
        "retried an in-flight write"
    );
    let result: Result<(), _> = attempt.persist_with(|input, retry| {
        assert!(!retry);
        assert_eq!(input.text, "complete final lines and tail");
        Err("disk full".into())
    });
    assert!(result.is_err());
    let mut retries = storage.retry_failed();
    assert_eq!(retries.len(), 1);
    assert!(storage.retry_failed().is_empty());
    retries
        .pop()
        .unwrap()
        .persist_with(|input, retry| {
            assert!(retry);
            assert_eq!(input.conversation_id, "voice");
            assert_eq!(input.text, "complete final lines and tail");
            Ok(())
        })
        .unwrap();
    assert!(storage.retry_failed().is_empty());
    assert!(storage.0.lock().unwrap().is_empty());
}

#[test]
fn unwinding_a_writer_retains_speech_for_a_future_retry() {
    let storage = Arc::new(VoiceStorage::default());
    let attempt = storage.accept("voice".into(), "accepted speech".into());
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        attempt.persist_with::<()>(|_, _| panic!("simulated storage worker panic"))
    }));
    assert!(panic.is_err());
    let mut retries = storage.retry_failed();
    assert_eq!(retries.len(), 1);
    retries
        .pop()
        .unwrap()
        .persist_with(|input, retry| {
            assert!(retry);
            assert_eq!(input.text, "accepted speech");
            Ok(())
        })
        .unwrap();
    assert!(storage.0.lock().unwrap().is_empty());
}

#[test]
fn concurrent_retry_requests_do_not_duplicate_accepted_voice() {
    let storage = Arc::new(VoiceStorage::default());
    drop(storage.accept("voice".into(), "accepted speech".into()));
    let start = Arc::new(std::sync::Barrier::new(9));
    let owners: Vec<_> = (0..8)
        .map(|_| {
            let storage = storage.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                storage.retry_failed()
            })
        })
        .collect();
    start.wait();
    let attempts: Vec<_> = owners
        .into_iter()
        .flat_map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(attempts.len(), 1);
    for attempt in attempts {
        attempt.persist_with(|_, _| Ok(())).unwrap();
    }
    assert!(storage.0.lock().unwrap().is_empty());
}
