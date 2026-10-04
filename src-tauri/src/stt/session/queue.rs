use super::super::*;

static STT_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

pub(in crate::stt) fn next_stt_event_seq() -> u64 {
    STT_EVENT_SEQ.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug)]
pub(in crate::stt) enum SttDecodeJob {
    Partial { seq: u64, samples: Vec<f32> },
    Final { seq: u64, samples: Vec<f32> },
    Shutdown,
}

fn enqueue_stt_decode_job(
    jobs: &mut VecDeque<SttDecodeJob>,
    job: SttDecodeJob,
    prioritize_partials: bool,
) {
    let is_partial = matches!(job, SttDecodeJob::Partial { .. });
    if is_partial {
        // A newer partial supersedes any partial still waiting.
        jobs.retain(|existing| !matches!(existing, SttDecodeJob::Partial { .. }));
        if prioritize_partials {
            // Shared-queue fallback only. Live does not use this path: it has
            // a second recognizer, so an in-flight final cannot block a partial.
            // Finals stay in their own order. The UI ignores an older final
            // that would otherwise wipe the newer partial.
            let insert_at = jobs
                .iter()
                .position(|existing| {
                    matches!(
                        existing,
                        SttDecodeJob::Final { .. } | SttDecodeJob::Shutdown
                    )
                })
                .unwrap_or(jobs.len());
            jobs.insert(insert_at, job);
            return;
        }
    }
    jobs.push_back(job);
}

pub(in crate::stt) struct SttDecodeInbox {
    jobs: Mutex<VecDeque<SttDecodeJob>>,
    cv: Condvar,
    pub(in crate::stt) failed: Arc<AtomicBool>,
    prioritize_partials: bool,
}

impl SttDecodeInbox {
    pub(in crate::stt) fn new(prioritize_partials: bool, failed: Arc<AtomicBool>) -> Self {
        Self {
            jobs: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            failed,
            prioritize_partials,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<SttDecodeJob>> {
        self.jobs.lock().unwrap_or_else(|err| err.into_inner())
    }

    pub(in crate::stt) fn push(&self, job: SttDecodeJob) {
        let mut jobs = self.lock();
        enqueue_stt_decode_job(&mut jobs, job, self.prioritize_partials);
        self.cv.notify_one();
    }

    pub(in crate::stt) fn discard_pending(&self) {
        let mut jobs = self.lock();
        jobs.retain(|job| matches!(job, SttDecodeJob::Shutdown));
        self.cv.notify_one();
    }

    pub(in crate::stt) fn pop(&self) -> SttDecodeJob {
        let mut jobs = self.lock();
        loop {
            if let Some(job) = jobs.pop_front() {
                return job;
            }
            jobs = self.cv.wait(jobs).unwrap_or_else(|err| err.into_inner());
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

#[derive(Clone, Copy)]
pub(in crate::stt) enum SttDecodeLane {
    /// Agent and other short inputs: one recognizer handles both job kinds.
    Combined,
    /// Live partials. A final already being decoded must not block these.
    Partial,
    /// Live finals, kept in arrival order on their own recognizer.
    Final,
}

pub(in crate::stt) struct DecodePanicGuard {
    pub(in crate::stt) failed: Arc<AtomicBool>,
}

impl Drop for DecodePanicGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            log::error!("[stt] recognizer thread panicked; stopping recognition");
            self.failed.store(true, Ordering::SeqCst);
        }
    }
}

pub(in crate::stt) struct LastPartialText {
    pub(in crate::stt) text: String,
    pub(in crate::stt) seq: u64,
}

pub(in crate::stt) fn lock_last_partial(
    last_partial: &Mutex<LastPartialText>,
) -> std::sync::MutexGuard<'_, LastPartialText> {
    last_partial.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
