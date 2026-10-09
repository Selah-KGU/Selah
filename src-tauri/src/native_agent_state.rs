use crate::latest_ui_mailbox::CurrentUiValue;
use crate::native_agent_events::{StreamEvent, StreamOwner};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CapsuleMode {
    Listening,
    Processing,
    Result,
    Notice,
}

#[derive(Default)]
pub(super) struct SharedState {
    pub(super) mode: Option<CapsuleMode>,
    pub(super) capture: crate::native_capture::NativeCapture,
    pub(super) shortcut: crate::native_shortcut::ShortcutIntent,
    pub(super) view_epoch: u64,
    text_revision: Arc<AtomicU64>,
    pub(super) active_stream: Option<Arc<StreamOwner>>,
    pub(super) stop_requested: bool,
    /// All VAD-finalized segments so far, joined with separators.
    pub(super) finals_accumulated: String,
    /// The current in-flight partial (the latest segment not yet finalized).
    pub(super) current_speech: String,
    pub(super) result_accumulated: String,
    #[cfg(target_os = "windows")]
    pub(super) event_listeners: Vec<tauri::EventId>,
}

#[derive(Clone)]
pub(super) struct NativeViewLease {
    revision: u64,
    current_revision: Arc<AtomicU64>,
}

impl CurrentUiValue for NativeViewLease {
    fn is_current(&self) -> bool {
        self.current_revision.load(Ordering::Relaxed) == self.revision
    }
}

pub(super) struct NativeViewUpdate {
    pub(super) mode: CapsuleMode,
    pub(super) text: String,
    pub(super) epoch: u64,
    pub(super) lease: NativeViewLease,
}

impl CurrentUiValue for NativeViewUpdate {
    fn is_current(&self) -> bool {
        self.lease.is_current()
    }
}

#[cfg(any(target_os = "windows", test))]
pub(super) enum NativeViewRequest {
    Update(NativeViewUpdate),
    Close {
        lease: NativeViewLease,
        immediate: bool,
    },
}
#[cfg(any(target_os = "windows", test))]
impl CurrentUiValue for NativeViewRequest {
    fn is_current(&self) -> bool {
        match self {
            Self::Update(view) => view.is_current(),
            Self::Close { lease, .. } => lease.is_current(),
        }
    }
}

pub(super) struct StreamCompletion {
    pub(super) view: NativeViewUpdate,
}

pub(super) struct StreamStart {
    pub(super) owner: Arc<StreamOwner>,
    pub(super) view: NativeViewUpdate,
}

pub(super) struct ClosedView {
    pub(super) input_id: Option<String>,
}

pub(super) struct CaptureFailure {
    pub(super) text: String,
    pub(super) message: String,
    pub(super) view: NativeViewUpdate,
}

impl SharedState {
    /// Consume only this capture, before its eventual idle callback. Recognized
    /// speech survives the error; queued UI work loses the previous lease.
    pub(super) fn fail_capture(&mut self, input_id: &str, message: &str) -> Option<CaptureFailure> {
        if !self.capture.finish(input_id) {
            return None;
        }
        self.stop_requested = false;
        let text = consume_all_speech(self);
        self.cancel_stream();
        let notice = if text.is_empty() {
            message.to_owned()
        } else {
            format!("認識済みの内容を保存しています · {message}")
        };
        Some(CaptureFailure {
            text,
            message: message.to_owned(),
            view: self.notice_view(&notice),
        })
    }

    pub(super) fn complete_capture_recovery(
        &mut self,
        lease: &NativeViewLease,
        message: &str,
        error: Option<&str>,
    ) -> Option<NativeViewUpdate> {
        if !lease.is_current() || self.mode != Some(CapsuleMode::Notice) {
            return None;
        }
        let notice = match error {
            Some(error) => format!("認識済みの内容は保存待ちです · {message} · {error}"),
            None => format!("認識済みの内容を履歴に保存しました · {message}"),
        };
        Some(self.notice_view(&notice))
    }
    #[cfg(any(target_os = "windows", test))]
    pub(super) fn view_lease(&self) -> NativeViewLease {
        NativeViewLease {
            revision: self.text_revision.load(Ordering::Relaxed),
            current_revision: self.text_revision.clone(),
        }
    }
    pub(super) fn set_mode(&mut self, mode: CapsuleMode) {
        if self.mode != Some(mode) {
            self.invalidate_view();
            self.mode = Some(mode);
        }
    }

    pub(super) fn invalidate_view(&mut self) {
        self.view_epoch = self.view_epoch.wrapping_add(1);
        self.text_revision.fetch_add(1, Ordering::Relaxed);
    }

    // Prepare under the same lock as the display-affecting state mutation.
    // A delayed callback cannot acquire a fresh version after its text is stale.
    pub(super) fn prepare_view(&mut self, mode: CapsuleMode, text: String) -> NativeViewUpdate {
        self.set_mode(mode);
        let revision = self
            .text_revision
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        NativeViewUpdate {
            mode,
            text,
            epoch: self.view_epoch,
            lease: NativeViewLease {
                revision,
                current_revision: self.text_revision.clone(),
            },
        }
    }

    pub(super) fn accept_speech(
        &mut self,
        input_id: &str,
        text: Cow<'_, str>,
        is_final: bool,
    ) -> Option<NativeViewUpdate> {
        if !self.capture.owns(Some(input_id))
            || self.mode != Some(CapsuleMode::Listening)
            || text.trim().is_empty()
        {
            return None;
        }
        if is_final {
            append_final_segment(self, &text);
        } else {
            self.current_speech = text.into_owned();
        }
        // Keep accepting queued finals after release while freezing display.
        self.listening_view()
    }
    pub(super) fn speech_ready_view(&mut self, input_id: &str) -> Option<NativeViewUpdate> {
        if !self.capture.owns(Some(input_id))
            || self.mode != Some(CapsuleMode::Listening)
            || self.stop_requested
        {
            return None;
        }
        let text = listening_display_text(self);
        Some(self.prepare_view(
            CapsuleMode::Listening,
            if text.trim().is_empty() {
                "話してください".into()
            } else {
                text
            },
        ))
    }
    pub(super) fn accept_stream(
        &mut self,
        conversation_id: &str,
        request_id: &str,
        event: StreamEvent<'_>,
    ) -> Option<StreamCompletion> {
        match event {
            StreamEvent::Token(text) => {
                self.append_stream_token(conversation_id, request_id, &text);
                None
            }
            StreamEvent::Done => self.complete_stream(conversation_id, request_id, None),
            StreamEvent::Error(message) => {
                self.complete_stream(conversation_id, request_id, Some(&message))
            }
        }
    }

    pub(super) fn listening_view(&mut self) -> Option<NativeViewUpdate> {
        if self.mode != Some(CapsuleMode::Listening) || self.stop_requested {
            return None;
        }
        let text = listening_display_text(self);
        if text.is_empty() {
            return None;
        }
        Some(self.prepare_view(CapsuleMode::Listening, text))
    }

    pub(super) fn notice_view(&mut self, message: &str) -> NativeViewUpdate {
        self.invalidate_view();
        self.prepare_view(CapsuleMode::Notice, message.to_owned())
    }

    pub(super) fn begin_stream(&mut self, conversation_id: String) -> StreamStart {
        let owner = StreamOwner::new(conversation_id);
        self.active_stream = Some(owner.clone());
        self.invalidate_view();
        self.current_speech.clear();
        self.finals_accumulated.clear();
        self.result_accumulated.clear();
        StreamStart {
            owner,
            view: self.prepare_view(CapsuleMode::Processing, String::new()),
        }
    }

    pub(super) fn cancel_stream(&mut self) {
        self.active_stream = None;
    }

    pub(super) fn owns_stream(&self, conversation_id: &str) -> bool {
        self.mode == Some(CapsuleMode::Processing)
            && self
                .active_stream
                .as_ref()
                .is_some_and(|owner| owner.conversation_id() == conversation_id)
    }

    fn owns_request(&self, conversation_id: &str, request_id: &str) -> bool {
        self.owns_stream(conversation_id)
            && self
                .active_stream
                .as_ref()
                .is_some_and(|owner| owner.request_id() == request_id)
    }

    pub(super) fn append_stream_token(
        &mut self,
        conversation_id: &str,
        request_id: &str,
        text: &str,
    ) -> bool {
        if !self.owns_request(conversation_id, request_id) || text.is_empty() {
            return false;
        }
        self.result_accumulated.push_str(text);
        true
    }

    pub(super) fn complete_stream(
        &mut self,
        conversation_id: &str,
        request_id: &str,
        error: Option<&str>,
    ) -> Option<StreamCompletion> {
        if !self.owns_request(conversation_id, request_id) {
            return None;
        }
        self.finish_stream(error)
    }

    pub(super) fn complete_conversation(
        &mut self,
        conversation_id: &str,
        error: Option<&str>,
    ) -> Option<StreamCompletion> {
        if !self.owns_stream(conversation_id) {
            return None;
        }
        self.finish_stream(error)
    }

    fn finish_stream(&mut self, error: Option<&str>) -> Option<StreamCompletion> {
        self.active_stream = None;
        let (mode, text) = if let Some(message) = error {
            self.result_accumulated = message.to_owned();
            (CapsuleMode::Notice, message.to_owned())
        } else {
            let text = self.result_accumulated.trim().to_owned();
            if text.is_empty() {
                (CapsuleMode::Notice, "応答を取得できませんでした".to_owned())
            } else {
                (CapsuleMode::Result, text)
            }
        };
        Some(StreamCompletion {
            view: self.prepare_view(mode, text),
        })
    }

    pub(super) fn close_view(&mut self, expected: Option<&NativeViewLease>) -> Option<ClosedView> {
        if expected.is_some_and(|lease| !lease.is_current()) {
            return None;
        }
        self.shortcut.cancel();
        self.invalidate_view();
        self.mode = None;
        self.stop_requested = false;
        self.finals_accumulated.clear();
        self.current_speech.clear();
        self.result_accumulated.clear();
        self.cancel_stream();
        Some(ClosedView {
            input_id: self.capture.cancel(),
        })
    }
}

pub(super) fn append_final_segment(sh: &mut SharedState, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    if !sh.finals_accumulated.is_empty() {
        let needs_space = !ends_with_cjk(&sh.finals_accumulated) && !starts_with_cjk(trimmed);
        if needs_space {
            sh.finals_accumulated.push(' ');
        }
    }
    sh.finals_accumulated.push_str(trimmed);
    sh.current_speech.clear();
}

pub(super) fn listening_display_text(sh: &SharedState) -> String {
    let mut out = sh.finals_accumulated.clone();
    let partial = sh.current_speech.trim();
    if !partial.is_empty() {
        if !out.is_empty() {
            let needs_space = !ends_with_cjk(&out) && !starts_with_cjk(partial);
            if needs_space {
                out.push(' ');
            }
        }
        out.push_str(partial);
    }
    out
}

pub(super) fn consume_all_speech(sh: &mut SharedState) -> String {
    let partial = std::mem::take(&mut sh.current_speech);
    let mut out = std::mem::take(&mut sh.finals_accumulated);
    if out.is_empty() {
        return trim_owned(partial);
    }
    let partial = partial.trim();
    if !partial.is_empty() {
        if !out.is_empty() {
            let needs_space = !ends_with_cjk(&out) && !starts_with_cjk(&partial);
            if needs_space {
                out.push(' ');
            }
        }
        out.push_str(partial);
    }
    trim_owned(out)
}

fn trim_owned(mut text: String) -> String {
    text.truncate(text.trim_end().len());
    let leading = text.len() - text.trim_start().len();
    if leading > 0 {
        drop(text.drain(..leading));
    }
    text
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x309F   // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF // CJK Unified
        | 0xF900..=0xFAFF // CJK Compat
        | 0xFF00..=0xFFEF // Halfwidth/Fullwidth
    )
}

fn ends_with_cjk(s: &str) -> bool {
    s.chars().next_back().map(is_cjk_char).unwrap_or(false)
}

fn starts_with_cjk(s: &str) -> bool {
    s.chars().next().map(is_cjk_char).unwrap_or(false)
}

#[cfg(test)]
#[path = "native_agent_state/tests.rs"]
mod tests;
