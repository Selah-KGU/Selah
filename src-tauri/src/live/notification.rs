//! A carried immutable board refers to its earlier summary in this recording.
//! New boards remain complete; receivers recover missing history via the page RPC.
use super::types::LiveSessionUpdate;
use super::{LiveSummaryChunk, SharedSummaryChunk};
use serde::{Serialize, Serializer};

#[derive(Clone, Serialize)]
pub(super) struct LiveSessionNotification {
    whiteboard_delta_version: u8,
    #[serde(flatten)]
    update: LiveSessionUpdate<NotificationChunk>,
}

impl LiveSessionNotification {
    pub(super) fn new(update: LiveSessionUpdate, reference: Option<usize>) -> Self {
        // Move all metadata rather than maintaining a second metadata schema
        // or deep-cloning the chunk. The owner also freezes the capture during emit.
        let LiveSessionUpdate {
            update_revision,
            session_id,
            active,
            course,
            started_at,
            next_summary_at_ms,
            summarizing,
            finish_phase,
            finish_revision,
            transcript_line_count,
            pending_line_count,
            summary_count,
            latest_summary,
        } = update;
        Self {
            whiteboard_delta_version: 1,
            update: LiveSessionUpdate {
                update_revision,
                session_id,
                active,
                course,
                started_at,
                next_summary_at_ms,
                summarizing,
                finish_phase,
                finish_revision,
                transcript_line_count,
                pending_line_count,
                summary_count,
                latest_summary: latest_summary.map(|chunk| NotificationChunk { chunk, reference }),
            },
        }
    }
}

#[derive(Clone)]
struct NotificationChunk {
    chunk: SharedSummaryChunk,
    reference: Option<usize>,
}

impl Serialize for NotificationChunk {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Some(reference) = self.reference else {
            return self.chunk.serialize(serializer);
        };
        #[derive(Serialize)]
        struct Carried<'a> {
            title: &'a str,
            range_label: &'a str,
            body: &'a str,
            line_count: usize,
            terms: &'a [super::LiveTermExplanation],
            whiteboard_from_summary: usize,
        }
        let LiveSummaryChunk {
            title,
            range_label,
            body,
            line_count,
            terms,
            ..
        } = self.chunk.as_ref();
        Carried {
            title,
            range_label,
            body,
            line_count: *line_count,
            terms,
            whiteboard_from_summary: reference,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests;
