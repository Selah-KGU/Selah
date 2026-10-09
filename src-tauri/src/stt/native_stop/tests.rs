use super::*;
use crate::stt::{runtime::ActiveSttSession, SttSessionControl, SttStreamPhase};
use std::sync::Arc;

fn session(caller: &str, input: &str, phase: SttStreamPhase) -> ActiveSttSession {
    ActiveSttSession {
        id: 7,
        caller: caller.into(),
        live_session_id: Some("live identity".into()),
        input_session_id: Some(input.into()),
        phase,
        control: Arc::new(SttSessionControl::default()),
    }
}

#[test]
fn uncontended_stop_is_immediate_and_matches_caller_and_input_in_every_phase() {
    for phase in [
        SttStreamPhase::Initializing,
        SttStreamPhase::Listening,
        SttStreamPhase::Stopping,
    ] {
        for (caller, input, stops) in [
            ("native_agent", "input", true),
            ("native_agent", "new input", false),
            ("live", "input", false),
            ("agent", "input", false),
        ] {
            let mut state = SttInputState::default();
            let session = session(caller, input, phase);
            let control = session.control.clone();
            state.reserve(session, None).unwrap();
            let state = Arc::new(Mutex::new(state));
            assert!(request_on_registry(state.clone(), "input")
                .unwrap()
                .is_none());
            assert_eq!(control.is_stopping(), stops);
            // A stop requests drain; it never clears ownership before teardown.
            assert_eq!(state.lock().unwrap().active.as_ref().unwrap().phase, phase);
            assert!(!control.wait(std::time::Duration::ZERO));
        }
    }
    assert!(
        request_on_registry(Arc::new(Mutex::new(SttInputState::default())), "input")
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn contended_stop_returns_before_registry_release_and_cannot_stop_a_replacement() {
    for replace in [false, true] {
        let state = Arc::new(Mutex::new(SttInputState::default()));
        let old = session("native_agent", "input", SttStreamPhase::Initializing);
        let old_control = old.control.clone();
        let mut held = state.lock().unwrap();
        held.reserve(old, None).unwrap();
        let work = request_on_registry(state.clone(), "input")
            .unwrap()
            .unwrap();
        assert!(!old_control.is_stopping());
        assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        let next = session("native_agent", "replacement", SttStreamPhase::Listening);
        let next_control = next.control.clone();
        if replace {
            held.active = Some(next);
        }
        drop(held);
        work.await.unwrap().unwrap();
        assert_eq!(old_control.is_stopping(), !replace);
        assert!(!next_control.is_stopping());
        assert!(state.lock().unwrap().active.is_some());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dropped_stop_handle_keeps_the_accepted_request_alive() {
    let state = Arc::new(Mutex::new(SttInputState::default()));
    let active = session("native_agent", "input", SttStreamPhase::Listening);
    let control = active.control.clone();
    let mut held = state.lock().unwrap();
    held.reserve(active, None).unwrap();
    drop(
        request_on_registry(state.clone(), "input")
            .unwrap()
            .unwrap(),
    );
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !control.is_stopping() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping a response discarded its stop");
    assert!(state.lock().unwrap().active.is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn immediate_and_deferred_registry_poison_return_the_existing_error() {
    let state = Arc::new(Mutex::new(SttInputState::default()));
    let held = state.lock().unwrap();
    let work = request_on_registry(state.clone(), "input")
        .unwrap()
        .unwrap();
    assert!(std::panic::catch_unwind(move || {
        let _held = held;
        panic!("poison isolated STT fixture");
    })
    .is_err());
    assert_eq!(work.await.unwrap().unwrap_err(), "STT state lock failed");
    assert_eq!(
        request_on_registry(state, "input").err().unwrap(),
        "STT state lock failed"
    );
}
