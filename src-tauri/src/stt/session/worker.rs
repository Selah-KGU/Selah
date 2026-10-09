use super::super::*;
use super::partial::{PartialMode, PartialRecognizer};
use super::queue::{
    lock_last_partial, next_stt_event_seq, DecodePanicGuard, LastPartialText, SttDecodeInbox,
    SttDecodeJob, SttDecodeLane,
};

#[derive(Clone)]
pub(super) struct SttDecodeContext {
    pub(super) app: tauri::AppHandle,
    pub(super) caller: String,
    pub(super) live_session_id: Option<String>,
    pub(super) input_session_id: Option<String>,
    pub(super) control: Arc<SttSessionControl>,
}

impl SttDecodeContext {
    fn capturing(&self) -> bool {
        !self.control.is_stopping() && !STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
    }
}

pub(super) struct LivePartialSetup {
    pub(super) model: SttModelInfo,
    pub(super) config: SttConfig,
    pub(super) recognizer: Option<(OfflineRecognizer, u64)>,
}

pub(super) struct SttDecodeWorker {
    partial_inbox: Arc<SttDecodeInbox>,
    final_inbox: Arc<SttDecodeInbox>,
    joins: Vec<std::thread::JoinHandle<()>>,
    failed: Arc<AtomicBool>,
    partial_mode: Arc<PartialMode>,
}

impl SttDecodeWorker {
    pub(super) fn spawn(
        context: SttDecodeContext,
        recognizer: OfflineRecognizer,
        live_partial: Option<LivePartialSetup>,
        partial_mode: Arc<PartialMode>,
    ) -> Result<Self, String> {
        let failed = Arc::new(AtomicBool::new(false));
        let last_partial = Arc::new(Mutex::new(LastPartialText {
            text: String::new(),
            seq: 0,
        }));
        if let Some(partial) = live_partial {
            return Self::spawn_split(
                context,
                recognizer,
                partial,
                failed,
                last_partial,
                partial_mode,
            );
        }
        let mode_for_thread = Arc::clone(&partial_mode);
        let inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let inbox_thread = Arc::clone(&inbox);
        let join = std::thread::Builder::new()
            .name("stt-decode".into())
            .spawn(move || {
                decode_worker_loop(
                    context,
                    recognizer,
                    inbox_thread,
                    SttDecodeLane::Combined,
                    last_partial,
                    mode_for_thread,
                )
            })
            .map_err(|err| format!("音声認識スレッドの起動に失敗しました: {}", err))?;
        Ok(Self {
            partial_inbox: Arc::clone(&inbox),
            final_inbox: inbox,
            joins: vec![join],
            failed,
            partial_mode,
        })
    }

    fn spawn_split(
        context: SttDecodeContext,
        final_recognizer: OfflineRecognizer,
        partial: LivePartialSetup,
        failed: Arc<AtomicBool>,
        last_partial: Arc<Mutex<LastPartialText>>,
        partial_mode: Arc<PartialMode>,
    ) -> Result<Self, String> {
        let partial_inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let final_inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let partial_for_thread = Arc::clone(&partial_inbox);
        let final_for_thread = Arc::clone(&final_inbox);
        let partial_context = context.clone();
        let partial_last = Arc::clone(&last_partial);
        let mode_for_thread = Arc::clone(&partial_mode);
        let partial_join = std::thread::Builder::new()
            .name("stt-decode-partial".into())
            .spawn(move || {
                partial_worker_loop(
                    partial_context,
                    partial,
                    partial_for_thread,
                    partial_last,
                    mode_for_thread,
                )
            })
            .map_err(|err| format!("音声認識スレッドの起動に失敗しました: {}", err))?;
        let mode_for_final = Arc::clone(&partial_mode);
        let final_join = match std::thread::Builder::new()
            .name("stt-decode-final".into())
            .spawn(move || {
                decode_worker_loop(
                    context,
                    final_recognizer,
                    final_for_thread,
                    SttDecodeLane::Final,
                    last_partial,
                    mode_for_final,
                )
            }) {
            Ok(join) => join,
            Err(err) => {
                partial_inbox.push(SttDecodeJob::Shutdown);
                let _ = partial_join.join();
                return Err(format!("音声認識スレッドの起動に失敗しました: {}", err));
            }
        };
        // Apply the seed's generation before any audio, including in silence.
        partial_inbox.push(SttDecodeJob::ConfigurePartial);
        Ok(Self {
            partial_inbox,
            final_inbox,
            joins: vec![partial_join, final_join],
            failed,
            partial_mode,
        })
    }

    pub(super) fn sync_partial_version(&self, version: u64) {
        if !self.partial_mode.sync_version(version) {
            return;
        }
        // Wake an idle partial decoder so disabling frees its model even in
        // silence. Agent uses one required final model, so only prune captions.
        if self.lanes_are_split() {
            self.partial_inbox.push(SttDecodeJob::ConfigurePartial);
        } else if version & 1 == 0 {
            self.partial_inbox.discard_partials();
        }
    }

    pub(super) fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    fn lanes_are_split(&self) -> bool {
        !Arc::ptr_eq(&self.partial_inbox, &self.final_inbox)
    }

    pub(super) fn push_partial(&self, samples: Vec<f32>) {
        if samples.is_empty() || self.failed() {
            return;
        }
        self.partial_inbox.push(SttDecodeJob::Partial {
            seq: next_stt_event_seq(),
            version: self.partial_mode.snapshot(),
            samples,
        });
    }

    pub(super) fn push_final(&self, samples: Vec<f32>) {
        if samples.is_empty() || self.failed() {
            return;
        }
        self.final_inbox.push(SttDecodeJob::Final {
            seq: next_stt_event_seq(),
            samples,
        });
    }
}

impl Drop for SttDecodeWorker {
    fn drop(&mut self) {
        // Captions are no longer useful once the microphone stops. Keep all
        // finals, including the flushed tail, and avoid another partial decode.
        self.partial_mode.set_enabled(false);
        finish_decode_lanes(&self.partial_inbox, &self.final_inbox, self.failed());
        for join in self.joins.drain(..) {
            let _ = join.join();
        }
    }
}

pub(super) fn finish_decode_lanes(
    partials: &SttDecodeInbox,
    finals: &SttDecodeInbox,
    failed: bool,
) {
    partials.discard_partials();
    if failed {
        partials.discard_pending();
        finals.discard_pending();
    }
    partials.push(SttDecodeJob::Shutdown);
    if !std::ptr::eq(partials, finals) {
        finals.push(SttDecodeJob::Shutdown);
    }
}

fn partial_worker_loop(
    context: SttDecodeContext,
    setup: LivePartialSetup,
    inbox: Arc<SttDecodeInbox>,
    last_partial: Arc<Mutex<LastPartialText>>,
    mode: Arc<PartialMode>,
) {
    let _panic_guard = DecodePanicGuard {
        failed: Arc::clone(&inbox.failed),
    };
    let mut lease = PartialRecognizer::new(Arc::clone(&mode), setup.recognizer);
    loop {
        if inbox.failed.load(Ordering::SeqCst) {
            break;
        }
        let job = inbox.pop();
        if matches!(job, SttDecodeJob::Shutdown) {
            break;
        }
        let request_version = mode.snapshot();
        if matches!(&job, SttDecodeJob::Partial { version, .. } if *version != request_version) {
            continue;
        }
        let loaded = lease.refresh(
            || context.capturing() && current_partial_version() == mode.snapshot(),
            || {
                let init = create_recognizer_for_config(&setup.model, &setup.config)?;
                if let Some(fallback) = init.fallback_from.as_deref().filter(|_| {
                    context.capturing()
                        && mode.accepts(request_version)
                        && current_partial_version() == request_version
                }) {
                    emit_info(
                        &context.app,
                        stt_fallback_message(fallback),
                        &context.caller,
                        context.live_session_id.as_deref(),
                        context.input_session_id.as_deref(),
                    );
                }
                Ok::<_, String>(init.recognizer)
            },
        );
        let (recognizer, version) = match loaded {
            Ok(Some(loaded)) => loaded,
            Ok(None) => continue,
            Err(error) => {
                emit_error(
                    &context.app,
                    format!("リアルタイム字幕の起動に失敗しました: {error}"),
                    &context.caller,
                    context.live_session_id.as_deref(),
                    context.input_session_id.as_deref(),
                );
                inbox.failed.store(true, Ordering::SeqCst);
                break;
            }
        };
        let SttDecodeJob::Partial { seq, samples, .. } = job else {
            continue;
        };
        let text = match decode_samples_safely(recognizer, &samples) {
            Ok(text) => text,
            Err(()) => {
                inbox.failed.store(true, Ordering::SeqCst);
                break;
            }
        };
        if !context.capturing() || !mode.accepts(version) || current_partial_version() != version {
            continue;
        }
        let mut seen = lock_last_partial(&last_partial);
        if text.is_empty() || seq < seen.seq || text == seen.text {
            continue;
        }
        seen.text = text.clone();
        seen.seq = seq;
        drop(seen);
        emit_partial(
            &context.app,
            text,
            &context.caller,
            seq,
            context.live_session_id.as_deref(),
            context.input_session_id.as_deref(),
        );
    }
}

fn decode_worker_loop(
    context: SttDecodeContext,
    recognizer: OfflineRecognizer,
    inbox: Arc<SttDecodeInbox>,
    lane: SttDecodeLane,
    last_partial: Arc<Mutex<LastPartialText>>,
    partial_mode: Arc<PartialMode>,
) {
    let _panic_guard = DecodePanicGuard {
        failed: Arc::clone(&inbox.failed),
    };
    let mut final_gate = FinalTranscriptGate::default();
    loop {
        if inbox.failed.load(Ordering::SeqCst) {
            break;
        }
        match inbox.pop() {
            SttDecodeJob::Shutdown => break,
            SttDecodeJob::ConfigurePartial => continue,
            SttDecodeJob::Partial {
                seq,
                samples,
                version: mode_version,
            } => {
                if matches!(lane, SttDecodeLane::Final) {
                    log::warn!("[stt] final decoder received a partial; dropping");
                    continue;
                }
                if !context.capturing()
                    || !partial_mode.accepts(mode_version)
                    || current_partial_version() != mode_version
                {
                    continue;
                }
                let text = match decode_samples_safely(&recognizer, &samples) {
                    Ok(text) => text,
                    Err(()) => {
                        inbox.failed.store(true, Ordering::SeqCst);
                        break;
                    }
                };
                if !context.capturing()
                    || !partial_mode.accepts(mode_version)
                    || current_partial_version() != mode_version
                {
                    continue;
                }
                let mut seen = lock_last_partial(&last_partial);
                // An older partial can finish after a newer final. Do not let
                // it overwrite the text the next partial is compared against.
                if text.is_empty() || seq < seen.seq || text == seen.text {
                    continue;
                }
                seen.text = text.clone();
                seen.seq = seq;
                drop(seen);
                emit_partial(
                    &context.app,
                    text,
                    &context.caller,
                    seq,
                    context.live_session_id.as_deref(),
                    context.input_session_id.as_deref(),
                );
            }
            SttDecodeJob::Final { seq, samples } => {
                let text = match decode_samples_safely(&recognizer, &samples) {
                    Ok(text) => text,
                    Err(()) => {
                        inbox.failed.store(true, Ordering::SeqCst);
                        break;
                    }
                };
                if text.is_empty() {
                    continue;
                }
                {
                    let mut seen = lock_last_partial(&last_partial);
                    if seq >= seen.seq {
                        seen.text = text.clone();
                        seen.seq = seq;
                    }
                }
                emit_final_once(
                    &context.app,
                    text,
                    &context.caller,
                    seq,
                    &mut final_gate,
                    context.live_session_id.as_deref(),
                    context.input_session_id.as_deref(),
                );
            }
        }
    }
}

fn decode_samples_safely(recognizer: &OfflineRecognizer, samples: &[f32]) -> Result<String, ()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_samples(recognizer, TARGET_SAMPLE_RATE, samples)
    })) {
        Ok(text) => Ok(text),
        Err(_) => {
            log::error!("[stt] recognizer panicked; stopping recognition");
            Err(())
        }
    }
}

pub(super) fn drain_vad_segments(vad: &VoiceActivityDetector, worker: &SttDecodeWorker) -> bool {
    let mut drained = false;
    while !vad.is_empty() {
        if let Some(segment) = vad.front() {
            let samples = segment.samples().to_vec();
            drop(segment);
            worker.push_final(samples);
        }
        vad.pop();
        drained = true;
    }
    drained
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct TrackedModel(Arc<AtomicUsize>);
    impl TrackedModel {
        fn load(count: &Arc<AtomicUsize>) -> Result<Self, ()> {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Self(Arc::clone(count)))
        }
    }
    impl Drop for TrackedModel {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn disabling_captions_while_silent_releases_only_the_optional_model() {
        let count = Arc::new(AtomicUsize::new(0));
        let required_final = TrackedModel::load(&count).unwrap();
        let caption = TrackedModel::load(&count).unwrap();
        let failed = Arc::new(AtomicBool::new(false));
        let partials = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let finals = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let mode = Arc::new(PartialMode::new(true));
        let partial_mode = Arc::clone(&mode);
        let partial_inbox = Arc::clone(&partials);
        let caption_count = Arc::clone(&count);
        let loads = Arc::new(AtomicUsize::new(0));
        let caption_loads = Arc::clone(&loads);
        let (applied, updates) = mpsc::channel();
        let caption_worker = std::thread::spawn(move || {
            let mut lease = PartialRecognizer::new(partial_mode, Some((caption, 1)));
            loop {
                match partial_inbox.pop() {
                    SttDecodeJob::Shutdown => break,
                    SttDecodeJob::ConfigurePartial => {
                        lease
                            .refresh(
                                || true,
                                || {
                                    caption_loads.fetch_add(1, Ordering::SeqCst);
                                    TrackedModel::load(&caption_count)
                                },
                            )
                            .unwrap();
                        applied.send(caption_count.load(Ordering::SeqCst)).unwrap();
                    }
                    other => panic!("unexpected caption job: {other:?}"),
                }
            }
        });
        let (committed, transcript) = mpsc::channel();
        let final_inbox = Arc::clone(&finals);
        let final_worker = std::thread::spawn(move || {
            let _model = required_final;
            loop {
                match final_inbox.pop() {
                    SttDecodeJob::Final { seq, .. } => committed.send(seq).unwrap(),
                    SttDecodeJob::Shutdown => break,
                    other => panic!("unexpected final job: {other:?}"),
                }
            }
        });
        let worker = SttDecodeWorker {
            partial_inbox: partials,
            final_inbox: finals,
            joins: vec![caption_worker, final_worker],
            failed,
            partial_mode: mode,
        };
        assert_eq!(count.load(Ordering::SeqCst), 2);
        worker.sync_partial_version(2);
        assert_eq!(updates.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        worker.final_inbox.push(SttDecodeJob::Final {
            seq: 1,
            samples: vec![1.0],
        });
        assert_eq!(transcript.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        worker.sync_partial_version(5);
        assert_eq!(updates.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        // Both saves happened between capture polls: enabled is still true,
        // but the old model must be released and reloaded in silence.
        worker.sync_partial_version(9);
        assert_eq!(updates.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        worker.sync_partial_version(5);
        assert_eq!(worker.partial_mode.snapshot(), 9);
        worker.final_inbox.push(SttDecodeJob::Final {
            seq: 2,
            samples: vec![2.0],
        });
        assert_eq!(transcript.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        drop(worker);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn closing_split_workers_waits_for_all_delayed_finals() {
        // Exercise the production Drop/join path with a decoder held in flight;
        // no microphone or downloaded recognizer is needed for this race.
        let failed = Arc::new(AtomicBool::new(false));
        let partial = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let finals = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let (release, blocked) = mpsc::channel();
        let (committed, transcript) = mpsc::channel();
        let partial_worker = Arc::clone(&partial);
        let partial_join = std::thread::spawn(move || loop {
            match partial_worker.pop() {
                SttDecodeJob::Partial { .. } => {}
                SttDecodeJob::Shutdown => break,
                other => panic!("unexpected partial job: {other:?}"),
            }
        });
        let final_worker = Arc::clone(&finals);
        let final_join = std::thread::spawn(move || {
            blocked.recv().unwrap();
            loop {
                match final_worker.pop() {
                    SttDecodeJob::Final { seq, .. } => committed.send(seq).unwrap(),
                    SttDecodeJob::Shutdown => break,
                    other => panic!("unexpected final job: {other:?}"),
                }
            }
        });
        for seq in 1..=3 {
            finals.push(SttDecodeJob::Final {
                seq,
                samples: vec![seq as f32],
            });
        }
        partial.push(SttDecodeJob::Partial {
            version: 1,
            seq: 4,
            samples: vec![0.0],
        });
        let worker = SttDecodeWorker {
            partial_inbox: partial,
            final_inbox: finals,
            joins: vec![partial_join, final_join],
            failed,
            partial_mode: Arc::new(PartialMode::new(true)),
        };
        let control = Arc::new(SttSessionControl::default());
        control.request_stop();
        let completion = Arc::clone(&control);
        let (closing, close_started) = mpsc::channel();
        let close = std::thread::spawn(move || {
            closing.send(()).unwrap();
            drop(worker);
            completion.finish();
        });
        close_started.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!control.wait(Duration::ZERO));
        release.send(()).unwrap();
        assert!(control.wait(Duration::from_secs(5)));
        assert_eq!(transcript.try_iter().collect::<Vec<_>>(), vec![1, 2, 3]);
        close.join().unwrap();
    }
}
