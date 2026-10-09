use super::*;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::native_agent_state::{CapsuleMode, SharedState};
use crate::stt::runtime::{ActiveSttSession, SttInputState};
use crate::stt::{SttSessionControl, SttStreamPhase};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn native_session(id: &str) -> ActiveSttSession {
    ActiveSttSession {
        id: 42,
        caller: "native_agent".into(),
        live_session_id: None,
        input_session_id: Some(id.into()),
        phase: SttStreamPhase::Initializing,
        control: Arc::new(SttSessionControl::default()),
    }
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_state() -> Arc<Mutex<SharedState>> {
    let mut state = SharedState::default();
    state.capture.begin("input".into());
    state.prepare_view(CapsuleMode::Listening, "prompt".into());
    Arc::new(Mutex::new(state))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test(flavor = "current_thread")]
async fn waiting_for_the_registry_releases_native_state_and_revalidates_every_cancel() {
    for cancel in 0..3 {
        let registry = Arc::new(Mutex::new(SttInputState::default()));
        let held = registry.lock().unwrap();
        let state = native_state();
        let worker_state = state.clone();
        let worker_registry = registry.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let mut entered = Some(entered);
        let work = tauri::async_runtime::spawn_blocking(move || {
            with_available_registry(
                &worker_registry,
                |attempt| {
                    let result = worker_state
                        .lock()
                        .unwrap()
                        .reserve_listening_capture("input", attempt);
                    if matches!(result, Ok(None)) {
                        if let Some(entered) = entered.take() {
                            entered.send(()).unwrap();
                        }
                    }
                    result
                },
                |registry| {
                    worker_calls.fetch_add(1, Ordering::SeqCst);
                    registry.reserve(native_session("input"), None)
                },
            )
        });
        waiting.await.unwrap();
        {
            let mut state = state.try_lock().expect("STT wait held the native UI lock");
            match cancel {
                0 => {
                    state.shortcut.release();
                    state.stop_requested = true;
                }
                1 => {
                    state.close_view(None).unwrap();
                }
                _ => {
                    state.capture.begin("new input".into());
                }
            }
        }
        assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        drop(held);
        assert!(work.await.unwrap().is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(registry.lock().unwrap().active.is_none());
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test(flavor = "current_thread")]
async fn retry_reserves_once_under_both_locks_and_retains_exact_control_and_identity() {
    let registry = Arc::new(Mutex::new(SttInputState::default()));
    let held = registry.lock().unwrap();
    let state = native_state();
    let worker_state = state.clone();
    let worker_registry = registry.clone();
    let reserve_registry = registry.clone();
    let reserve_state = state.clone();
    let control = Arc::new(SttSessionControl::default());
    let expected = control.clone();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let mut entered = Some(entered);
    let mut calls = 0;
    let work = tauri::async_runtime::spawn_blocking(move || {
        let result = with_available_registry(
            &worker_registry,
            |attempt| {
                let result = worker_state
                    .lock()
                    .unwrap()
                    .reserve_listening_capture("input", attempt);
                if matches!(result, Ok(None)) {
                    if let Some(entered) = entered.take() {
                        entered.send(()).unwrap();
                    }
                }
                result
            },
            |registry| {
                calls += 1;
                assert!(matches!(
                    reserve_state.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                assert!(matches!(
                    reserve_registry.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                let mut session = native_session("input");
                session.control = control.clone();
                registry.reserve(session, None)?;
                Ok(control)
            },
        );
        assert_eq!(calls, 1);
        result
    });
    waiting.await.unwrap();
    assert!(state.try_lock().is_ok());
    drop(held);
    let actual = work.await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&actual, &expected));
    let registry = registry.lock().unwrap();
    let session = registry.active.as_ref().unwrap();
    assert_eq!(session.id, 42);
    assert_eq!(session.input_session_id.as_deref(), Some("input"));
    assert_eq!(session.phase, SttStreamPhase::Initializing);
    assert!(Arc::ptr_eq(&session.control, &expected));
}

#[test]
fn busy_owner_poisoned_registry_and_reservation_failure_preserve_errors_without_replay() {
    let registry = Mutex::new(SttInputState::default());
    registry
        .lock()
        .unwrap()
        .reserve(native_session("existing"), None)
        .unwrap();
    let mut calls = 0;
    let result = with_available_registry(
        &registry,
        |attempt| attempt(),
        |registry| {
            calls += 1;
            registry.reserve(native_session("new"), None)
        },
    );
    assert_eq!(
        result.unwrap_err(),
        "音声入力は「native_agent」で使用中です"
    );
    assert_eq!(calls, 1);
    assert_eq!(
        registry
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .input_session_id
            .as_deref(),
        Some("existing")
    );
    assert!(std::panic::catch_unwind(|| {
        let _held = registry.lock().unwrap();
        panic!("poison isolated STT fixture");
    })
    .is_err());
    let result = with_available_registry(
        &registry,
        |attempt| attempt(),
        |_| -> Result<(), String> { panic!("poisoned registry admitted input") },
    );
    assert_eq!(result.unwrap_err(), "STT state lock failed");
    let result = with_available_registry(
        &registry,
        |_| Err("owner canceled".into()),
        |_| -> Result<(), String> { panic!("canceled owner admitted input") },
    );
    assert_eq!(result.unwrap_err(), "owner canceled");
}
