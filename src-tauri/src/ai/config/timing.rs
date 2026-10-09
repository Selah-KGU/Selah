//! LIVE snapshots read only an atomic number; config file IO is serialized here.
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Mutex,
};

pub(super) struct Timing {
    minutes: AtomicI64,
    revision: Mutex<u64>,
}

impl Timing {
    pub(super) const fn new() -> Self {
        Self {
            minutes: AtomicI64::new(5),
            revision: Mutex::new(0),
        }
    }

    pub(super) fn minutes(&self) -> i64 {
        self.minutes.load(Ordering::Acquire)
    }

    fn publish(&self, minutes: u32) {
        self.minutes
            .store(i64::from(minutes.max(5)), Ordering::Release);
    }

    /// One non-secret file read before a recording starts, outside LIVE/storage.
    pub(super) fn refresh(&self, read: impl FnOnce() -> u32) {
        let mut revision = self.revision.lock().unwrap_or_else(|e| e.into_inner());
        self.publish(read());
        *revision = revision.wrapping_add(1);
    }

    /// Capture file contents and their revision together, without holding this
    /// gate across keychain access or any later model work.
    pub(super) fn read<R>(&self, read: impl FnOnce() -> R) -> (u64, R) {
        let revision = self.revision.lock().unwrap_or_else(|e| e.into_inner());
        (*revision, read())
    }

    /// Full config reads may outlive a save or keychain lookup. They cannot
    /// restore an interval captured before a successful newer file write.
    pub(super) fn observe(&self, revision: u64, minutes: u32) {
        let current = self.revision.lock().unwrap_or_else(|e| e.into_inner());
        if *current == revision {
            self.publish(minutes);
        }
    }

    pub(super) fn commit<E>(
        &self,
        minutes: u32,
        write: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        self.commit_if(None, minutes, write).map(|_| ())
    }

    /// Older config migration writes are skipped after a newer successful save.
    pub(super) fn commit_if<E>(
        &self,
        expected: Option<u64>,
        minutes: u32,
        write: impl FnOnce() -> Result<(), E>,
    ) -> Result<bool, E> {
        let mut revision = self.revision.lock().unwrap_or_else(|e| e.into_inner());
        if expected.is_some_and(|expected| expected != *revision) {
            return Ok(false);
        }
        write()?;
        *revision = revision.wrapping_add(1);
        self.publish(minutes);
        Ok(true)
    }

    #[cfg(test)]
    pub(super) fn hold_io<R>(&self, work: impl FnOnce() -> R) -> R {
        let _io = self.revision.lock().unwrap_or_else(|e| e.into_inner());
        work()
    }
}

#[cfg(test)]
#[path = "timing/tests.rs"]
mod tests;
