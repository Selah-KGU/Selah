//! Ownership of one native-shortcut microphone intent, independent of UI mode.
#[derive(Default)]
pub(crate) struct NativeCapture {
    input_id: Option<String>,
    revision: u64,
}

impl NativeCapture {
    pub(crate) fn begin(&mut self, input_id: String) {
        self.revision = self.revision.wrapping_add(1);
        self.input_id = Some(input_id);
    }
    #[cfg(any(target_os = "windows", test))]
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn id(&self) -> Option<&str> {
        self.input_id.as_deref()
    }
    pub(crate) fn owns(&self, input_id: Option<&str>) -> bool {
        input_id.is_some_and(|id| !id.is_empty()) && self.input_id.as_deref() == input_id
    }
    /// Caller holds the native state lock through this short reservation.
    /// Never perform UI work, emit events, or wait for teardown in `reserve`.
    pub(crate) fn reserve_if_current<T>(
        &self,
        input_id: &str,
        reserve: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        if !self.owns(Some(input_id)) {
            return Err("この音声入力は終了しています".into());
        }
        reserve()
    }
    pub(crate) fn finish(&mut self, input_id: &str) -> bool {
        if !self.owns(Some(input_id)) {
            return false;
        }
        self.revision = self.revision.wrapping_add(1);
        self.input_id = None;
        true
    }
    pub(crate) fn cancel(&mut self) -> Option<String> {
        self.revision = self.revision.wrapping_add(1);
        self.input_id.take()
    }
}

/// Keep the UI intent alive until STT has reserved this exact input.
pub(crate) fn with_capture_owner<T>(
    input_id: &str,
    reserve: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    #[cfg(target_os = "macos")]
    {
        crate::macos_native_agent::with_capture_owner(input_id, reserve)
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_native_agent::with_capture_owner(input_id, reserve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_old_terminal_event_cannot_finish_the_replacement_input() {
        let mut input = NativeCapture::default();
        input.begin("a".into());
        let old_revision = input.revision();
        input.begin("b".into());
        assert_ne!(input.revision(), old_revision);
        assert!(!input.owns(Some("a")));
        assert!(!input.finish("a"));
        assert_eq!(input.id(), Some("b"));
        assert!(input.finish("b"));
        assert!(!input.finish("b"));
    }
    #[test]
    fn missing_ids_and_events_after_cancel_are_never_admitted() {
        let mut input = NativeCapture::default();
        assert!(!input.owns(None));
        input.begin("a".into());
        assert!(!input.owns(None));
        assert_eq!(input.cancel().as_deref(), Some("a"));
        assert!(!input.owns(Some("a")));
        assert!(!input.owns(None));
    }
    #[test]
    fn cancellation_and_replacement_reject_a_queued_start_before_it_touches_stt() {
        let state = std::sync::Mutex::new(NativeCapture::default());
        state.lock().unwrap().begin("a".into());
        let mut called = false;
        {
            let mut capture = state.lock().unwrap();
            capture.cancel();
            capture.begin("b".into());
            assert!(capture
                .reserve_if_current("a", || {
                    called = true;
                    Ok(())
                })
                .is_err());
            assert!(!called);
            assert_eq!(
                capture.reserve_if_current("b", || {
                    // The native intent cannot be replaced during STT reservation.
                    assert!(matches!(
                        state.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    Ok(42)
                }),
                Ok(42)
            );
            assert_eq!(capture.id(), Some("b"));
        }
        state.lock().unwrap().cancel();
        assert!(state
            .lock()
            .unwrap()
            .reserve_if_current("b", || {
                called = true;
                Ok(())
            })
            .is_err());
        assert!(!called);
    }

    #[test]
    fn empty_identity_never_owns_events_or_admits_a_microphone() {
        let mut capture = NativeCapture::default();
        capture.begin(String::new());
        assert!(!capture.owns(Some("")));
        assert!(!capture.finish(""));
        assert!(capture
            .reserve_if_current::<()>("", || panic!("empty input admitted"))
            .is_err());
    }
}
