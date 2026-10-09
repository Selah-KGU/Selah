use super::*;
use crate::live::{tests::transcript::recording, LiveTranscriptLine};
use crate::stt::{ipc_status, SttStreamOwner, SttStreamPhase, SttStreamState};
use chrono::TimeZone;
use serde_json::{json, Value};
use std::sync::{
    mpsc::{channel, Receiver},
    Arc,
};
use std::time::Duration;
use tauri::ipc::IpcResponse;
use tauri::{
    ipc::{CallbackFn, InvokeBody, InvokeResponse},
    test::{mock_builder, mock_context, noop_assets, MockRuntime},
};

fn microphone(phase: SttStreamPhase, caller: &str, id: &str) -> SttStreamState {
    SttStreamState {
        phase,
        session_id: Some(7),
        owner: Some(SttStreamOwner {
            caller: caller.into(),
            live_session_id: Some(id.into()),
            input_session_id: None,
        }),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worker_preserves_complete_metadata_bytes_and_does_not_capture_history_or_revision() {
    let state = LiveState::new();
    let mut session = recording();
    session.transcript_lines = Arc::new(
        (0..10000)
            .map(|i| {
                Arc::new(LiveTranscriptLine {
                    text: format!("完全な字幕 {i} 🌙"),
                    at: "10:00:00".into(),
                })
            })
            .collect(),
    );
    session.pending_lines = Arc::clone(&session.transcript_lines);
    let lines = Arc::downgrade(&session.transcript_lines);
    *state.session.lock().unwrap() = Some(session);
    let before = state
        .snapshot_revision
        .load(std::sync::atomic::Ordering::Relaxed);
    let caller = std::thread::current().id();
    for (phase, owner, id) in [
        (SttStreamPhase::Listening, "live", "recording-test"),
        (SttStreamPhase::Initializing, "live", "recording-test"),
        (SttStreamPhase::Stopping, "live", "recording-test"),
        (SttStreamPhase::Listening, "agent", "recording-test"),
        (SttStreamPhase::Listening, "live", "old"),
    ] {
        let expected =
            serde_json::to_string(&snapshot(&state, || Ok(microphone(phase, owner, id))).unwrap())
                .unwrap();
        let body = reply(state.clone(), move || {
            assert_ne!(std::thread::current().id(), caller);
            Ok(microphone(phase, owner, id))
        })
        .await
        .unwrap()
        .body()
        .unwrap();
        assert!(matches!(body,tauri::ipc::InvokeResponseBody::Json(actual) if actual==expected));
        assert_eq!(lines.strong_count(), 2);
    }
    assert_eq!(
        state
            .snapshot_revision
            .load(std::sync::atomic::Ordering::Relaxed),
        before
    );
    *state.session.lock().unwrap() = None;
    let idle = reply(state, || panic!("inactive must skip STT"))
        .await
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<Value>()
        .unwrap();
    assert_eq!(
        idle,
        json!({"active":false,"listening":false,"started_at":null})
    );
    assert_eq!(lines.strong_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn a_busy_live_owner_waits_on_a_worker_and_reads_the_state_after_release() {
    let state = LiveState::new();
    let held = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = channel();
    let holder = std::thread::spawn(move || {
        let mut guard = held.session.lock().unwrap();
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        *guard = Some(recording());
    });
    started.await.unwrap();
    let reading = tokio::spawn(reply(state, || {
        Ok(microphone(
            SttStreamPhase::Listening,
            "live",
            "recording-test",
        ))
    }));
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let pending = !reading.is_finished();
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(pending);
    let status = reading
        .await
        .unwrap()
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<Value>()
        .unwrap();
    assert_eq!(status["active"], true);
    assert_eq!(status["listening"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn a_slow_microphone_read_preserves_lock_order_without_blocking_async_tasks_and_returns_its_error(
) {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(recording());
    let held = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = channel();
    let reading = tokio::spawn(reply(state.clone(), move || {
        assert!(held.session.try_lock().is_err());
        assert!(held.persistence.gate.try_lock().is_ok());
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        Err("STT state lock failed".into())
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let pending = !reading.is_finished();
    release.send(()).unwrap();
    assert!(pending);
    assert_eq!(
        reading.await.unwrap().err().unwrap(),
        "STT state lock failed"
    );
    assert!(state.session.lock().unwrap().is_some());
}

mod legacy {
    use super::*;
    #[tauri::command]
    fn live_get_tray_status(state: tauri::State<'_, LiveState>) -> Result<LiveTrayStatus, String> {
        snapshot(&state, crate::stt::stt_get_stream_state)
    }
    #[tauri::command]
    fn stt_is_running() -> bool {
        crate::stt::stt_is_running()
    }
    #[tauri::command]
    fn stt_get_active_caller() -> Option<String> {
        crate::stt::stt_get_active_caller()
    }
    #[tauri::command]
    fn stt_get_stream_state() -> Result<SttStreamState, String> {
        crate::stt::stt_get_stream_state()
    }
    pub(super) fn handler(
    ) -> impl Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync + 'static {
        tauri::generate_handler![
            live_get_tray_status,
            stt_is_running,
            stt_get_active_caller,
            stt_get_stream_state
        ]
    }
}

fn app(state: LiveState, old: bool) -> tauri::App<MockRuntime> {
    let builder = mock_builder().manage(state);
    let builder = if old {
        builder.invoke_handler(legacy::handler())
    } else {
        builder.invoke_handler(tauri::generate_handler![
            crate::live::commands::live_get_tray_status,
            ipc_status::stt_is_running,
            ipc_status::stt_get_active_caller,
            ipc_status::stt_get_stream_state
        ])
    };
    builder.build(mock_context(noop_assets())).unwrap()
}
fn window(app: &tauri::App<MockRuntime>) -> tauri::WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
        .build()
        .unwrap()
}
fn request(
    window: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
) -> Receiver<Result<String, Value>> {
    let (sent, received) = channel();
    window.as_ref().clone().on_message(
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: InvokeBody::Json(json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
        Box::new(move |_, _, response, _, _| {
            let result = match response {
                InvokeResponse::Ok(tauri::ipc::InvokeResponseBody::Json(json)) => Ok(json),
                InvokeResponse::Err(error) => Err(error.0),
                _ => panic!("status reply must be raw JSON"),
            };
            let _ = sent.send(result);
        }),
    );
    received
}
fn call(window: &tauri::WebviewWindow<MockRuntime>, command: &str) -> Result<Value, Value> {
    request(window, command)
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .map(|json| serde_json::from_str(&json).unwrap())
}

#[test]
fn actual_ipc_status_commands_match_legacy_objects_scalars_nulls_and_errors() {
    let state = LiveState::new();
    let old = app(state.clone(), true);
    let current = app(state.clone(), false);
    let old_window = window(&old);
    let new_window = window(&current);
    let mut wire = serde_json::Map::new();
    for command in [
        "live_get_tray_status",
        "stt_is_running",
        "stt_get_active_caller",
        "stt_get_stream_state",
    ] {
        let actual = call(&new_window, command).unwrap();
        assert_eq!(actual, call(&old_window, command).unwrap());
        wire.insert(command.into(), actual);
    }
    let mut session = recording();
    session.started_at = chrono::Local
        .with_ymd_and_hms(2026, 10, 7, 11, 45, 0)
        .single()
        .unwrap();
    *state.session.lock().unwrap() = Some(session);
    let active = call(&new_window, "live_get_tray_status").unwrap();
    assert_eq!(active, call(&old_window, "live_get_tray_status").unwrap());
    wire.insert("live_active".into(), active);
    let poisoned = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoned.session.lock().unwrap();
        panic!("fixture poisoned LIVE owner");
    })
    .join();
    let error = call(&new_window, "live_get_tray_status").unwrap_err();
    assert_eq!(
        error,
        call(&old_window, "live_get_tray_status").unwrap_err()
    );
    wire.insert("live_error".into(), error);
    let wire = Value::Object(wire);
    if let Ok(path) = std::env::var("SELAH_STATUS_READ_WIRE") {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&wire).unwrap()),
        )
        .unwrap();
    }
    assert_eq!(
        wire,
        serde_json::from_str::<Value>(include_str!(
            "../../../../../tests/fixtures/status-read-wire.json"
        ))
        .unwrap()
    );
}

#[test]
fn actual_tray_handler_returns_to_ipc_while_its_live_read_waits_for_the_owner() {
    let state = LiveState::new();
    let app = app(state.clone(), false);
    let window = window(&app);
    let guard = state.session.lock().unwrap();
    let (returned, dispatch) = channel();
    let caller = std::thread::spawn(move || {
        let response = request(&window, "live_get_tray_status");
        returned.send(response).unwrap();
    });
    let dispatched = dispatch.recv_timeout(Duration::from_secs(2));
    // Release before asserting so a synchronous regression cannot hang tests.
    drop(guard);
    caller.join().unwrap();
    let response = dispatched.expect("synchronous tray handler waited on the caller's LIVE lock");
    let json = response
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&json).unwrap(),
        json!({"active":false,"listening":false,"started_at":null})
    );
}
