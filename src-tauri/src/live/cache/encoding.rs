use super::{atomic::BUFFER_BYTES, LiveLineDeltaRef, SharedTranscriptLine};
use std::io::{BufWriter, Write};

/// One bounded buffer; the caller owns commit/rollback and file synchronization.
pub(in crate::live) fn cache(
    target: impl Write,
    value: &(impl serde::Serialize + ?Sized),
) -> serde_json::Result<()> {
    let mut writer = BufWriter::with_capacity(BUFFER_BYTES, target);
    serde_json::to_writer(&mut writer, value)?;
    writer.flush().map_err(serde_json::Error::io)
}

pub(in crate::live) fn deltas(
    target: impl Write,
    lines: &[SharedTranscriptLine],
    start: usize,
) -> serde_json::Result<()> {
    let mut writer = BufWriter::with_capacity(BUFFER_BYTES, target);
    for (offset, line) in lines.iter().enumerate().skip(start) {
        let delta = LiveLineDeltaRef {
            i: offset,
            t: &line.text,
            a: &line.at,
        };
        serde_json::to_writer(&mut writer, &delta)?;
        writer.write_all(b"\n").map_err(serde_json::Error::io)?;
    }
    writer.flush().map_err(serde_json::Error::io)
}
