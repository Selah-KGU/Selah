//! Atomically replace complete files without exposing staged or partial contents.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) const BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(crate) enum WriteError<E> {
    Content(E),
    Storage(io::Error),
}

struct Replacement {
    file: Option<File>,
    temporary: PathBuf,
    committed: bool,
}
impl Drop for Replacement {
    fn drop(&mut self) {
        // Close before cleanup, including on Windows and during unwinding.
        drop(self.file.take());
        if !self.committed {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}

/// Keep the prior complete file until the replacement has been written and
/// synced. The temporary file lives beside its destination for atomic rename.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes)).map_err(|error| match error {
        WriteError::Content(error) | WriteError::Storage(error) => error,
    })
}

pub(crate) fn atomic_write_with<E>(
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<(), E>,
) -> Result<(), WriteError<E>> {
    let parent = path
        .parent()
        .ok_or_else(|| WriteError::Storage(io::Error::other("missing parent directory")))?;
    let temporary = parent.join(format!(".selah-write-{}.tmp", uuid::Uuid::new_v4()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(WriteError::Storage)?;
    // Ownership begins only after create_new succeeds: never clean up a file
    // this operation did not create. Failed/panicked writers leave the old file.
    let mut replacement = Replacement {
        file: Some(file),
        temporary,
        committed: false,
    };
    let file = replacement.file.as_mut().unwrap();
    write(file).map_err(WriteError::Content)?;
    file.sync_all().map_err(WriteError::Storage)?;
    drop(replacement.file.take());
    std::fs::rename(&replacement.temporary, path).map_err(WriteError::Storage)?;
    replacement.committed = true;
    Ok(())
}

#[cfg(test)]
#[path = "atomic_file_tests.rs"]
mod tests;
