//! A held shortcut owns its delayed status read and subsequent capture request.
use crate::native_agent_state::{CapsuleMode, NativeViewUpdate, SharedState};
use crate::stt::SttStreamState;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct ShortcutIntent {
    held: bool,
    revision: u64,
}
impl ShortcutIntent {
    pub(super) fn press(&mut self) -> Option<u64> {
        if self.held {
            return None;
        }
        self.held = true;
        self.cancel();
        Some(self.revision)
    }
    pub(super) fn release(&mut self) {
        self.held = false;
        self.cancel();
    }
    // Closing a panel invalidates pending work, but a held key must still be
    // released before repeats can arm another capture.
    pub(super) fn cancel(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    fn owns(&self, revision: u64) -> bool {
        self.held && self.revision == revision
    }
}

#[derive(Clone, Copy)]
pub(super) struct ShortcutRequest {
    revision: u64,
    epoch: u64,
}
impl SharedState {
    pub(super) fn press_shortcut(&mut self) -> Option<ShortcutRequest> {
        let revision = self.shortcut.press()?;
        Some(ShortcutRequest {
            revision,
            epoch: self.view_epoch,
        })
    }
    fn owns_shortcut(&self, request: ShortcutRequest) -> bool {
        self.shortcut.owns(request.revision) && self.view_epoch == request.epoch
    }
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn finish_released_capture(
        &mut self,
        input_id: &str,
        expected: u64,
        current: u64,
    ) -> bool {
        expected == current
            && self.stop_requested
            && self.mode == Some(CapsuleMode::Listening)
            && self.capture.finish(input_id)
    }
    pub(super) fn reserve_listening_capture<T>(
        &self,
        input_id: &str,
        reserve: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        if self.mode != Some(CapsuleMode::Listening) || self.stop_requested {
            return Err("この音声入力は終了しています".into());
        }
        self.capture.reserve_if_current(input_id, reserve)
    }
}

pub(super) struct ShortcutUpdate {
    pub(super) view: NativeViewUpdate,
    pub(super) start: bool,
}

/// Run on a blocking worker. Never hold native state while waiting on STT;
/// revalidate under that same native lock before changing UI or capture intent.
pub(super) fn prepare_capture(
    state: &Mutex<SharedState>,
    request: ShortcutRequest,
    input_id: String,
    prompt: &str,
    read_status: impl FnOnce() -> Result<SttStreamState, String>,
) -> Option<ShortcutUpdate> {
    if !state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .owns_shortcut(request)
    {
        return None;
    }
    let status = read_status();
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    if !state.owns_shortcut(request) {
        return None;
    }
    let message = match status {
        Ok(status) if status.owner.is_some() => {
            if status
                .owner
                .as_ref()
                .is_some_and(|owner| owner.caller == "native_agent")
            {
                return None;
            }
            Some("ほかの音声入力が動作中です".to_owned())
        }
        Err(error) => Some(error),
        _ => None,
    };
    state.cancel_stream();
    if let Some(message) = message {
        return Some(ShortcutUpdate {
            view: state.notice_view(&message),
            start: false,
        });
    }
    state.capture.begin(input_id);
    state.stop_requested = false;
    state.finals_accumulated.clear();
    state.current_speech.clear();
    state.result_accumulated.clear();
    state.invalidate_view();
    Some(ShortcutUpdate {
        view: state.prepare_view(CapsuleMode::Listening, prompt.into()),
        start: true,
    })
}

#[cfg(test)]
#[path = "native_shortcut/tests.rs"]
mod tests;
