use super::*;
use crate::latest_ui_mailbox::CurrentUiValue;
use crate::stt::{SttStreamOwner, SttStreamPhase};
use std::sync::Arc;

fn idle() -> Result<SttStreamState, String> {
    Ok(SttStreamState {
        phase: SttStreamPhase::Idle,
        session_id: None,
        owner: None,
    })
}
fn occupied(caller: &str) -> Result<SttStreamState, String> {
    Ok(SttStreamState {
        phase: SttStreamPhase::Stopping,
        session_id: Some(3),
        owner: Some(SttStreamOwner {
            caller: caller.into(),
            live_session_id: None,
            input_session_id: Some("old input".into()),
        }),
    })
}

#[test]
fn repeated_press_and_close_while_held_cannot_arm_another_capture() {
    let mut state = SharedState::default();
    let old = state.press_shortcut().unwrap();
    assert!(state.press_shortcut().is_none());
    state.close_view(None).unwrap();
    assert!(state.press_shortcut().is_none());
    assert!(!state.owns_shortcut(old));
    state.shortcut.release();
    let new = state.press_shortcut().unwrap();
    assert!(state.owns_shortcut(new));
    assert!(!state.owns_shortcut(old));
}

#[test]
fn canceled_delays_never_read_stt_or_clear_the_existing_response() {
    let state = Mutex::new(SharedState::default());
    let request = {
        let mut state = state.lock().unwrap();
        state.begin_stream("existing".into());
        let request = state.press_shortcut().unwrap();
        state.shortcut.release();
        request
    };
    assert!(
        prepare_capture(&state, request, "canceled".into(), "prompt", || panic!(
            "canceled hold read STT"
        ))
        .is_none()
    );
    assert!(state.lock().unwrap().owns_stream("existing"));
}

#[test]
fn release_close_replacement_and_new_view_retire_a_status_read_in_flight() {
    for action in 0..4 {
        for failure in [false, true] {
            let state = Mutex::new(SharedState::default());
            let request = state.lock().unwrap().press_shortcut().unwrap();
            let update = prepare_capture(&state, request, "old".into(), "prompt", || {
                let mut state = state.lock().unwrap(); // STT read must not hold this lock.
                match action {
                    0 => state.shortcut.release(),
                    1 => {
                        state.close_view(None).unwrap();
                    }
                    2 => {
                        state.shortcut.release();
                        state.press_shortcut().unwrap();
                        state.capture.begin("replacement".into());
                        state.prepare_view(CapsuleMode::Listening, "new input".into());
                    }
                    _ => {
                        state.notice_view("newer notice");
                    }
                }
                if failure {
                    Err("old failure".into())
                } else {
                    idle()
                }
            });
            assert!(update.is_none());
            let state = state.lock().unwrap();
            assert!(!state.capture.owns(Some("old")));
            if action == 2 {
                assert!(state.capture.owns(Some("replacement")));
            }
            if action == 3 {
                assert_eq!(state.mode, Some(CapsuleMode::Notice));
            }
        }
    }
}

#[test]
fn current_status_errors_and_other_microphone_owners_keep_their_notices() {
    for (status, message) in [
        (Err("STT state lock failed".into()), "STT state lock failed"),
        (occupied("live"), "ほかの音声入力が動作中です"),
    ] {
        let state = Mutex::new(SharedState::default());
        let request = state.lock().unwrap().press_shortcut().unwrap();
        let update = prepare_capture(&state, request, "new".into(), "prompt", || status).unwrap();
        assert!(!update.start);
        assert_eq!(update.view.text, message);
        assert!(update.view.is_current());
        assert!(state.lock().unwrap().capture.id().is_none());
    }
}

#[test]
fn existing_native_input_does_not_lose_its_capture_speech_or_response() {
    let state = Mutex::new(SharedState::default());
    let request = {
        let mut state = state.lock().unwrap();
        state.begin_stream("existing response".into());
        state.capture.begin("old input".into());
        state.current_speech = "recognized speech".into();
        state.press_shortcut().unwrap()
    };
    assert!(
        prepare_capture(&state, request, "new".into(), "prompt", || occupied(
            "native_agent"
        ))
        .is_none()
    );
    let state = state.lock().unwrap();
    assert!(state.capture.owns(Some("old input")));
    assert_eq!(state.current_speech, "recognized speech");
    assert!(state.owns_stream("existing response"));
}

#[test]
fn release_after_preflight_cannot_reserve_a_microphone_or_the_replacement_input() {
    let state = Mutex::new(SharedState::default());
    let request = state.lock().unwrap().press_shortcut().unwrap();
    let update = prepare_capture(&state, request, "new".into(), "prompt", idle).unwrap();
    assert!(update.start);
    let mut state = state.lock().unwrap();
    assert_eq!(state.reserve_listening_capture("new", || Ok(42)), Ok(42));
    state.shortcut.release();
    state.stop_requested = true;
    assert!(state
        .reserve_listening_capture::<()>("new", || panic!("released input reserved STT"))
        .is_err());
    state.close_view(None).unwrap();
    assert!(!update.view.is_current());
    state.capture.begin("replacement".into());
    state.prepare_view(CapsuleMode::Listening, "replacement".into());
    assert!(state
        .reserve_listening_capture::<()>("new", || panic!("old input reserved STT"))
        .is_err());
    assert_eq!(
        state.reserve_listening_capture("replacement", || Ok(7)),
        Ok(7)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_contended_status_read_keeps_executor_and_native_release_responsive() {
    let state = Arc::new(Mutex::new(SharedState::default()));
    let request = state.lock().unwrap().press_shortcut().unwrap();
    let stt = Arc::new(Mutex::new(()));
    let held = stt.lock().unwrap();
    let caller = std::thread::current().id();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let worker_state = state.clone();
    let worker_stt = stt.clone();
    let work = tauri::async_runtime::spawn_blocking(move || {
        prepare_capture(&worker_state, request, "late".into(), "prompt", || {
            assert_ne!(caller, std::thread::current().id());
            started.send(()).unwrap();
            let _guard = worker_stt.lock().unwrap();
            idle()
        })
    });
    waiting.await.unwrap();
    // This is the same state mutation used by native release callbacks.
    state
        .try_lock()
        .expect("native release blocked behind STT status")
        .shortcut
        .release();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    drop(held);
    assert!(work.await.unwrap().is_none());
    assert!(state.lock().unwrap().capture.id().is_none());
}

#[test]
fn a_replaced_release_deadline_cannot_consume_the_same_input_or_its_speech() {
    let mut state = SharedState::default();
    state.capture.begin("same input".into());
    state.prepare_view(CapsuleMode::Listening, "speech".into());
    state.stop_requested = true;
    state.current_speech = "recognized speech".into();
    assert!(!state.finish_released_capture("same input", 1, 2));
    assert!(state.capture.owns(Some("same input")));
    assert_eq!(state.current_speech, "recognized speech");
    assert!(!state.finish_released_capture("old input", 2, 2));
    state.stop_requested = false;
    assert!(!state.finish_released_capture("same input", 2, 2));
    state.stop_requested = true;
    assert!(state.finish_released_capture("same input", 2, 2));
    assert!(!state.finish_released_capture("same input", 2, 2));
}
