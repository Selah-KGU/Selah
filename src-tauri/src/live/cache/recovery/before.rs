// Frozen pre-streaming recovery, used only by tests and an isolated benchmark.
use super::{LiveDayCache, LiveTranscriptLine};
use std::io::{self, Read};
use std::path::Path;

#[derive(serde::Deserialize)]
struct LiveLineDeltaOwned {
    i: usize,
    t: String,
    a: String,
}

pub(super) fn load(
    path: &Path,
    journal: &Path,
    today: &str,
    course_name: &str,
) -> Option<LiveDayCache> {
    let data = std::fs::read_to_string(path).ok()?;
    let mut cache: LiveDayCache = serde_json::from_str(&data).ok()?;
    if cache.date != today || cache.course_name != course_name {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(journal);
        return None;
    }
    if let Ok(deltas_data) = std::fs::read_to_string(journal) {
        replay(&mut cache, &deltas_data);
    }
    Some(cache)
}

pub(super) fn replay_reader(cache: &mut LiveDayCache, mut reader: impl Read) -> io::Result<()> {
    let mut data = String::new();
    reader.read_to_string(&mut data)?;
    replay(cache, &data);
    Ok(())
}

fn replay(cache: &mut LiveDayCache, deltas_text: &str) {
    for raw in deltas_text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(delta) = serde_json::from_str::<LiveLineDeltaOwned>(raw) else {
            continue;
        };
        let expected = cache.transcript_lines.len();
        if delta.i < expected {
            continue;
        }
        if delta.i != expected {
            break;
        }
        cache.transcript_lines.push(
            LiveTranscriptLine {
                text: delta.t,
                at: delta.a,
            }
            .into(),
        );
    }
}
