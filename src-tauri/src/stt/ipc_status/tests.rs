use super::*;
use crate::stt::{SttStreamOwner, SttStreamPhase, SttStreamState};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::IpcResponse;

#[tokio::test(flavor = "current_thread")]
async fn worker_status_preserves_all_phases_ids_and_complete_unicode_owners() {
    let caller = std::thread::current().id();
    for phase in [
        SttStreamPhase::Idle,
        SttStreamPhase::Initializing,
        SttStreamPhase::Listening,
        SttStreamPhase::Stopping,
    ] {
        let state = SttStreamState {
            phase,
            session_id: Some(u64::MAX),
            owner: Some(SttStreamOwner {
                caller: "caller 🌙".repeat(1024),
                live_session_id: Some("完整录音 ID 🌙".repeat(1024)),
                input_session_id: Some("入力 ID 🌙".repeat(1024)),
            }),
        };
        let expected = serde_json::to_string(&state).unwrap();
        let body = reply(move || {
            assert_ne!(std::thread::current().id(), caller);
            Ok(state)
        })
        .await
        .unwrap()
        .body()
        .unwrap();
        assert!(matches!(body,tauri::ipc::InvokeResponseBody::Json(actual) if actual==expected));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn state_lock_waits_do_not_block_async_work_and_poison_errors_are_preserved() {
    let state = Arc::new(Mutex::new(true));
    let held = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let mut guard = held.lock().unwrap();
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        *guard = false;
    });
    started.await.unwrap();
    let read = state.clone();
    let reading = tokio::spawn(reply(move || {
        read.lock()
            .map(|value| *value)
            .map_err(|_| "STT state lock failed".into())
    }));
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let pending = !reading.is_finished();
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(pending);
    assert!(!reading
        .await
        .unwrap()
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<bool>()
        .unwrap());
    let held = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = held.lock().unwrap();
        panic!("fixture STT lock poison");
    })
    .join();
    assert_eq!(
        reply(move || state
            .lock()
            .map(|value| *value)
            .map_err(|_| "STT state lock failed".into()))
        .await
        .err()
        .unwrap(),
        "STT state lock failed"
    );
}
