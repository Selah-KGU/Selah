//! Incremental SSE data framing, independent of HTTP chunk and UTF-8 boundaries.
//! Complete lines borrow the incoming bytes; only partial lines are retained.
use super::is_remote_cancelled;
use futures_util::StreamExt;

pub(super) async fn receive(
    response: reqwest::Response,
    generation: &str,
    mut on_data: impl FnMut(&str) -> bool + Send,
) -> Result<(), String> {
    let mut decoder = Decoder::default();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if is_remote_cancelled(generation) {
            break;
        }
        let bytes = chunk.map_err(|error| format!("ストリーム読み取り失敗: {error}"))?;
        if !decoder.push(&bytes, |data| {
            !is_remote_cancelled(generation) && on_data(data)
        }) {
            break;
        }
    }
    // EOF does not dispatch an event that lacks its terminating blank line.
    Ok(())
}

#[derive(Default)]
struct Decoder {
    partial_line: Vec<u8>,
    event: Event,
    skip_lf: bool,
    stopped: bool,
}

impl Decoder {
    /// The callback returns false to stop the entire stream, including events
    /// already coalesced into this chunk. A stopped decoder cannot emit again.
    fn push(&mut self, mut bytes: &[u8], mut on_data: impl FnMut(&str) -> bool) -> bool {
        if self.stopped {
            return false;
        }
        while !bytes.is_empty() {
            if self.skip_lf {
                self.skip_lf = false;
                if bytes[0] == b'\n' {
                    bytes = &bytes[1..];
                    continue;
                }
            }
            let Some(end) = bytes.iter().position(|byte| matches!(*byte, b'\r' | b'\n')) else {
                self.partial_line.extend_from_slice(bytes);
                return true;
            };
            self.skip_lf = bytes[end] == b'\r';
            let keep_reading = if self.partial_line.is_empty() {
                self.event.line(&bytes[..end], &mut on_data)
            } else {
                self.partial_line.extend_from_slice(&bytes[..end]);
                let keep_reading = self.event.line(&self.partial_line, &mut on_data);
                self.partial_line.clear();
                keep_reading
            };
            if !keep_reading {
                self.stopped = true;
                return false;
            }
            bytes = &bytes[end + 1..];
        }
        true
    }
}

struct Event {
    first_line: bool,
    data: String,
}
impl Default for Event {
    fn default() -> Self {
        Self {
            first_line: true,
            data: String::new(),
        }
    }
}
impl Event {
    // SSE data rules: https://html.spec.whatwg.org/multipage/server-sent-events.html
    // Metadata/reconnect fields are intentionally not used by AI generation.
    fn line(&mut self, bytes: &[u8], on_data: &mut impl FnMut(&str) -> bool) -> bool {
        let line = String::from_utf8_lossy(bytes);
        let line = if std::mem::take(&mut self.first_line) {
            line.strip_prefix('\u{feff}').unwrap_or(&line)
        } else {
            &line
        };
        if line.is_empty() {
            if self.data.is_empty() {
                return true;
            }
            self.data.pop(); // Remove the final LF appended by the data field.
            let keep_reading = on_data(&self.data);
            self.data.clear(); // Reuse the event allocation, never copy a tail.
            return keep_reading;
        }
        if line.starts_with(':') {
            return true;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        if field == "data" {
            self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
            self.data.push('\n');
        }
        true
    }
}

#[cfg(test)]
#[path = "sse/tests.rs"]
mod tests;
