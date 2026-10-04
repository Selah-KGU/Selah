use super::super::*;
use super::queue::{
    lock_last_partial, next_stt_event_seq, DecodePanicGuard, LastPartialText, SttDecodeInbox,
    SttDecodeJob, SttDecodeLane,
};

pub(super) struct SttDecodeWorker {
    partial_inbox: Arc<SttDecodeInbox>,
    final_inbox: Arc<SttDecodeInbox>,
    joins: Vec<std::thread::JoinHandle<()>>,
    failed: Arc<AtomicBool>,
}

impl SttDecodeWorker {
    pub(super) fn spawn(
        app: tauri::AppHandle,
        caller: String,
        recognizer: OfflineRecognizer,
        live_partial_recognizer: Option<OfflineRecognizer>,
    ) -> Result<Self, String> {
        let failed = Arc::new(AtomicBool::new(false));
        let last_partial = Arc::new(Mutex::new(LastPartialText {
            text: String::new(),
            seq: 0,
        }));
        if let Some(partial_recognizer) = live_partial_recognizer {
            return Self::spawn_split(
                app,
                caller,
                recognizer,
                partial_recognizer,
                failed,
                last_partial,
            );
        }

        let inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let inbox_thread = Arc::clone(&inbox);
        let join = std::thread::Builder::new()
            .name("stt-decode".into())
            .spawn(move || {
                decode_worker_loop(
                    app,
                    caller,
                    recognizer,
                    inbox_thread,
                    SttDecodeLane::Combined,
                    last_partial,
                )
            })
            .map_err(|err| format!("音声認識スレッドの起動に失敗しました: {}", err))?;
        Ok(Self {
            partial_inbox: Arc::clone(&inbox),
            final_inbox: inbox,
            joins: vec![join],
            failed,
        })
    }

    fn spawn_split(
        app: tauri::AppHandle,
        caller: String,
        final_recognizer: OfflineRecognizer,
        partial_recognizer: OfflineRecognizer,
        failed: Arc<AtomicBool>,
        last_partial: Arc<Mutex<LastPartialText>>,
    ) -> Result<Self, String> {
        let partial_inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let final_inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let partial_for_thread = Arc::clone(&partial_inbox);
        let final_for_thread = Arc::clone(&final_inbox);
        let partial_app = app.clone();
        let final_caller = caller.clone();
        let partial_last = Arc::clone(&last_partial);
        let partial_join = std::thread::Builder::new()
            .name("stt-decode-partial".into())
            .spawn(move || {
                decode_worker_loop(
                    partial_app,
                    caller,
                    partial_recognizer,
                    partial_for_thread,
                    SttDecodeLane::Partial,
                    partial_last,
                )
            })
            .map_err(|err| format!("音声認識スレッドの起動に失敗しました: {}", err))?;
        let final_join = match std::thread::Builder::new()
            .name("stt-decode-final".into())
            .spawn(move || {
                decode_worker_loop(
                    app,
                    final_caller,
                    final_recognizer,
                    final_for_thread,
                    SttDecodeLane::Final,
                    last_partial,
                )
            }) {
            Ok(join) => join,
            Err(err) => {
                partial_inbox.push(SttDecodeJob::Shutdown);
                let _ = partial_join.join();
                return Err(format!("音声認識スレッドの起動に失敗しました: {}", err));
            }
        };
        Ok(Self {
            partial_inbox,
            final_inbox,
            joins: vec![partial_join, final_join],
            failed,
        })
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
        let discard = STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) || self.failed();
        if discard {
            self.partial_inbox.discard_pending();
            if self.lanes_are_split() {
                self.final_inbox.discard_pending();
            }
        }
        self.partial_inbox.push(SttDecodeJob::Shutdown);
        if self.lanes_are_split() {
            self.final_inbox.push(SttDecodeJob::Shutdown);
        }
        for join in self.joins.drain(..) {
            let _ = join.join();
        }
    }
}

fn decode_worker_loop(
    app: tauri::AppHandle,
    caller: String,
    recognizer: OfflineRecognizer,
    inbox: Arc<SttDecodeInbox>,
    lane: SttDecodeLane,
    last_partial: Arc<Mutex<LastPartialText>>,
) {
    let _panic_guard = DecodePanicGuard {
        failed: Arc::clone(&inbox.failed),
    };
    let mut last_final = String::new();
    loop {
        if inbox.failed.load(Ordering::SeqCst) {
            break;
        }
        match inbox.pop() {
            SttDecodeJob::Shutdown => break,
            SttDecodeJob::Partial { seq, samples } => {
                if matches!(lane, SttDecodeLane::Final) {
                    log::warn!("[stt] final decoder received a partial; dropping");
                    continue;
                }
                if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                    continue;
                }
                let text = match decode_samples_safely(&recognizer, &samples) {
                    Ok(text) => text,
                    Err(()) => {
                        inbox.failed.store(true, Ordering::SeqCst);
                        break;
                    }
                };
                let mut seen = lock_last_partial(&last_partial);
                // An older partial can finish after a newer final. Do not let
                // it overwrite the text the next partial is compared against.
                if text.is_empty() || seq < seen.seq || text == seen.text {
                    continue;
                }
                seen.text = text.clone();
                seen.seq = seq;
                drop(seen);
                emit_partial(&app, text, &caller, seq);
            }
            SttDecodeJob::Final { seq, samples } => {
                if matches!(lane, SttDecodeLane::Partial) {
                    log::warn!("[stt] partial decoder received a final; dropping");
                    continue;
                }
                if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                    continue;
                }
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
                emit_final_deduped(&app, text, &caller, seq, &mut last_final);
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
