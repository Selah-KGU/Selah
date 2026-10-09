use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub(in crate::live) use crate::atomic_file::{
    atomic_write, atomic_write_with, WriteError, BUFFER_BYTES,
};

/// Repair a torn last record before appending. A complete last record that
/// lacks its newline is preserved, including after recovery loaded it.
pub(in crate::live) fn append_ndjson<E>(
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<(), E>,
    valid_record: impl Fn(&[u8]) -> bool,
) -> Result<(), WriteError<E>> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(WriteError::Storage)?;
    repair_tail(&mut file, valid_record).map_err(WriteError::Storage)?;
    let length = file.metadata().map_err(WriteError::Storage)?.len();
    let mut transaction = AppendTransaction {
        file,
        original_length: length,
        committed: false,
    };
    // Writer-local buffers must be flushed on success. On failure/unwind they
    // drop before this guard rolls back all bytes from the new batch.
    write(&mut transaction.file).map_err(WriteError::Content)?;
    transaction.file.sync_data().map_err(WriteError::Storage)?;
    transaction.committed = true;
    Ok(())
}

struct AppendTransaction {
    file: File,
    original_length: u64,
    committed: bool,
}
impl Drop for AppendTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.file.set_len(self.original_length);
            let _ = self.file.sync_data();
        }
    }
}

fn repair_tail(file: &mut File, valid_record: impl Fn(&[u8]) -> bool) -> io::Result<()> {
    let length = file.metadata()?.len();
    if length > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            let mut end = length;
            let mut blocks = Vec::new();
            let beginning = loop {
                let start = end.saturating_sub(4096);
                let mut block = vec![0; (end - start) as usize];
                file.seek(SeekFrom::Start(start))?;
                file.read_exact(&mut block)?;
                if let Some(newline) = block.iter().rposition(|byte| *byte == b'\n') {
                    blocks.push(block[newline + 1..].to_vec());
                    break start + newline as u64 + 1;
                }
                blocks.push(block);
                if start == 0 {
                    break 0;
                }
                end = start;
            };
            let tail: Vec<u8> = blocks.into_iter().rev().flatten().collect();
            if valid_record(&tail) {
                file.write_all(b"\n")?;
            } else {
                file.set_len(beginning)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_file_keeps_a_complete_document_and_cleans_temporary_files() {
        let dir = std::env::temp_dir().join(format!("selah-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("note.md");
        atomic_write(&file, b"original note").unwrap();
        atomic_write(&file, b"complete replacement").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"complete replacement");
        // Replacing a directory fails after staging. Existing content survives.
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("existing"), b"keep").unwrap();
        assert!(atomic_write(&blocked, b"new").is_err());
        assert_eq!(std::fs::read(blocked.join("existing")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn complete_record_without_newline_survives_a_later_append() {
        let dir = std::env::temp_dir().join(format!("selah-journal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("lines.ndjson");
        // A large final record crosses several backward scan blocks.
        let first =
            serde_json::json!({ "i": 1, "t": "語".repeat(5000), "a": "10:00:01" }).to_string();
        std::fs::write(&path, first.as_bytes()).unwrap();
        let next = b"{\"i\":2,\"t\":\"tail\",\"a\":\"10:00:02\"}\n";
        append_ndjson(
            &path,
            |file| file.write_all(next),
            |tail| serde_json::from_slice::<serde_json::Value>(tail).is_ok(),
        )
        .unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        let records: Vec<_> = contents
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["t"].as_str().unwrap().chars().count(), 5000);
        assert_eq!(records[1]["i"], 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
