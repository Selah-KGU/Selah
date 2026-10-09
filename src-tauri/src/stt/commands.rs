//! Tauri commands for speech-to-text config, models, and streaming.

use super::lifecycle::{SttLifecycleGate, SttModelOperation};
use super::*;
use tauri::Manager;

static STT_LIFECYCLE_GATE: LazyLock<SttLifecycleGate> = LazyLock::new(SttLifecycleGate::default);

async fn run_stt_blocking<R: Send + 'static>(
    operation: impl FnOnce() -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|err| format!("音声認識の処理に失敗しました: {err}"))?
}

pub(super) fn reserve_model_operation() -> Result<SttModelOperation<'static>, String> {
    STT_LIFECYCLE_GATE.reserve_model_operation(|| {
        if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            return Err("アプリケーションを終了中です".into());
        }
        let session = STT_SESSION
            .lock()
            .map_err(|_| "STT state lock failed".to_string())?;
        if let Some(session) = session.active.as_ref() {
            return Err(format!(
                "音声入力は「{}」で使用中です。停止してからモデルを操作してください",
                session.caller
            ));
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn get_stt_config() -> Result<SttConfig, String> {
    run_stt_blocking(|| Ok(load_config())).await
}

#[tauri::command]
pub async fn save_stt_config(app: tauri::AppHandle, config: SttConfig) -> Result<(), String> {
    run_stt_blocking(move || save_stt_config_blocking(&app, config)).await
}

fn save_stt_config_blocking(app: &tauri::AppHandle, mut config: SttConfig) -> Result<(), String> {
    config.selected_model = normalize_stt_model_id(&config.selected_model);
    config.language = normalize_stt_language(&config.language);
    config.execution_backend = validate_stt_execution_backend(&config.execution_backend)?;
    config.partial_mode = normalize_stt_partial_mode(&config.partial_mode);
    config.sensitivity = normalize_stt_sensitivity(&config.sensitivity);
    if stt_model_catalog()
        .iter()
        .all(|m| m.id != config.selected_model)
    {
        return Err("不明な STT モデルです".into());
    }
    save_config(&config)?;
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn list_stt_execution_backends() -> Vec<SttExecutionBackendInfo> {
    stt_execution_backend_catalog()
}

#[tauri::command]
pub async fn list_stt_models() -> Result<Vec<serde_json::Value>, String> {
    run_stt_blocking(|| Ok(stt_model_status())).await
}

fn stt_model_status() -> Vec<serde_json::Value> {
    stt_model_catalog()
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "size_label": m.size_label,
                "file_size_mb": m.file_size_mb,
                "downloaded": is_stt_model_downloaded(m),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn download_stt_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let model = stt_model_catalog()
        .iter()
        .find(|m| m.id == model_id)
        .cloned()
        .ok_or_else(|| format!("不明な STT モデル: {}", model_id))?;
    run_stt_blocking(move || {
        download_stt_model_blocking(&app, &model)?;
        let _ = app.emit("stt-config-changed", ());
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn delete_stt_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    run_stt_blocking(move || delete_stt_model_blocking(&app, &model_id)).await
}

fn delete_stt_model_blocking(app: &tauri::AppHandle, model_id: &str) -> Result<(), String> {
    let _reservation = reserve_model_operation()?;
    let model = stt_model_catalog()
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("不明な STT モデル: {}", model_id))?;

    let model_dir = stt_model_dir(model);
    if model_dir.exists() {
        std::fs::remove_dir_all(&model_dir).map_err(|e| format!("削除失敗: {}", e))?;
    }
    let archive = stt_archive_path(model);
    if archive.exists() {
        let _ = std::fs::remove_file(&archive);
    }
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn cancel_stt_model_download() {
    cancel_stt_download();
}

#[tauri::command]
pub async fn stt_test_model(app: tauri::AppHandle) -> Result<String, String> {
    run_stt_blocking(move || stt_test_model_blocking(&app)).await
}

fn stt_test_model_blocking(app: &tauri::AppHandle) -> Result<String, String> {
    let _reservation = reserve_model_operation()?;
    // One settings snapshot chooses both model and execution preferences.
    let config = load_config();
    let model = selected_model_for_config(&config)?;
    ensure_stt_model_downloaded(&model)?;
    let recognizer_init = create_recognizer_for_config(&model, &config)?;
    update_runtime_debug_state(
        "test-ok",
        None,
        Some(&recognizer_init.execution_backend),
        recognizer_init.fallback_from.as_deref(),
    );
    // Release native resources before notifying the UI or admitting a start.
    drop(recognizer_init.recognizer);

    if let Some(fallback_from) = recognizer_init.fallback_from {
        let message = format!(
            "OK: {} ({}) / {}",
            model.name,
            stt_execution_backend_label(&recognizer_init.execution_backend),
            stt_fallback_message(&fallback_from)
        );
        update_runtime_debug_message(Some(message.clone()), None);
        emit_runtime_debug_changed(app);
        return Ok(message);
    }

    let message = format!(
        "OK: {} ({})",
        model.name,
        stt_execution_backend_label(&recognizer_init.execution_backend)
    );
    update_runtime_debug_message(Some(message.clone()), None);
    emit_runtime_debug_changed(app);
    Ok(message)
}

pub fn stt_is_running() -> bool {
    STT_SESSION
        .lock()
        .map(|s| s.active.is_some())
        .unwrap_or(false)
}

pub fn stt_get_active_caller() -> Option<String> {
    STT_SESSION
        .lock()
        .ok()
        .and_then(|s| s.active.as_ref().map(|sess| sess.caller.clone()))
}

/// One reservation lock supplies both phase and owner; debug/model state is
/// intentionally excluded so a foreground recovery read performs no IO.
pub fn stt_get_stream_state() -> Result<SttStreamState, String> {
    let session = STT_SESSION
        .lock()
        .map_err(|_| "STT state lock failed".to_string())?;
    Ok(SttStreamState::from_session(session.active.as_ref()))
}

const STT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

fn wait_for_stop(control: &SttSessionControl) -> Result<(), String> {
    if control.wait(STT_DRAIN_TIMEOUT) {
        Ok(())
    } else {
        Err("最後の音声を処理中です。少し待ってからもう一度停止・保存してください".into())
    }
}

fn request_stop(
    caller: Option<&str>,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) -> Result<Option<Arc<SttSessionControl>>, String> {
    let lock = STT_SESSION
        .lock()
        .map_err(|_| "STT state lock failed".to_string())?;
    Ok(lock.active.as_ref().and_then(|session| {
        session.request_stop_for_owner(caller, live_session_id, input_session_id)
    }))
}

#[tauri::command]
pub async fn stt_start_stream(
    app: tauri::AppHandle,
    caller: String,
    preempt: Option<bool>,
    live_session_id: Option<String>,
    input_session_id: Option<String>,
) -> Result<Option<SttStreamOwner>, String> {
    validate_input_session_id(Some(&caller), input_session_id.as_deref())?;
    // Capture the LIVE owner before a queued preemption waits for teardown.
    // Cancellation must not retarget this request to a replacement recording.
    let live_session_id =
        validate_live_session_id(Some(&caller), live_session_id.as_deref())?.map(str::to_owned);
    with_start_owner(
        &app,
        &caller,
        live_session_id.as_deref(),
        input_session_id.as_deref(),
        || Ok(()),
    )?;
    tokio::task::spawn_blocking(move || {
        let _gate = STT_LIFECYCLE_GATE.lock_start()?;
        let (previous_caller, previous_control) = with_start_owner(
            &app,
            &caller,
            live_session_id.as_deref(),
            input_session_id.as_deref(),
            || {
                let lock = STT_SESSION
                    .lock()
                    .map_err(|_| "STT state lock failed".to_string())?;
                if let Some(session) = lock.active.as_ref() {
                    if !preempt.unwrap_or(false) && !session.control.is_stopping() {
                        return Err(format!("音声入力は「{}」で使用中です", session.caller));
                    }
                }
                let previous = lock.borrowed_owner(
                    &caller,
                    input_session_id.as_deref(),
                    preempt.unwrap_or(false),
                );
                let control = lock.active.as_ref().map(|session| {
                    session.control.request_stop();
                    Arc::clone(&session.control)
                });
                Ok((previous, control))
            },
        )?;
        if let Some(control) = previous_control.as_ref() {
            wait_for_stop(control)?;
        }
        start_stream_now(
            app,
            caller,
            live_session_id,
            input_session_id,
            previous_caller.clone(),
        )?;
        Ok(previous_caller)
    })
    .await
    .map_err(|err| format!("音声入力の開始に失敗しました: {err}"))?
}

/// Native shortcut callbacks request startup without waiting on the UI thread.
/// The active session remains reserved during teardown, so models cannot overlap.
pub(crate) fn stt_start_native_input_now(
    app: tauri::AppHandle,
    input_session_id: String,
) -> Result<(), String> {
    validate_input_session_id(Some("native_agent"), Some(&input_session_id))?;
    let _gate = STT_LIFECYCLE_GATE.try_start()?;
    start_stream_now(
        app,
        "native_agent".into(),
        None,
        Some(input_session_id),
        None,
    )
}

fn validate_live_session_id<'a>(
    caller: Option<&str>,
    expected: Option<&'a str>,
) -> Result<Option<&'a str>, String> {
    match (caller, expected) {
        (Some("live"), Some(id)) if !id.is_empty() => Ok(Some(id)),
        (Some("live"), _) => Err("Live録音IDが必要です".into()),
        (_, Some(_)) => Err("Live録音IDはLIVEの音声入力にのみ指定できます".into()),
        (_, None) => Ok(None),
    }
}

fn validate_input_session_id(caller: Option<&str>, expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(str::is_empty)
        || (matches!(caller, Some("agent" | "native_agent")) && expected.is_none())
    {
        return Err("Agent音声入力IDが必要です".into());
    }
    Ok(())
}

fn with_start_owner<T>(
    app: &tauri::AppHandle,
    caller: &str,
    expected: Option<&str>,
    input_id: Option<&str>,
    reserve: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if caller == "native_agent" {
        let id = input_id.ok_or_else(|| "Agent音声入力IDが必要です".to_string())?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        return crate::native_capture::with_capture_owner(id, reserve);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = id;
            return Err("Native音声入力はこの環境では使用できません".into());
        }
    }
    match expected {
        Some(id) => app
            .state::<crate::live::LiveState>()
            .with_microphone_owner(id, reserve),
        None => reserve(),
    }
}

fn start_stream_now(
    app: tauri::AppHandle,
    caller: String,
    live_session_id: Option<String>,
    input_session_id: Option<String>,
    resume_owner: Option<SttStreamOwner>,
) -> Result<(), String> {
    if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
        return Err("アプリケーションを終了中です".into());
    }
    let caller = if caller.is_empty() {
        "unknown".to_string()
    } else {
        caller
    };
    let (session_id, control) = super::reservation::with_available_registry(
        &STT_SESSION,
        |reserve| {
            with_start_owner(
                &app,
                &caller,
                live_session_id.as_deref(),
                input_session_id.as_deref(),
                reserve,
            )
        },
        |lock| {
            if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                return Err("アプリケーションを終了中です".into());
            }
            let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
            let control = Arc::new(SttSessionControl::default());
            lock.reserve(
                ActiveSttSession {
                    id: session_id,
                    caller: caller.clone(),
                    live_session_id: live_session_id.clone(),
                    input_session_id: input_session_id.clone(),
                    phase: SttStreamPhase::Initializing,
                    control: Arc::clone(&control),
                },
                resume_owner,
            )?;
            Ok((session_id, control))
        },
    )?;
    let failure_control = Arc::clone(&control);
    std::thread::Builder::new()
        .name("stt-capture".into())
        .spawn(move || {
            run_stt_session(
                app,
                session_id,
                control,
                &caller,
                live_session_id,
                input_session_id,
            )
        })
        .map_err(|err| {
            clear_session_if_matches(session_id);
            failure_control.finish();
            format!("音声入力スレッドの起動に失敗しました: {err}")
        })?;
    Ok(())
}

#[tauri::command]
pub async fn stt_stop_stream(
    caller: Option<String>,
    live_session_id: Option<String>,
    input_session_id: Option<String>,
) -> Result<(), String> {
    validate_input_session_id(caller.as_deref(), input_session_id.as_deref())?;
    stop_stream_for_owner(caller, live_session_id, input_session_id).await
}

pub(crate) async fn stop_stream_for_caller(
    caller: Option<String>,
    live_session_id: Option<String>,
) -> Result<(), String> {
    validate_input_session_id(caller.as_deref(), None)?;
    stop_stream_for_owner(caller, live_session_id, None).await
}

async fn stop_stream_for_owner(
    caller: Option<String>,
    live_session_id: Option<String>,
    input_session_id: Option<String>,
) -> Result<(), String> {
    validate_live_session_id(caller.as_deref(), live_session_id.as_deref())?;
    tokio::task::spawn_blocking(move || {
        // No lifecycle gate here: a stop can cancel initialization even while
        // a start/preemption operation waits on the previous session.
        if let Some(control) = request_stop(
            caller.as_deref(),
            live_session_id.as_deref(),
            input_session_id.as_deref(),
        )? {
            wait_for_stop(&control)?;
        }
        Ok(())
    })
    .await
    .map_err(|err| format!("音声入力の停止に失敗しました: {err}"))?
}

/// Request an exact native input stop without waiting on the caller thread.
/// Busy registry access is queued; acceptance does not imply teardown completion.
pub(crate) fn stt_request_native_input_stop(input_session_id: &str) -> Result<(), String> {
    validate_input_session_id(Some("native_agent"), Some(input_session_id))?;
    super::native_stop::request(input_session_id)
}

pub(crate) fn stt_shutdown_for_exit(timeout: Duration) -> Result<bool, String> {
    STT_SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    Ok(request_stop(None, None, None)?.is_none_or(|control| control.wait(timeout)))
}

pub(crate) fn stt_cancel_shutdown() {
    STT_SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_ipc_requires_the_specific_input_identity_while_native_and_live_keep_their_owners() {
        assert!(validate_input_session_id(Some("agent"), None).is_err());
        assert!(validate_input_session_id(Some("agent"), Some("")).is_err());
        assert!(validate_input_session_id(Some("agent"), Some("input-a")).is_ok());
        assert!(stt_request_native_input_stop("").is_err());
        assert!(validate_input_session_id(Some("native_agent"), None).is_err());
        assert!(validate_input_session_id(Some("native_agent"), Some("native-a")).is_ok());
        for caller in [None, Some("live")] {
            assert!(validate_input_session_id(caller, None).is_ok());
            assert!(validate_input_session_id(caller, Some("")).is_err());
        }
    }

    #[test]
    fn live_microphone_commands_require_an_explicit_recording_owner() {
        assert!(validate_live_session_id(Some("live"), None).is_err());
        assert!(validate_live_session_id(Some("live"), Some("")).is_err());
        assert_eq!(
            validate_live_session_id(Some("live"), Some("recording-a")).unwrap(),
            Some("recording-a")
        );
        for caller in [None, Some("agent"), Some("native_agent")] {
            assert!(validate_live_session_id(caller, Some("recording-a")).is_err());
            assert!(validate_live_session_id(caller, None).unwrap().is_none());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_model_work_keeps_the_runtime_responsive_and_survives_a_canceled_caller() {
        let gate = Arc::new(SttLifecycleGate::default());
        let operation_gate = Arc::clone(&gate);
        let runtime_thread = std::thread::current().id();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (finished, complete) = tokio::sync::oneshot::channel();
        let (release, blocked) = mpsc::channel();
        let task = tokio::spawn(run_stt_blocking(move || {
            assert_ne!(std::thread::current().id(), runtime_thread);
            {
                let _reservation = operation_gate.reserve_model_operation(|| Ok(()))?;
                entered.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            finished.send(()).unwrap();
            Ok(())
        }));
        tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        // Another async task can run while the model thread is held in flight.
        assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            gate.try_start().is_err(),
            "caller cancellation released native resources early"
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), complete)
            .await
            .unwrap()
            .unwrap();
        assert!(gate.try_start().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_failures_preserve_the_operation_error_and_do_not_poison_later_work() {
        let gate = Arc::new(SttLifecycleGate::default());
        let failed_gate = Arc::clone(&gate);
        let error = run_stt_blocking(move || -> Result<(), String> {
            let _reservation = failed_gate.reserve_model_operation(|| Ok(()))?;
            Err("model file unreadable".into())
        })
        .await;
        assert_eq!(error.unwrap_err(), "model file unreadable");
        let panic_gate = Arc::clone(&gate);
        let error = run_stt_blocking(move || -> Result<(), String> {
            let _reservation = panic_gate.reserve_model_operation(|| Ok(()))?;
            panic!("native model initialization panicked");
        })
        .await;
        assert!(error
            .unwrap_err()
            .starts_with("音声認識の処理に失敗しました:"));
        assert!(gate.try_start().is_ok());
        assert_eq!(run_stt_blocking(|| Ok(7)).await.unwrap(), 7);
    }
}
