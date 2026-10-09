//! Serialize storage operations without holding the recording state lock.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[cfg(test)]
use super::LiveTranscriptLine;
use super::{LiveCourseInfo, LiveState, LiveSummaryChunks, LiveTranscriptLines};

const CACHE_DEBOUNCE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default)]
pub(super) struct CacheProgress {
    pub(super) lines: usize,
    pub(super) summaries: usize,
    written_at: Option<Instant>,
}

impl CacheProgress {
    pub(super) fn restored(lines: usize, summaries: usize) -> Self {
        Self {
            lines,
            summaries,
            written_at: None,
        }
    }

    pub(super) fn committed(lines: usize, summaries: usize) -> Self {
        Self {
            lines,
            summaries,
            written_at: Some(Instant::now()),
        }
    }
}

pub(super) struct SavePlan {
    pub(super) session_id: String,
    pub(super) course: LiveCourseInfo,
    pub(super) started_at: chrono::DateTime<chrono::Local>,
    pub(super) lines: LiveTranscriptLines,
    pub(super) summaries: LiveSummaryChunks,
    pub(super) start: usize,
    pub(super) full: bool,
    pub(super) markdown: bool,
}

struct SaveIntent {
    session_id: String,
    force: bool,
}

#[derive(Default)]
struct SaveQueue {
    pending: Option<SaveIntent>,
    running: bool,
}

#[derive(Default)]
pub(super) struct LivePersistence {
    pub(super) gate: Mutex<()>,
    queue: Mutex<SaveQueue>,
    changed: Notify,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SaveOutcome {
    Unchanged,
    Deferred(Duration),
    Written,
}

impl LivePersistence {
    pub(super) fn wake(&self) {
        self.changed.notify_one();
    }

    pub(super) fn schedule(state: &LiveState, force: bool) {
        let Some(session_id) = state.active_session_id() else {
            return;
        };
        Self::schedule_for(state, &session_id, force);
    }

    pub(super) fn schedule_for(state: &LiveState, session_id: &str, force: bool) {
        Self::schedule_with(
            state,
            session_id,
            force,
            CACHE_DEBOUNCE,
            super::cache::persist_plan,
        );
    }

    fn schedule_with(
        state: &LiveState,
        session_id: &str,
        force: bool,
        debounce: Duration,
        write: impl Fn(&SavePlan) -> Result<(), String> + Send + Sync + 'static,
    ) {
        let mut queue = state
            .persistence
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !state.is_session_current(session_id) {
            return;
        }
        let wake = force
            || queue
                .pending
                .as_ref()
                .is_none_or(|intent| intent.session_id != session_id);
        match queue.pending.as_mut() {
            Some(intent) if intent.session_id == session_id => intent.force |= force,
            _ => {
                queue.pending = Some(SaveIntent {
                    session_id: session_id.to_string(),
                    force,
                })
            }
        }
        if queue.running {
            drop(queue);
            if wake {
                state.persistence.changed.notify_one();
            }
            return;
        }
        queue.running = true;
        drop(queue);
        let state = state.clone();
        let write = Arc::new(write);
        tauri::async_runtime::spawn(async move {
            loop {
                let intent = {
                    let mut queue = state
                        .persistence
                        .queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    match queue.pending.take() {
                        Some(intent) => intent,
                        None => {
                            queue.running = false;
                            return;
                        }
                    }
                };
                let storage_state = state.clone();
                let write = Arc::clone(&write);
                let worker_expected = intent.session_id.clone();
                let force = intent.force;
                let result = tokio::task::spawn_blocking(move || {
                    Self::persist_with_window(
                        &storage_state,
                        &worker_expected,
                        force,
                        debounce,
                        |plan| write(plan),
                    )
                })
                .await;
                match result {
                    Ok(Ok(SaveOutcome::Deferred(wait))) => {
                        // Keep the final lines eligible even if no more speech
                        // or summary events arrive during the debounce window.
                        Self::defer(&state, intent, wait).await;
                    }
                    Ok(Err(error)) => {
                        log::warn!("[Live] autosave failed; retrying uncommitted lines: {error}");
                        Self::defer(&state, intent, debounce).await;
                    }
                    Err(error) => {
                        log::error!(
                            "[Live] autosave worker failed; retrying uncommitted lines: {error}"
                        );
                        Self::defer(&state, intent, debounce).await;
                    }
                    _ => {}
                }
            }
        });
    }

    async fn defer(state: &LiveState, intent: SaveIntent, wait: Duration) {
        let should_wait = {
            let mut queue = state
                .persistence
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match queue.pending.as_mut() {
                Some(next) if next.session_id == intent.session_id => {
                    let wait = !next.force;
                    next.force |= intent.force;
                    wait
                }
                None if state.is_session_current(&intent.session_id) => {
                    queue.pending = Some(intent);
                    true
                }
                _ => false,
            }
        };
        if should_wait {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {},
                _ = state.persistence.changed.notified() => {},
            }
        }
    }

    /// The caller runs on a blocking worker. Cancellation/start/finish use the
    /// same gate, so a stale write cannot recreate deleted recording files.
    #[cfg(test)]
    pub(super) fn persist_with(
        state: &LiveState,
        expected: &str,
        force: bool,
        write: impl FnOnce(&SavePlan) -> Result<(), String>,
    ) -> Result<SaveOutcome, String> {
        Self::persist_with_window(state, expected, force, CACHE_DEBOUNCE, write)
    }

    fn persist_with_window(
        state: &LiveState,
        expected: &str,
        force: bool,
        debounce: Duration,
        write: impl FnOnce(&SavePlan) -> Result<(), String>,
    ) -> Result<SaveOutcome, String> {
        Self::persist_with_options(state, expected, force, false, debounce, write)
    }

    pub(super) fn persist_for_exit(
        state: &LiveState,
        expected: &str,
        write: impl FnOnce(&SavePlan) -> Result<(), String>,
    ) -> Result<SaveOutcome, String> {
        Self::persist_with_options(state, expected, true, true, CACHE_DEBOUNCE, write)
    }

    fn persist_with_options(
        state: &LiveState,
        expected: &str,
        force: bool,
        exiting: bool,
        debounce: Duration,
        write: impl FnOnce(&SavePlan) -> Result<(), String>,
    ) -> Result<SaveOutcome, String> {
        let _gate = state
            .persistence
            .gate
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let plan = {
            let guard = state
                .session
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            let Some(session) = guard
                .as_ref()
                .filter(|session| session.session_id == expected)
            else {
                return Ok(SaveOutcome::Unchanged);
            };
            let progress = &session.cache_progress;
            let changed = session.transcript_lines.len() > progress.lines
                || session.summaries.len() > progress.summaries;
            if !changed && (!exiting || session.transcript_lines.is_empty()) {
                return Ok(SaveOutcome::Unchanged);
            }
            if !force {
                if let Some(wait) = progress
                    .written_at
                    .and_then(|time| debounce.checked_sub(time.elapsed()))
                {
                    return Ok(SaveOutcome::Deferred(wait));
                }
            }
            SavePlan {
                session_id: session.session_id.clone(),
                course: session.course.clone(),
                started_at: session.started_at,
                lines: Arc::clone(&session.transcript_lines),
                summaries: Arc::clone(&session.summaries),
                start: progress.lines,
                full: force || session.summaries.len() > progress.summaries,
                // Summarizing/SavingFinal already have a complete pre-AI file;
                // do not overwrite that file with an in-progress placeholder.
                markdown: if exiting {
                    matches!(
                        session.finish_phase,
                        None | Some(
                            super::LiveFinishPhase::Stopping | super::LiveFinishPhase::SavingRecord
                        )
                    )
                } else {
                    session.finish_phase.is_none()
                        && (session.course.is_free_note
                            || session.summaries.len() > progress.summaries)
                },
            }
        };
        write(&plan)?;
        let mut guard = state
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        if let Some(session) = guard
            .as_mut()
            .filter(|session| session.session_id == plan.session_id)
        {
            session.cache_progress = CacheProgress {
                lines: plan.lines.len(),
                summaries: plan.summaries.len(),
                written_at: Some(Instant::now()),
            };
        }
        Ok(SaveOutcome::Written)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::transcript::recording;
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn exit_forces_the_quiet_tail_to_a_full_record_without_waiting_for_debounce() {
        let state = recently_saved_state();
        let outcome = LivePersistence::persist_for_exit(&state, "recording-test", |plan| {
            assert!(plan.full);
            assert!(
                plan.markdown,
                "course without summaries still needs an exit record"
            );
            assert_eq!(plan.lines.len(), 1);
            assert_eq!(plan.lines[0].text, "first");
            Ok(())
        })
        .unwrap();
        assert_eq!(outcome, SaveOutcome::Written);
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            1
        );
    }

    #[test]
    fn exit_does_not_replace_an_existing_pre_ai_record_during_final_summary() {
        for phase in [
            None,
            Some(super::super::LiveFinishPhase::Stopping),
            Some(super::super::LiveFinishPhase::SavingRecord),
            Some(super::super::LiveFinishPhase::Summarizing),
            Some(super::super::LiveFinishPhase::SavingFinal),
        ] {
            let state = state_with_speech();
            {
                let mut guard = state.session.lock().unwrap();
                let session = guard.as_mut().unwrap();
                session.course.is_free_note = true;
                session.finish_phase = phase;
                session.cache_progress = CacheProgress::committed(1, 0);
            }
            LivePersistence::persist_for_exit(&state, "recording-test", |plan| {
                assert!(plan.full);
                assert_eq!(
                    plan.markdown,
                    !matches!(
                        phase,
                        Some(
                            super::super::LiveFinishPhase::Summarizing
                                | super::super::LiveFinishPhase::SavingFinal
                        )
                    )
                );
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn failed_exit_storage_keeps_uncommitted_speech_available_for_retry() {
        let state = recently_saved_state();
        assert!(
            LivePersistence::persist_for_exit(
                &state,
                "recording-test",
                |_| Err("disk full".into())
            )
            .is_err()
        );
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            0
        );
        LivePersistence::persist_for_exit(&state, "recording-test", |plan| {
            assert_eq!(plan.start, 0);
            assert_eq!(plan.lines.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    fn state_with_speech() -> LiveState {
        let state = LiveState::new();
        let mut session = recording();
        session.append_line(LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "first".into(),
        });
        *state.session.lock().unwrap() = Some(session);
        state
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("autosave queue did not progress");
    }

    fn recently_saved_state() -> LiveState {
        let state = state_with_speech();
        state
            .session
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .cache_progress = CacheProgress::committed(0, 0);
        state
    }

    #[tokio::test]
    async fn quiet_tail_is_saved_after_debounce_without_another_speech_event() {
        let state = recently_saved_state();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let written = Arc::clone(&calls);
        let debounce = Duration::from_millis(100);
        assert!(matches!(
            LivePersistence::persist_with_window(
                &state,
                "recording-test",
                false,
                debounce,
                |_| panic!("wrote before debounce")
            ),
            Ok(SaveOutcome::Deferred(_))
        ));
        LivePersistence::schedule_with(&state, "recording-test", false, debounce, move |plan| {
            assert_eq!(plan.lines[0].text, "first");
            written.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        wait_until(|| !state.persistence.queue.lock().unwrap().running).await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            1
        );
    }

    #[tokio::test]
    async fn forced_save_wakes_a_deferred_queue_and_canceled_tail_is_discarded() {
        for cancel in [false, true] {
            let state = recently_saved_state();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let written = Arc::clone(&calls);
            LivePersistence::schedule_with(
                &state,
                "recording-test",
                false,
                Duration::from_secs(60),
                move |plan| {
                    assert!(plan.full, "forced request was lost");
                    written.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                },
            );
            wait_until(|| state.persistence.queue.lock().unwrap().pending.is_some()).await;
            if cancel {
                *state.session.lock().unwrap() = None;
                state.persistence.wake();
            } else {
                LivePersistence::schedule_with(
                    &state,
                    "recording-test",
                    true,
                    Duration::from_secs(60),
                    |_| panic!("second worker spawned"),
                );
            }
            wait_until(|| !state.persistence.queue.lock().unwrap().running).await;
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!cancel)
            );
        }
    }

    #[tokio::test]
    async fn failed_forced_save_retries_a_quiet_recording_without_spinning() {
        let state = state_with_speech();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempted = Arc::clone(&calls);
        let retry = Duration::from_millis(80);
        let failed_at = Mutex::new(None);
        LivePersistence::schedule_with(&state, "recording-test", true, retry, move |plan| {
            let attempt = attempted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(plan.full);
            if attempt == 0 {
                *failed_at.lock().unwrap() = Some(Instant::now());
                Err("transient disk error".into())
            } else {
                assert_eq!(attempt, 1);
                assert!(failed_at.lock().unwrap().unwrap().elapsed() >= retry);
                Ok(())
            }
        });
        wait_until(|| !state.persistence.queue.lock().unwrap().running).await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            1
        );
    }

    #[test]
    fn failed_write_keeps_the_cursor_and_next_request_retries_without_debounce() {
        let state = state_with_speech();
        assert!(
            LivePersistence::persist_with(&state, "recording-test", false, |_| Err(
                "disk full".into()
            ))
            .is_err()
        );
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            0
        );
        assert_eq!(
            LivePersistence::persist_with(&state, "recording-test", false, |plan| {
                assert_eq!(plan.start, 0);
                assert_eq!(plan.lines.len(), 1);
                Ok(())
            })
            .unwrap(),
            SaveOutcome::Written
        );
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            1
        );
        assert_eq!(
            LivePersistence::persist_with(&state, "recording-test", true, |_| panic!(
                "unchanged snapshot rewritten"
            ))
            .unwrap(),
            SaveOutcome::Unchanged
        );
    }

    #[test]
    fn slow_storage_does_not_lock_speech_and_acknowledges_only_its_captured_lines() {
        let state = state_with_speech();
        let writer_state = state.clone();
        let (writing, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            LivePersistence::persist_with(&writer_state, "recording-test", false, |plan| {
                assert_eq!(plan.lines.len(), 1);
                writing.send(()).unwrap();
                blocked.recv().unwrap();
                assert_eq!(plan.lines[0].text, "first");
                Ok(())
            })
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(state.session.try_lock().is_ok());
        state
            .append_line_for_session(
                Some("recording-test"),
                LiveTranscriptLine {
                    at: "10:00:01".into(),
                    text: "second".into(),
                },
            )
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(writer.join().unwrap().unwrap(), SaveOutcome::Written);
        let guard = state.session.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert_eq!(session.transcript_lines.len(), 2);
        assert_eq!(session.cache_progress.lines, 1);
        drop(guard);
        assert_eq!(
            LivePersistence::persist_with(&state, "recording-test", true, |plan| {
                assert_eq!(plan.start, 1);
                assert_eq!(plan.lines[1].text, "second");
                Ok(())
            })
            .unwrap(),
            SaveOutcome::Written
        );
    }

    #[test]
    fn a_queued_old_recording_cannot_write_after_its_replacement() {
        let state = state_with_speech();
        let gate = state.persistence.gate.lock().unwrap();
        let writer_state = state.clone();
        let (attempting, waiting) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            attempting.send(()).unwrap();
            LivePersistence::persist_with(&writer_state, "recording-test", true, |_| {
                panic!("stale recording wrote files")
            })
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut next = recording();
        next.session_id = "replacement".into();
        *state.session.lock().unwrap() = Some(next);
        drop(gate);
        assert_eq!(writer.join().unwrap().unwrap(), SaveOutcome::Unchanged);
        assert_eq!(state.active_session_id().as_deref(), Some("replacement"));
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .cache_progress
                .lines,
            0
        );
    }
}
