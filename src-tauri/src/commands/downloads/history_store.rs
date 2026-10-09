//! A locked read/modify/replace transaction for the existing JSON history.

use super::DownloadRecord;
use crate::atomic_file::{atomic_write_with, WriteError, BUFFER_BYTES};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, ErrorKind, Write};
use std::path::PathBuf;

pub(super) struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    // Lock a stable sidecar, never the JSON inode that rename replaces. Every
    // operation opens its own handle; cloned handles can share an OS lock.
    // Keep the sidecar on disk so competing processes always lock the same file.
    fn lock(&self, exclusive: bool) -> Result<File, String> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("lock"))
            .map_err(|e| format!("Failed to open download history lock: {e}"))?;
        if exclusive {
            file.lock()
        } else {
            file.lock_shared()
        }
        .map_err(|e| format!("Failed to lock download history: {e}"))?;
        Ok(file) // Dropping this unique handle releases the lock, also on unwind.
    }

    fn read_locked(&self) -> Result<Vec<DownloadRecord>, String> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("Failed to read download history: {e}")),
        };
        serde_json::from_reader(BufReader::with_capacity(BUFFER_BYTES, file))
            .map_err(|e| format!("Invalid download history JSON: {e}"))
    }

    pub(super) fn read(&self) -> Result<Vec<DownloadRecord>, String> {
        let _lock = self.lock(false)?;
        self.read_locked()
    }

    fn write_locked(&self, records: &[DownloadRecord]) -> Result<(), String> {
        atomic_write_with(&self.path, |file| {
            let mut writer = BufWriter::with_capacity(BUFFER_BYTES, file);
            serde_json::to_writer(&mut writer, records).map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())
        })
        .map_err(|e| match e {
            WriteError::Content(e) => format!("Failed to encode download history: {e}"),
            WriteError::Storage(e) => format!("Failed to write download history: {e}"),
        })
    }

    /// Read the latest records, transform them, and commit while holding the
    /// same exclusive lock. Return the owned result without copying all records.
    /// The closure reports whether anything changed so no-ops avoid disk writes.
    pub(super) fn update(
        &self,
        mutate: impl FnOnce(&mut Vec<DownloadRecord>) -> bool,
    ) -> Result<Vec<DownloadRecord>, String> {
        let _lock = self.lock(true)?;
        let mut records = self.read_locked()?;
        if mutate(&mut records) {
            self.write_locked(&records)?;
        }
        Ok(records)
    }

    /// Explicit clear is allowed to replace even malformed history. Other
    /// mutations must decode successfully and never erase unreadable records.
    pub(super) fn clear(&self) -> Result<(), String> {
        let _lock = self.lock(true)?;
        self.write_locked(&[])
    }
}

#[cfg(test)]
#[path = "history_store_tests.rs"]
mod tests;
