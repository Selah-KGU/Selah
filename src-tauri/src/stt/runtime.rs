use super::*;

#[derive(Clone, Serialize)]
struct SttEventPayload {
    text: String,
    caller: String,
    /// Capture order. Live captions use it so a newer partial is not wiped
    /// when an older final finishes decoding later.
    seq: u64,
}

#[derive(Clone, Serialize)]
struct SttStatePayload {
    state: String,
    caller: String,
}

#[derive(Clone, Serialize)]
struct SttInfoPayload {
    message: String,
    caller: String,
}

#[derive(Debug, Clone, Default)]
struct SttRuntimeDebugState {
    execution_backend: Option<String>,
    fallback_from: Option<String>,
    state: String,
    active_caller: Option<String>,
    last_info: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SttRuntimeDebugInfo {
    pub configured_backend: String,
    pub configured_partial_mode: String,
    pub configured_sensitivity: String,
    pub runtime_backend: String,
    pub runtime_state: String,
    pub active_caller: String,
    pub runtime_note: String,
    pub runtime_error: String,
}

pub(in crate::stt) struct ActiveSttSession {
    pub(in crate::stt) id: u64,
    pub(in crate::stt) caller: String,
    pub(in crate::stt) stop_tx: mpsc::Sender<()>,
}

pub(in crate::stt) static STT_SESSION: Mutex<Option<ActiveSttSession>> = Mutex::new(None);
pub(in crate::stt) static STT_SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static STT_RUNTIME_DEBUG: LazyLock<Mutex<SttRuntimeDebugState>> = LazyLock::new(|| {
    Mutex::new(SttRuntimeDebugState {
        execution_backend: None,
        fallback_from: None,
        state: "idle".into(),
        active_caller: None,
        last_info: None,
        last_error: None,
    })
});
pub(in crate::stt) static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

pub(in crate::stt) fn clear_session_if_matches(id: u64) {
    if let Ok(mut lock) = STT_SESSION.lock() {
        if lock.as_ref().map(|s| s.id) == Some(id) {
            *lock = None;
        }
    }
}

fn stt_runtime_state_label(state: &str) -> &'static str {
    match state {
        "initializing" => "初期化中",
        "listening" => "音声入力中",
        "test-ok" => "テスト成功",
        _ => "待機",
    }
}

pub(in crate::stt) fn update_runtime_debug_state(
    state: &str,
    caller: Option<&str>,
    execution_backend: Option<&str>,
    fallback_from: Option<&str>,
) {
    if let Ok(mut debug) = STT_RUNTIME_DEBUG.lock() {
        debug.state = state.to_string();
        debug.active_caller = caller.map(|value| value.to_string());
        if let Some(execution_backend) = execution_backend {
            debug.execution_backend = Some(execution_backend.to_string());
            debug.fallback_from = fallback_from.map(|value| value.to_string());
        }
        if state == "idle" {
            debug.active_caller = None;
        }
    }
}

pub(in crate::stt) fn update_runtime_debug_message(info: Option<String>, error: Option<String>) {
    if let Ok(mut debug) = STT_RUNTIME_DEBUG.lock() {
        if let Some(info) = info {
            debug.last_info = Some(info);
        }
        if let Some(error) = error {
            debug.last_error = Some(error);
        }
    }
}

pub(in crate::stt) fn emit_runtime_debug_changed(app: &tauri::AppHandle) {
    let _ = app.emit("stt-runtime-debug-changed", ());
}

fn stt_runtime_backend_debug_label(
    execution_backend: Option<&str>,
    fallback_from: Option<&str>,
) -> String {
    match execution_backend {
        Some(execution_backend) => {
            let active = stt_execution_backend_label(execution_backend);
            if let Some(fallback_from) = fallback_from {
                format!(
                    "{} ({} からフォールバック)",
                    active,
                    stt_execution_backend_label(fallback_from)
                )
            } else {
                active.to_string()
            }
        }
        None => "未初期化".into(),
    }
}

pub fn stt_runtime_debug_info() -> SttRuntimeDebugInfo {
    let config = load_config();
    let configured_backend = stt_execution_backend_label(&config.execution_backend).to_string();
    let configured_partial_mode = stt_partial_mode_label(&config.partial_mode).to_string();
    let configured_sensitivity = stt_sensitivity_label(&config.sensitivity).to_string();
    if let Ok(debug) = STT_RUNTIME_DEBUG.lock() {
        return SttRuntimeDebugInfo {
            configured_backend,
            configured_partial_mode,
            configured_sensitivity,
            runtime_backend: stt_runtime_backend_debug_label(
                debug.execution_backend.as_deref(),
                debug.fallback_from.as_deref(),
            ),
            runtime_state: stt_runtime_state_label(&debug.state).to_string(),
            active_caller: debug.active_caller.clone().unwrap_or_else(|| "-".into()),
            runtime_note: debug.last_info.clone().unwrap_or_default(),
            runtime_error: debug.last_error.clone().unwrap_or_default(),
        };
    }

    SttRuntimeDebugInfo {
        configured_backend,
        configured_partial_mode,
        configured_sensitivity,
        runtime_backend: "未取得".into(),
        runtime_state: "待機".into(),
        active_caller: "-".into(),
        runtime_note: String::new(),
        runtime_error: String::new(),
    }
}

pub(in crate::stt) fn emit_state(app: &tauri::AppHandle, state: &str, caller: &str) {
    update_runtime_debug_state(state, Some(caller), None, None);
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-state",
        SttStatePayload {
            state: state.to_string(),
            caller: caller.to_string(),
        },
    );
}

pub(in crate::stt) fn emit_error(app: &tauri::AppHandle, message: impl Into<String>, caller: &str) {
    let message = message.into();
    update_runtime_debug_message(None, Some(message.clone()));
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-error",
        serde_json::json!({ "message": message, "caller": caller }),
    );
}

pub(in crate::stt) fn emit_info(app: &tauri::AppHandle, message: impl Into<String>, caller: &str) {
    let message = message.into();
    update_runtime_debug_message(Some(message.clone()), None);
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-info",
        SttInfoPayload {
            message,
            caller: caller.to_string(),
        },
    );
}

pub(in crate::stt) fn emit_partial(app: &tauri::AppHandle, text: String, caller: &str, seq: u64) {
    let _ = app.emit(
        "stt-partial",
        SttEventPayload {
            text,
            caller: caller.to_string(),
            seq,
        },
    );
}

fn emit_final(app: &tauri::AppHandle, text: String, caller: &str, seq: u64) {
    let _ = app.emit(
        "stt-final",
        SttEventPayload {
            text,
            caller: caller.to_string(),
            seq,
        },
    );
}

/// Emit a final transcript line, but suppress it when SenseVoice repeats
/// itself on adjacent VAD segments. This happens occasionally when the VAD
/// splits an utterance at an unlucky point and both pieces get decoded to
/// the same phrase. `last_final` is updated with whatever we end up keeping
/// (so any legitimate later repeat of the same phrase, spaced by other
/// content, still goes through).
pub(in crate::stt) fn emit_final_deduped(
    app: &tauri::AppHandle,
    text: String,
    caller: &str,
    seq: u64,
    last_final: &mut String,
) {
    if text.is_empty() {
        return;
    }
    if text == *last_final {
        return;
    }
    *last_final = text.clone();
    emit_final(app, text, caller, seq);
}
