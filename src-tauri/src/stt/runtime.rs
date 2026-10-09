use super::*;

#[derive(Clone, Serialize)]
struct SttEventPayload {
    text: String,
    caller: String,
    /// Capture order. Live captions use it so a newer partial is not wiped
    /// when an older final finishes decoding later.
    seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_session_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct SttStatePayload {
    state: String,
    caller: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_session_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct SttInfoPayload {
    message: String,
    caller: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_session_id: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SttStreamPhase {
    Idle,
    Initializing,
    Listening,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SttStreamOwner {
    pub caller: String,
    pub live_session_id: Option<String>,
    pub input_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SttStreamState {
    pub phase: SttStreamPhase,
    pub session_id: Option<u64>,
    pub owner: Option<SttStreamOwner>,
}

impl SttStreamState {
    pub(in crate::stt) fn from_session(session: Option<&ActiveSttSession>) -> Self {
        match session {
            Some(session) => Self {
                phase: if session.control.is_stopping() {
                    SttStreamPhase::Stopping
                } else {
                    session.phase
                },
                session_id: Some(session.id),
                owner: Some(session.owner()),
            },
            None => Self {
                phase: SttStreamPhase::Idle,
                session_id: None,
                owner: None,
            },
        }
    }
}

pub(in crate::stt) struct ActiveSttSession {
    pub(in crate::stt) id: u64,
    pub(in crate::stt) caller: String,
    pub(in crate::stt) live_session_id: Option<String>,
    pub(in crate::stt) input_session_id: Option<String>,
    pub(in crate::stt) phase: SttStreamPhase,
    pub(in crate::stt) control: Arc<SttSessionControl>,
}

impl ActiveSttSession {
    fn owner(&self) -> SttStreamOwner {
        SttStreamOwner {
            caller: self.caller.clone(),
            live_session_id: self.live_session_id.clone(),
            input_session_id: self.input_session_id.clone(),
        }
    }

    pub(in crate::stt) fn request_stop_for_owner(
        &self,
        caller: Option<&str>,
        live_session_id: Option<&str>,
        input_session_id: Option<&str>,
    ) -> Option<Arc<SttSessionControl>> {
        if caller.is_some_and(|caller| self.caller != caller) {
            return None;
        }
        if live_session_id.is_some_and(|id| self.live_session_id.as_deref() != Some(id)) {
            return None;
        }
        if input_session_id.is_some_and(|id| self.input_session_id.as_deref() != Some(id)) {
            return None;
        }
        self.control.request_stop();
        Some(Arc::clone(&self.control))
    }
}

#[derive(Default)]
pub(in crate::stt) struct SttInputState {
    pub(in crate::stt) active: Option<ActiveSttSession>,
    // A borrowed input survives the empty gap between Agent microphone leases.
    // A successful return or a different non-Agent start consumes this owner.
    suspended_owner: Option<SttStreamOwner>,
}

impl SttInputState {
    pub(in crate::stt) fn borrowed_owner(
        &self,
        caller: &str,
        input_session_id: Option<&str>,
        preempt: bool,
    ) -> Option<SttStreamOwner> {
        if !preempt {
            return None;
        }
        match self.active.as_ref() {
            Some(session) if caller == "agent" && session.caller == "agent" => {
                (session.input_session_id.as_deref() != input_session_id)
                    .then(|| self.suspended_owner.clone())
                    .flatten()
            }
            Some(session) if !session.control.is_stopping() && session.caller != caller => {
                Some(session.owner())
            }
            None if caller == "agent" => self.suspended_owner.clone(),
            _ => None,
        }
    }

    pub(in crate::stt) fn reserve(
        &mut self,
        session: ActiveSttSession,
        previous: Option<SttStreamOwner>,
    ) -> Result<(), String> {
        if let Some(active) = self.active.as_ref() {
            return Err(format!("音声入力は「{}」で使用中です", active.caller));
        }
        self.suspended_owner = if session.caller == "agent" {
            previous
        } else {
            None
        };
        self.active = Some(session);
        Ok(())
    }

    fn clear_if_matches(&mut self, id: u64) {
        if self.active.as_ref().map(|session| session.id) == Some(id) {
            self.active = None;
        }
    }
}

pub(in crate::stt) static STT_SESSION: Mutex<SttInputState> = Mutex::new(SttInputState {
    active: None,
    suspended_owner: None,
});
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
        lock.clear_if_matches(id);
    }
}

fn update_session_phase(session: &mut Option<ActiveSttSession>, id: u64, state: &str) {
    if let Some(session) = session.as_mut().filter(|session| session.id == id) {
        session.phase = match state {
            "initializing" => SttStreamPhase::Initializing,
            "listening" => SttStreamPhase::Listening,
            // Ownership remains reserved through idle delivery and teardown.
            _ => SttStreamPhase::Stopping,
        };
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

pub(in crate::stt) fn emit_state(
    app: &tauri::AppHandle,
    session_id: u64,
    state: &str,
    caller: &str,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) {
    if let Ok(mut session) = STT_SESSION.lock() {
        update_session_phase(&mut session.active, session_id, state);
    }
    update_runtime_debug_state(state, Some(caller), None, None);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    crate::native_agent_events::publish(
        app,
        caller,
        input_session_id,
        crate::native_agent_events::NativeSttEvent::State(state),
    );
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-state",
        SttStatePayload {
            state: state.to_string(),
            caller: caller.to_string(),
            live_session_id: live_session_id.map(str::to_string),
            input_session_id: input_session_id.map(str::to_string),
        },
    );
}

pub(in crate::stt) fn emit_error(
    app: &tauri::AppHandle,
    message: impl Into<String>,
    caller: &str,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) {
    let message = message.into();
    update_runtime_debug_message(None, Some(message.clone()));
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    crate::native_agent_events::publish(
        app,
        caller,
        input_session_id,
        crate::native_agent_events::NativeSttEvent::Error(&message),
    );
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-error",
        SttInfoPayload {
            message,
            caller: caller.to_string(),
            live_session_id: live_session_id.map(str::to_string),
            input_session_id: input_session_id.map(str::to_string),
        },
    );
}

pub(in crate::stt) fn emit_info(
    app: &tauri::AppHandle,
    message: impl Into<String>,
    caller: &str,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) {
    let message = message.into();
    update_runtime_debug_message(Some(message.clone()), None);
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-info",
        SttInfoPayload {
            message,
            caller: caller.to_string(),
            live_session_id: live_session_id.map(str::to_string),
            input_session_id: input_session_id.map(str::to_string),
        },
    );
}

pub(in crate::stt) fn emit_partial(
    app: &tauri::AppHandle,
    text: String,
    caller: &str,
    seq: u64,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    crate::native_agent_events::publish(
        app,
        caller,
        input_session_id,
        crate::native_agent_events::NativeSttEvent::Speech {
            text: std::borrow::Cow::Borrowed(&text),
            is_final: false,
        },
    );
    let _ = app.emit(
        "stt-partial",
        SttEventPayload {
            text,
            caller: caller.to_string(),
            seq,
            live_session_id: live_session_id.map(str::to_string),
            input_session_id: input_session_id.map(str::to_string),
        },
    );
}

fn emit_final(
    app: &tauri::AppHandle,
    text: String,
    caller: &str,
    seq: u64,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    crate::native_agent_events::publish(
        app,
        caller,
        input_session_id,
        crate::native_agent_events::NativeSttEvent::Speech {
            text: std::borrow::Cow::Borrowed(&text),
            is_final: true,
        },
    );
    if caller == "live" {
        let Some(id) = live_session_id else {
            return;
        };
        match crate::live::append_recognized_transcript(app, &text, id, seq) {
            Ok(true) => {}
            Ok(false) => return,
            Err(err) => {
                log::warn!("[Live] failed to commit recognized transcript: {err}");
                emit_error(app, err, caller, live_session_id, input_session_id);
                return;
            }
        }
    }
    let _ = app.emit(
        "stt-final",
        SttEventPayload {
            text,
            caller: caller.to_string(),
            seq,
            live_session_id: live_session_id.map(str::to_string),
            input_session_id: input_session_id.map(str::to_string),
        },
    );
}

#[derive(Default)]
pub(in crate::stt) struct FinalTranscriptGate {
    last_seq: Option<u64>,
}

impl FinalTranscriptGate {
    fn deliver(&mut self, text: String, seq: u64, publish: impl FnOnce(String)) -> bool {
        if text.is_empty() || self.last_seq.is_some_and(|last| seq <= last) {
            return false;
        }
        self.last_seq = Some(seq);
        publish(text);
        true
    }
}

/// Final jobs carry unique capture sequence numbers and stay FIFO in their
/// own decode lane. Reject a replay of the same or an older job, but keep
/// identical text from distinct VAD segments: repeating a phrase is speech,
/// not evidence that the decoder processed the same audio twice. The gate
/// belongs to one worker/session, independent of partial-caption delivery.
pub(in crate::stt) fn emit_final_once(
    app: &tauri::AppHandle,
    text: String,
    caller: &str,
    seq: u64,
    gate: &mut FinalTranscriptGate,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) {
    gate.deliver(text, seq, |text| {
        emit_final(app, text, caller, seq, live_session_id, input_session_id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_speech_segments_preserve_legitimate_repetition_in_any_language() {
        for phrase in ["はい。", "Yes.", "对。", "123", "🌕\n同じ発話"] {
            let mut gate = FinalTranscriptGate::default();
            let mut delivered = Vec::new();
            for seq in [1, 4, 7] {
                assert!(gate.deliver(phrase.into(), seq, |text| delivered.push(text)));
            }
            assert_eq!(delivered, vec![phrase; 3]);
        }
    }

    #[test]
    fn replayed_or_older_segments_do_not_publish_even_if_their_text_changes() {
        let mut gate = FinalTranscriptGate::default();
        let mut delivered = Vec::new();
        assert!(gate.deliver("first".into(), 0, |text| delivered.push(text)));
        assert!(!gate.deliver("replay changed".into(), 0, |_| panic!("replayed")));
        assert!(gate.deliver("second".into(), 9, |text| delivered.push(text)));
        assert!(!gate.deliver("older".into(), 3, |_| panic!("out of order")));
        assert!(!gate.deliver("first".into(), 0, |_| panic!("older replay")));
        assert!(gate.deliver("second".into(), 10, |text| delivered.push(text)));
        assert_eq!(delivered, ["first", "second", "second"]);
    }

    #[test]
    fn empty_results_and_prior_recordings_do_not_consume_a_new_segment() {
        let mut gate = FinalTranscriptGate::default();
        assert!(!gate.deliver(String::new(), 5, |_| panic!("empty result")));
        assert!(gate.deliver("speech".into(), 5, |_| {}));
        assert!(!gate.deliver(String::new(), 99, |_| panic!("empty result")));
        assert!(gate.deliver("speech".into(), 6, |_| {}));
        let mut next_recording = FinalTranscriptGate::default();
        assert!(next_recording.deliver("speech".into(), 0, |_| {}));
    }

    fn recording(id: u64, caller: &str, live_id: Option<&str>) -> ActiveSttSession {
        ActiveSttSession {
            id,
            caller: caller.into(),
            live_session_id: live_id.map(str::to_owned),
            input_session_id: None,
            phase: SttStreamPhase::Initializing,
            control: Arc::new(SttSessionControl::default()),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn finished_native_input_cannot_preempt_or_reserve_a_microphone_on_late_return() {
        use crate::native_capture::NativeCapture;
        let mut intent = NativeCapture::default();
        intent.begin("native-a".into());
        let mut current = recording(30, "agent", None);
        current.input_session_id = Some("agent-b".into());
        let mut state = SttInputState {
            active: Some(current),
            suspended_owner: None,
        };
        assert!(intent.finish("native-a"));
        // The admission guard runs before the preemption closure, including
        // after waiting for the lifecycle gate and again at actual reservation.
        assert!(intent
            .reserve_if_current("native-a", || {
                state.active.as_ref().unwrap().control.request_stop();
                Ok(())
            })
            .is_err());
        assert!(!state.active.as_ref().unwrap().control.is_stopping());
        let snapshot = SttStreamState::from_session(state.active.as_ref());
        assert_eq!(
            snapshot.owner.unwrap().input_session_id.as_deref(),
            Some("agent-b")
        );
        state.active = None;
        assert!(intent
            .reserve_if_current("native-a", || {
                let mut session = recording(31, "native_agent", None);
                session.input_session_id = Some("native-a".into());
                state.reserve(session, None)
            })
            .is_err());
        assert!(state.active.is_none());
        intent.begin("native-c".into());
        assert!(intent
            .reserve_if_current("native-c", || {
                let mut session = recording(31, "native_agent", None);
                session.input_session_id = Some("native-c".into());
                state.reserve(session, None)
            })
            .is_ok());
        assert_eq!(
            state.active.as_ref().unwrap().input_session_id.as_deref(),
            Some("native-c")
        );
    }

    #[test]
    fn late_native_stop_cannot_cancel_a_new_native_live_or_agent_input() {
        for (caller, input_id, live_id) in [
            ("native_agent", Some("native-b"), None),
            ("agent", Some("agent-a"), None),
            ("live", None, Some("recording-a")),
        ] {
            for phase in [
                SttStreamPhase::Initializing,
                SttStreamPhase::Listening,
                SttStreamPhase::Stopping,
            ] {
                let mut session = recording(32, caller, None);
                session.phase = phase;
                session.input_session_id = input_id.map(str::to_owned);
                session.live_session_id = live_id.map(str::to_owned);
                assert!(session
                    .request_stop_for_owner(Some("native_agent"), None, Some("native-a"))
                    .is_none());
                assert!(!session.control.is_stopping());
                if caller == "native_agent" {
                    assert!(session
                        .request_stop_for_owner(Some("native_agent"), None, input_id)
                        .is_some());
                    assert!(session.control.is_stopping());
                }
            }
        }
    }

    #[test]
    fn closing_an_old_agent_view_cannot_stop_a_replacement_agent_input() {
        let mut session = recording(30, "agent", None);
        session.input_session_id = Some("input-b".into());
        assert!(session
            .request_stop_for_owner(Some("agent"), None, Some("input-a"))
            .is_none());
        assert!(!session.control.is_stopping());
        assert_eq!(
            SttStreamState::from_session(Some(&session))
                .owner
                .unwrap()
                .input_session_id
                .as_deref(),
            Some("input-b")
        );
        let stopped = session
            .request_stop_for_owner(Some("agent"), None, Some("input-b"))
            .unwrap();
        assert!(Arc::ptr_eq(&stopped, &session.control));
        assert!(stopped.is_stopping());
    }

    #[test]
    fn moving_between_agent_views_transfers_the_original_borrowed_live_recording() {
        let original = recording(31, "live", Some("recording-a")).owner();
        let mut agent = recording(32, "agent", None);
        agent.input_session_id = Some("input-a".into());
        let mut agent = SttInputState {
            active: Some(agent),
            suspended_owner: Some(original.clone()),
        };
        assert!(agent
            .borrowed_owner("agent", Some("input-b"), false)
            .is_none());
        assert!(agent
            .borrowed_owner("agent", Some("input-a"), true)
            .is_none());
        assert_eq!(
            agent.borrowed_owner("agent", Some("input-b"), true),
            Some(original.clone())
        );
        // A closing view may already have requested stop when the new view
        // acquires admission. Its underlying LIVE owner must still transfer.
        agent.active.as_ref().unwrap().control.request_stop();
        assert_eq!(
            agent.borrowed_owner("agent", Some("input-b"), true),
            Some(original)
        );
        agent.suspended_owner = None;
        assert!(agent
            .borrowed_owner("agent", Some("input-b"), true)
            .is_none());
    }

    #[test]
    fn borrowed_live_owner_survives_the_empty_gap_between_two_agent_inputs() {
        let original = recording(33, "live", Some("recording-a")).owner();
        let mut state = SttInputState {
            active: None,
            suspended_owner: None,
        };
        let mut first = recording(34, "agent", None);
        first.input_session_id = Some("input-a".into());
        state.reserve(first, Some(original.clone())).unwrap();
        state.clear_if_matches(34);
        assert_eq!(
            SttStreamState::from_session(state.active.as_ref()).phase,
            SttStreamPhase::Idle
        );
        assert!(state
            .borrowed_owner("agent", Some("input-b"), false)
            .is_none());
        assert_eq!(
            state.borrowed_owner("agent", Some("input-b"), true),
            Some(original.clone())
        );
        let mut next = recording(35, "agent", None);
        next.input_session_id = Some("input-b".into());
        let previous = state.borrowed_owner("agent", Some("input-b"), true);
        state.reserve(next, previous).unwrap();
        state.clear_if_matches(34);
        assert_eq!(state.active.as_ref().unwrap().id, 35);
        assert_eq!(
            state.borrowed_owner("agent", Some("input-c"), true),
            Some(original)
        );
    }

    #[test]
    fn returning_or_starting_a_different_native_input_consumes_the_suspended_owner() {
        let original = recording(36, "live", Some("recording-a")).owner();
        for caller in ["live", "native_agent"] {
            let mut state = SttInputState {
                active: None,
                suspended_owner: Some(original.clone()),
            };
            state
                .reserve(recording(37, caller, Some("recording-a")), None)
                .unwrap();
            state.clear_if_matches(37);
            assert!(state
                .borrowed_owner("agent", Some("input-a"), true)
                .is_none());
        }
    }

    #[test]
    fn duplicate_reservation_preserves_the_active_input_and_its_borrowed_owner() {
        let original = recording(38, "live", Some("recording-a")).owner();
        let mut state = SttInputState {
            active: None,
            suspended_owner: None,
        };
        state
            .reserve(recording(39, "agent", None), Some(original.clone()))
            .unwrap();
        assert!(state
            .reserve(recording(40, "native_agent", None), None)
            .is_err());
        assert_eq!(state.active.as_ref().unwrap().id, 39);
        assert_eq!(state.suspended_owner, Some(original));
    }

    #[test]
    fn text_state_and_error_payloads_keep_the_input_identity() {
        let text = serde_json::to_value(SttEventPayload {
            text: "末尾の発言".into(),
            caller: "agent".into(),
            seq: 7,
            live_session_id: None,
            input_session_id: Some("input-a".into()),
        })
        .unwrap();
        let state = serde_json::to_value(SttStatePayload {
            state: "idle".into(),
            caller: "agent".into(),
            live_session_id: None,
            input_session_id: Some("input-a".into()),
        })
        .unwrap();
        let error = serde_json::to_value(SttInfoPayload {
            message: "マイク入力エラー".into(),
            caller: "agent".into(),
            live_session_id: None,
            input_session_id: Some("input-a".into()),
        })
        .unwrap();
        for payload in [text, state, error] {
            assert_eq!(payload["input_session_id"], "input-a");
            assert_eq!(payload["caller"], "agent");
            assert!(payload.get("live_session_id").is_none());
        }
    }

    #[test]
    fn coherent_state_distinguishes_initialization_capture_and_native_stop() {
        let mut session = Some(recording(20, "live", Some("recording-a")));
        let initializing = SttStreamState::from_session(session.as_ref());
        assert_eq!(initializing.phase, SttStreamPhase::Initializing);
        assert_eq!(initializing.session_id, Some(20));
        assert_eq!(
            initializing
                .owner
                .as_ref()
                .unwrap()
                .live_session_id
                .as_deref(),
            Some("recording-a")
        );
        update_session_phase(&mut session, 20, "listening");
        assert_eq!(
            SttStreamState::from_session(session.as_ref()).phase,
            SttStreamPhase::Listening
        );
        // Direct native stop requests also override a previously listening phase.
        session.as_ref().unwrap().control.request_stop();
        let stopping = SttStreamState::from_session(session.as_ref());
        assert_eq!(stopping.phase, SttStreamPhase::Stopping);
        assert_eq!(stopping.owner, initializing.owner);
        assert_eq!(stopping.session_id, initializing.session_id);
        assert!(!session.as_ref().unwrap().control.wait(Duration::ZERO));
        let idle = SttStreamState::from_session(None);
        assert_eq!(idle.phase, SttStreamPhase::Idle);
        assert!(idle.owner.is_none() && idle.session_id.is_none());
    }

    #[test]
    fn delayed_phase_changes_cannot_relabel_a_replacement_microphone() {
        let mut session = Some(recording(22, "live", Some("recording-b")));
        for phase in ["listening", "idle"] {
            update_session_phase(&mut session, 21, phase);
            let state = SttStreamState::from_session(session.as_ref());
            assert_eq!(state.phase, SttStreamPhase::Initializing);
            assert_eq!(state.session_id, Some(22));
            assert_eq!(
                state.owner.unwrap().live_session_id.as_deref(),
                Some("recording-b")
            );
        }
        update_session_phase(&mut session, 22, "idle");
        // Idle delivery still reserves the owner until cleanup has finished.
        assert_eq!(
            SttStreamState::from_session(session.as_ref()).phase,
            SttStreamPhase::Stopping
        );
    }

    #[test]
    fn borrowing_captures_the_exact_live_owner_before_requesting_stop() {
        let session = SttInputState {
            active: Some(recording(23, "live", Some("recording-a"))),
            suspended_owner: None,
        };
        assert!(session
            .borrowed_owner("agent", Some("input-a"), false)
            .is_none());
        assert!(session.borrowed_owner("live", None, true).is_none());
        let owner = session
            .borrowed_owner("agent", Some("input-a"), true)
            .unwrap();
        session.active.as_ref().unwrap().control.request_stop();
        assert!(session
            .borrowed_owner("agent", Some("input-a"), true)
            .is_none());
        assert_eq!(
            serde_json::to_value(owner).unwrap(),
            serde_json::json!({
                "caller": "live", "live_session_id": "recording-a", "input_session_id": null
            })
        );
        let native = SttInputState {
            active: Some(recording(24, "native_agent", None)),
            suspended_owner: None,
        };
        assert_eq!(
            native
                .borrowed_owner("agent", Some("input-a"), true)
                .unwrap()
                .caller,
            "native_agent"
        );
    }

    #[test]
    fn delayed_live_stop_cannot_stop_a_new_recording_with_the_same_caller() {
        let session = ActiveSttSession {
            id: 3,
            caller: "live".into(),
            live_session_id: Some("new-recording".into()),
            input_session_id: None,
            phase: SttStreamPhase::Initializing,
            control: Arc::new(SttSessionControl::default()),
        };
        assert!(session
            .request_stop_for_owner(Some("live"), Some("old-recording"), None)
            .is_none());
        assert!(!session.control.is_stopping());
        assert!(session
            .request_stop_for_owner(Some("agent"), Some("new-recording"), None)
            .is_none());
        assert!(!session.control.is_stopping());
        let owned = session
            .request_stop_for_owner(Some("live"), Some("new-recording"), None)
            .unwrap();
        assert!(Arc::ptr_eq(&owned, &session.control));
        assert!(owned.is_stopping());
        assert!(!owned.wait(Duration::ZERO));
    }

    #[test]
    fn process_shutdown_still_stops_a_live_recording_with_a_recording_owner() {
        let session = ActiveSttSession {
            id: 4,
            caller: "live".into(),
            live_session_id: Some("recording-a".into()),
            input_session_id: None,
            phase: SttStreamPhase::Initializing,
            control: Arc::new(SttSessionControl::default()),
        };
        assert!(session
            .request_stop_for_owner(None, None, None)
            .unwrap()
            .is_stopping());
    }

    #[test]
    fn live_stop_cannot_stop_a_borrowing_agent_microphone() {
        let session = ActiveSttSession {
            id: 1,
            caller: "agent".into(),
            live_session_id: None,
            input_session_id: None,
            phase: SttStreamPhase::Initializing,
            control: Arc::new(SttSessionControl::default()),
        };
        assert!(session
            .request_stop_for_owner(Some("live"), None, None)
            .is_none());
        assert!(!session.control.is_stopping());
        let owned = session
            .request_stop_for_owner(Some("agent"), None, None)
            .unwrap();
        assert!(Arc::ptr_eq(&owned, &session.control));
        assert!(owned.is_stopping());
        assert!(!owned.wait(Duration::ZERO));
    }

    #[test]
    fn app_shutdown_can_stop_every_caller_without_claiming_completion() {
        let session = ActiveSttSession {
            id: 2,
            caller: "native_agent".into(),
            live_session_id: None,
            input_session_id: None,
            phase: SttStreamPhase::Initializing,
            control: Arc::new(SttSessionControl::default()),
        };
        let control = session.request_stop_for_owner(None, None, None).unwrap();
        assert!(control.is_stopping());
        assert!(!control.wait(Duration::ZERO));
    }
}
