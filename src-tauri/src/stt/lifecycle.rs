//! A stop request and completion of final decoding are distinct states.
//! Keep the session owned until workers and microphone resources are released.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Condvar, Mutex, MutexGuard,
};
use std::time::Duration;

/// Recording startup and model maintenance share an admission gate. Long
/// maintenance keeps a reservation, not the mutex: new starts fail promptly
/// instead of waiting for an entire download or native model initialization.
#[derive(Default)]
pub(in crate::stt) struct SttLifecycleGate {
    start: Mutex<()>,
    model_busy: AtomicBool,
}

const MODEL_BUSY: &str = "STT モデルを準備中です。完了してからもう一度操作してください";

impl SttLifecycleGate {
    pub(in crate::stt) fn lock_start(&self) -> Result<MutexGuard<'_, ()>, String> {
        let guard = self
            .start
            .lock()
            .map_err(|_| "STT lifecycle lock failed".to_string())?;
        self.check_model_idle()?;
        Ok(guard)
    }

    pub(in crate::stt) fn try_start(&self) -> Result<MutexGuard<'_, ()>, String> {
        let guard = self
            .start
            .try_lock()
            .map_err(|_| "ほかの音声入力を切り替え中です".to_string())?;
        self.check_model_idle()?;
        Ok(guard)
    }

    pub(in crate::stt) fn reserve_model_operation(
        &self,
        available: impl FnOnce() -> Result<(), String>,
    ) -> Result<SttModelOperation<'_>, String> {
        let _guard = self.try_start()?;
        // The caller checks the active recording and shutdown under the same
        // gate used when publishing a new recording into the session registry.
        available()?;
        self.model_busy.store(true, Ordering::SeqCst);
        Ok(SttModelOperation { gate: self })
    }

    fn check_model_idle(&self) -> Result<(), String> {
        if self.model_busy.load(Ordering::SeqCst) {
            Err(MODEL_BUSY.into())
        } else {
            Ok(())
        }
    }
}

pub(in crate::stt) struct SttModelOperation<'a> {
    gate: &'a SttLifecycleGate,
}

impl Drop for SttModelOperation<'_> {
    fn drop(&mut self) {
        self.gate.model_busy.store(false, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub(in crate::stt) struct SttSessionControl {
    stop_requested: AtomicBool,
    finished: Mutex<bool>,
    changed: Condvar,
}

impl SttSessionControl {
    pub(in crate::stt) fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::SeqCst);
    }

    pub(in crate::stt) fn is_stopping(&self) -> bool {
        self.stop_requested.load(Ordering::SeqCst)
    }

    pub(in crate::stt) fn finish(&self) {
        *self.finished.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_all();
    }

    pub(in crate::stt) fn wait(&self, timeout: Duration) -> bool {
        let done = self.finished.lock().unwrap_or_else(|e| e.into_inner());
        let (done, _) = self
            .changed
            .wait_timeout_while(done, timeout, |done| !*done)
            .unwrap_or_else(|e| e.into_inner());
        *done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn maintenance_rejects_starts_and_other_model_work_without_holding_the_start_mutex() {
        let gate = Arc::new(SttLifecycleGate::default());
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let model_gate = Arc::clone(&gate);
        let operation = std::thread::spawn(move || {
            let _reservation = model_gate.reserve_model_operation(|| Ok(())).unwrap();
            entered.send(()).unwrap();
            blocked.recv().unwrap();
        });
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            gate.start.try_lock().is_ok(),
            "maintenance held the start mutex"
        );
        assert!(gate.lock_start().is_err());
        assert!(gate.try_start().is_err());
        assert!(gate
            .reserve_model_operation(|| panic!("overlapping model work was admitted"))
            .is_err());
        release.send(()).unwrap();
        operation.join().unwrap();
        assert!(gate.lock_start().is_ok());
        assert!(gate.reserve_model_operation(|| Ok(())).is_ok());
    }

    #[test]
    fn maintenance_cannot_race_startup_or_a_recording_that_is_still_draining() {
        let gate = SttLifecycleGate::default();
        let startup = gate.lock_start().unwrap();
        assert!(gate
            .reserve_model_operation(|| panic!("read recording state outside the startup gate"))
            .is_err());
        drop(startup);
        let draining = SttSessionControl::default();
        draining.request_stop();
        assert!(gate
            .reserve_model_operation(|| {
                if draining.wait(Duration::ZERO) {
                    Ok(())
                } else {
                    Err("recording still owns the models".into())
                }
            })
            .is_err());
        draining.finish();
        assert!(gate
            .reserve_model_operation(|| {
                assert!(draining.wait(Duration::ZERO));
                Ok(())
            })
            .is_ok());
    }

    #[test]
    fn model_resources_are_released_before_the_reservation_even_on_failure_or_panic() {
        struct Model<'a>(&'a AtomicBool, &'a SttLifecycleGate);
        impl Drop for Model<'_> {
            fn drop(&mut self) {
                assert!(
                    self.1.try_start().is_err(),
                    "start admitted a still-live model"
                );
                assert!(self.0.swap(false, Ordering::SeqCst));
            }
        }
        let gate = SttLifecycleGate::default();
        let live_model = AtomicBool::new(false);
        for panic in [false, true] {
            let result = std::panic::catch_unwind(|| -> Result<(), String> {
                let _reservation = gate.reserve_model_operation(|| {
                    assert!(!live_model.load(Ordering::SeqCst));
                    Ok(())
                })?;
                live_model.store(true, Ordering::SeqCst);
                let _model = Model(&live_model, &gate);
                if panic {
                    panic!("native model operation failed");
                }
                Err("native model operation failed".into())
            });
            assert!(matches!(result, Err(_) | Ok(Err(_))));
            assert!(!live_model.load(Ordering::SeqCst));
            assert!(gate.try_start().is_ok());
        }
    }

    #[test]
    fn stop_request_does_not_mean_the_last_final_has_been_committed() {
        let control = Arc::new(SttSessionControl::default());
        let (release, blocked) = mpsc::channel();
        let (committed, transcript) = mpsc::channel();
        let worker_control = Arc::clone(&control);
        let worker = std::thread::spawn(move || {
            blocked.recv().unwrap();
            committed.send("last sentence").unwrap();
            worker_control.finish();
        });
        control.request_stop();
        assert!(control.is_stopping());
        assert!(!control.wait(Duration::ZERO));
        release.send(()).unwrap();
        assert!(control.wait(Duration::from_secs(2)));
        assert_eq!(transcript.try_recv().unwrap(), "last sentence");
        worker.join().unwrap();
    }

    #[test]
    fn completion_wakes_every_waiter_and_is_safe_to_wait_again() {
        let control = Arc::new(SttSessionControl::default());
        let waiters: Vec<_> = (0..8)
            .map(|_| {
                let control = Arc::clone(&control);
                std::thread::spawn(move || control.wait(Duration::from_secs(2)))
            })
            .collect();
        control.finish();
        for waiter in waiters {
            assert!(waiter.join().unwrap());
        }
        assert!(control.wait(Duration::ZERO));
    }
}
