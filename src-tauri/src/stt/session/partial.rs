//! The optional caption model has its own lifetime, independent of finals.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub(super) struct PartialMode {
    // The low bit is enabled; the other bits distinguish disable/re-enable
    // while a load or decode is in flight, even if the final mode is the same.
    version: AtomicU64,
}

impl PartialMode {
    #[cfg(test)]
    pub(super) fn new(enabled: bool) -> Self {
        Self::from_version(u64::from(enabled))
    }

    pub(super) fn from_version(version: u64) -> Self {
        Self {
            version: AtomicU64::new(version),
        }
    }

    pub(super) fn sync_version(&self, version: u64) -> bool {
        self.version
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |previous| {
                (version > previous).then_some(version)
            })
            .is_ok()
    }

    pub(super) fn set_enabled(&self, enabled: bool) -> bool {
        self.version
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |previous| {
                if previous & 1 == u64::from(enabled) {
                    return None;
                }
                Some((previous & !1).wrapping_add(2) | u64::from(enabled))
            })
            .is_ok()
    }

    pub(super) fn snapshot(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    pub(super) fn accepts(&self, version: u64) -> bool {
        version & 1 != 0 && self.snapshot() == version
    }
}

/// Native initialization cannot be interrupted. Once it returns, discard both
/// resources and errors if the recording was stopped during that call.
pub(super) fn initialize_if_active<R, E>(
    active: impl Fn() -> bool,
    load: impl FnOnce() -> Result<R, E>,
) -> Result<Option<R>, E> {
    if !active() {
        return Ok(None);
    }
    let result = load();
    if !active() {
        return Ok(None);
    }
    result.map(Some)
}

pub(super) fn preload_partial<R, E>(
    active: impl Fn() -> bool,
    current_version: impl Fn() -> u64,
    load: impl FnOnce() -> Result<R, E>,
) -> Result<Option<(R, u64)>, E> {
    let version = current_version();
    if version & 1 == 0 {
        return Ok(None);
    }
    initialize_if_active(|| active() && current_version() == version, load)
        .map(|result| result.map(|recognizer| (recognizer, version)))
}

pub(super) struct PartialRecognizer<R> {
    mode: Arc<PartialMode>,
    version: u64,
    recognizer: Option<R>,
}

impl<R> PartialRecognizer<R> {
    pub(super) fn new(mode: Arc<PartialMode>, seed: Option<(R, u64)>) -> Self {
        let (recognizer, version) = match seed {
            Some((recognizer, version)) => (Some(recognizer), version),
            None => (None, mode.snapshot()),
        };
        Self {
            mode,
            version,
            recognizer,
        }
    }

    /// Called on the caption decoder thread, never on the microphone thread.
    pub(super) fn refresh<E>(
        &mut self,
        capturing: impl Fn() -> bool,
        load: impl FnOnce() -> Result<R, E>,
    ) -> Result<Option<(&R, u64)>, E> {
        let version = self.mode.snapshot();
        if self.version != version {
            self.recognizer = None;
            self.version = version;
        }
        if version & 1 == 0 || !capturing() {
            self.recognizer = None;
            return Ok(None);
        }
        if self.recognizer.is_none() {
            let loaded = load();
            // A setting change or stop during model initialization must not
            // retain this model or publish captions under a new generation.
            if !capturing() || !self.mode.accepts(version) {
                return Ok(None);
            }
            self.recognizer = Some(loaded?);
        }
        Ok(self
            .recognizer
            .as_ref()
            .map(|recognizer| (recognizer, version)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Model(Arc<AtomicUsize>);
    impl Model {
        fn load(count: &Arc<AtomicUsize>) -> Result<Self, ()> {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Self(Arc::clone(count)))
        }
    }
    impl Drop for Model {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn stopped_initialization_discards_both_success_and_failure() {
        let count = Arc::new(AtomicUsize::new(0));
        let active = std::sync::atomic::AtomicBool::new(true);
        let initialized = initialize_if_active(
            || active.load(Ordering::SeqCst),
            || {
                let model = Model::load(&count)?;
                active.store(false, Ordering::SeqCst);
                Ok::<_, ()>(model)
            },
        )
        .unwrap();
        assert!(initialized.is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(
            initialize_if_active::<Model, ()>(|| false, || panic!("loaded after stop"))
                .unwrap()
                .is_none()
        );
        active.store(true, Ordering::SeqCst);
        assert!(initialize_if_active::<Model, _>(
            || active.load(Ordering::SeqCst),
            || {
                active.store(false, Ordering::SeqCst);
                Err("canceled native init failed")
            }
        )
        .unwrap()
        .is_none());
        assert!(initialize_if_active::<Model, _>(|| true, || Err("current init failed")).is_err());
    }

    #[test]
    fn startup_caption_load_discards_obsolete_settings_and_preserves_current_errors() {
        let count = Arc::new(AtomicUsize::new(0));
        let settings = PartialMode::new(false);
        assert!(preload_partial::<Model, ()>(
            || true,
            || settings.snapshot(),
            || panic!("loaded disabled caption model")
        )
        .unwrap()
        .is_none());
        settings.set_enabled(true);
        let result = preload_partial(
            || true,
            || settings.snapshot(),
            || {
                let model = Model::load(&count)?;
                settings.set_enabled(false);
                settings.set_enabled(true);
                Ok::<_, ()>(model)
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let obsolete_error = preload_partial::<Model, _>(
            || true,
            || settings.snapshot(),
            || {
                settings.set_enabled(false);
                Err("obsolete caption init failed")
            },
        );
        assert!(obsolete_error.unwrap().is_none());
        settings.set_enabled(true);
        assert!(preload_partial::<Model, _>(
            || true,
            || settings.snapshot(),
            || Err("current caption init failed")
        )
        .is_err());
    }

    #[test]
    fn a_seed_invalidated_before_worker_handoff_is_released_before_decode() {
        let count = Arc::new(AtomicUsize::new(0));
        let settings = PartialMode::new(true);
        let seed =
            preload_partial(|| true, || settings.snapshot(), || Model::load(&count)).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        settings.set_enabled(false);
        settings.set_enabled(true);
        let mode = Arc::new(PartialMode::from_version(settings.snapshot()));
        let mut lease = PartialRecognizer::new(Arc::clone(&mode), seed);
        let mut reloads = 0;
        let (_, version) = lease
            .refresh(
                || true,
                || {
                    reloads += 1;
                    Model::load(&count)
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(version, settings.snapshot());
        assert_eq!(reloads, 1, "the obsolete seed was reused for decoding");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(
            !mode.sync_version(1),
            "a stale config read reverted the mode"
        );
        assert_eq!(mode.snapshot(), settings.snapshot());
    }

    #[test]
    fn final_only_does_not_load_a_caption_model_and_enabling_loads_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(PartialMode::new(false));
        let mut lease = PartialRecognizer::new(Arc::clone(&mode), None);
        for _ in 0..100 {
            assert!(lease
                .refresh(|| true, || Model::load(&count))
                .unwrap()
                .is_none());
        }
        assert_eq!(count.load(Ordering::SeqCst), 0);
        mode.set_enabled(true);
        let loads = AtomicUsize::new(0);
        for _ in 0..100 {
            assert!(lease
                .refresh(
                    || true,
                    || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Model::load(&count)
                    }
                )
                .unwrap()
                .is_some());
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        mode.set_enabled(false);
        assert!(lease
            .refresh(|| true, || Model::load(&count))
            .unwrap()
            .is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn disable_reenable_invalidates_an_in_flight_caption_generation() {
        let mode = PartialMode::new(true);
        let old = mode.snapshot();
        assert!(!mode.set_enabled(true));
        assert!(mode.accepts(old));
        assert!(mode.set_enabled(false));
        assert!(mode.set_enabled(true));
        assert!(!mode.accepts(old));
        assert!(mode.accepts(mode.snapshot()));
    }

    #[test]
    fn model_loaded_during_a_setting_change_is_released_immediately() {
        let count = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(PartialMode::new(true));
        let mut lease = PartialRecognizer::new(Arc::clone(&mode), None);
        let result = lease
            .refresh(
                || true,
                || {
                    let model = Model::load(&count)?;
                    mode.set_enabled(false);
                    mode.set_enabled(true);
                    Ok::<_, ()>(model)
                },
            )
            .unwrap();
        assert!(result.is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(lease
            .refresh(|| true, || Model::load(&count))
            .unwrap()
            .is_some());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_during_initialization_does_not_keep_the_new_model() {
        let count = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(PartialMode::new(true));
        let mut lease = PartialRecognizer::new(mode, None);
        let capturing = std::sync::atomic::AtomicBool::new(true);
        assert!(lease
            .refresh(
                || capturing.load(Ordering::SeqCst),
                || {
                    let model = Model::load(&count)?;
                    capturing.store(false, Ordering::SeqCst);
                    Ok::<_, ()>(model)
                }
            )
            .unwrap()
            .is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_failed_obsolete_load_does_not_abort_final_only_recording() {
        let mode = Arc::new(PartialMode::new(true));
        let mut lease: PartialRecognizer<Model> = PartialRecognizer::new(Arc::clone(&mode), None);
        let result = lease.refresh(
            || true,
            || {
                mode.set_enabled(false);
                Err("obsolete model failed")
            },
        );
        assert!(result.unwrap().is_none());
    }
}
