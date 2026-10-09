use super::super::*;
use crate::stt::reservation::with_available_registry;
use std::sync::atomic::AtomicUsize;

#[tokio::test(flavor = "current_thread")]
async fn stt_registry_wait_keeps_live_cancel_finish_and_replacement_available() {
    for cancel in 0..3 {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(super::transcript::recording());
        let registry = Arc::new(Mutex::new(None::<String>));
        let held = registry.lock().unwrap();
        let worker_state = state.clone();
        let worker_registry = registry.clone();
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let mut entered = Some(entered);
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let work = tauri::async_runtime::spawn_blocking(move || {
            with_available_registry(
                &worker_registry,
                |attempt| {
                    let result = worker_state.with_microphone_owner("recording-test", attempt);
                    if matches!(result, Ok(None)) {
                        if let Some(entered) = entered.take() {
                            entered.send(()).unwrap();
                        }
                    }
                    result
                },
                |registry| {
                    worker_calls.fetch_add(1, Ordering::SeqCst);
                    *registry = Some("recording-test".into());
                    Ok(())
                },
            )
        });
        waiting.await.unwrap();
        {
            let mut guard = state.session.try_lock().expect("STT wait held LIVE state");
            match cancel {
                0 => *guard = None,
                1 => guard.as_mut().unwrap().finish_phase = Some(LiveFinishPhase::Stopping),
                _ => guard.as_mut().unwrap().session_id = "replacement".into(),
            }
        }
        assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        drop(held);
        let error = work.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            if cancel == 1 {
                "Liveセッションを保存中です"
            } else {
                "Liveセッションが切り替わりました"
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(registry.lock().unwrap().is_none());
    }
}

#[test]
fn available_registry_reservation_keeps_live_owner_locked_until_admission() {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(super::transcript::recording());
    let registry = Mutex::new(None::<String>);
    let revision = state.snapshot_revision.load(Ordering::Relaxed);
    let result = with_available_registry(
        &registry,
        |attempt| state.with_microphone_owner("recording-test", attempt),
        |slot| {
            assert!(matches!(
                state.session.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            assert!(matches!(
                registry.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            *slot = Some("recording-test".into());
            Ok(42)
        },
    );
    assert_eq!(result, Ok(42));
    assert_eq!(registry.lock().unwrap().as_deref(), Some("recording-test"));
    assert_eq!(state.snapshot_revision.load(Ordering::Relaxed), revision);
}
