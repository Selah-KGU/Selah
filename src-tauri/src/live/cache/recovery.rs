use super::{LiveDayCache, LiveLineDeltaBorrowed, LiveTranscriptLine, SharedTranscriptLine};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

#[cfg(test)]
#[path = "recovery/tests.rs"]
mod tests;

pub(in crate::live) fn load(
    path: &Path,
    journal: &Path,
    today: &str,
    course_name: &str,
) -> Option<LiveDayCache> {
    // Keep Serde's slice parser, but release the raw JSON before opening the
    // journal. Neither file's entire source needs to live alongside both DTOs.
    let mut cache = {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<LiveDayCache>(&data).ok()?
    };
    if cache.date != today || cache.course_name != course_name {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(journal);
        return None;
    }
    if let Ok(file) = File::open(journal) {
        // A failed read/UTF-8 validation ignores the entire journal, just as
        // the previous read_to_string did. The guard keeps the complete base.
        let _ = replay(&mut cache, BufReader::new(file));
    }
    Some(cache)
}

struct Replay<'a> {
    lines: &'a mut Vec<SharedTranscriptLine>,
    original_length: usize,
    original_capacity: usize,
    committed: bool,
}
impl Drop for Replay<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.lines.truncate(self.original_length);
            // A rejected large log must not leave its expanded reference
            // index resident in the otherwise unchanged base snapshot.
            self.lines.shrink_to(self.original_capacity);
        }
    }
}

pub(in crate::live) fn replay(
    cache: &mut LiveDayCache,
    mut reader: impl BufRead,
) -> io::Result<()> {
    let mut transaction = Replay {
        original_length: cache.transcript_lines.len(),
        original_capacity: cache.transcript_lines.capacity(),
        lines: &mut cache.transcript_lines,
        committed: false,
    };
    let mut line = String::new();
    let mut gap = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        // Continue validating reads after a gap: invalid UTF-8 or an IO error
        // anywhere in the file used to reject even the earlier valid prefix.
        if gap || line.trim().is_empty() {
            continue;
        }
        let Ok(delta) = serde_json::from_str::<LiveLineDeltaBorrowed<'_>>(&line) else {
            continue;
        };
        let expected = transaction.lines.len();
        if delta.i < expected {
            continue;
        }
        if delta.i != expected {
            gap = true;
            continue;
        }
        transaction.lines.push(
            LiveTranscriptLine {
                text: delta.t.into_owned(),
                at: delta.a.into_owned(),
            }
            .into(),
        );
    }
    transaction.committed = true;
    Ok(())
}
