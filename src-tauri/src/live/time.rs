use chrono::{DateTime, Duration as ChronoDuration, Local};

use super::{LiveSession, LiveSummaryChunk, LiveTranscriptLine};

pub(in crate::live) fn format_datetime(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub(in crate::live) fn format_time(dt: DateTime<Local>) -> String {
    dt.format("%H:%M").to_string()
}

fn clock_time_on_session_date(
    session_started_at: DateTime<Local>,
    value: &str,
) -> Option<DateTime<Local>> {
    let time = chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M:%S")
        .or_else(|_| chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M"))
        .ok()?;
    let mut candidate = session_started_at
        .date_naive()
        .and_time(time)
        .and_local_timezone(Local)
        .earliest()?;
    if candidate + ChronoDuration::hours(12) < session_started_at {
        candidate += ChronoDuration::days(1);
    }
    Some(candidate)
}

pub(in crate::live) fn transcript_line_datetime(
    session_started_at: DateTime<Local>,
    line: &LiveTranscriptLine,
) -> Option<DateTime<Local>> {
    clock_time_on_session_date(session_started_at, &line.at)
}

fn summary_range_end_datetime(
    session_started_at: DateTime<Local>,
    summary: &LiveSummaryChunk,
) -> Option<DateTime<Local>> {
    let (_, end) = summary
        .range_label
        .rsplit_once('-')
        .or_else(|| summary.range_label.rsplit_once('–'))?;
    clock_time_on_session_date(session_started_at, end)
}

pub(in crate::live) fn latest_summary_end_datetime(
    session_started_at: DateTime<Local>,
    summaries: &[LiveSummaryChunk],
) -> Option<DateTime<Local>> {
    summaries
        .last()
        .and_then(|summary| summary_range_end_datetime(session_started_at, summary))
}

pub(in crate::live) fn last_transcript_line_datetime(
    session_started_at: DateTime<Local>,
    lines: &[LiveTranscriptLine],
    fallback: DateTime<Local>,
) -> DateTime<Local> {
    lines
        .last()
        .and_then(|line| transcript_line_datetime(session_started_at, line))
        .unwrap_or(fallback)
}

fn first_transcript_line_datetime(
    session_started_at: DateTime<Local>,
    lines: &[LiveTranscriptLine],
    fallback: DateTime<Local>,
) -> DateTime<Local> {
    lines
        .first()
        .and_then(|line| transcript_line_datetime(session_started_at, line))
        .unwrap_or(fallback)
}

pub(in crate::live) fn effective_batch_started_at(session: &LiveSession) -> DateTime<Local> {
    if session.summaries.is_empty() {
        return first_transcript_line_datetime(
            session.started_at,
            session.pending_lines.as_ref(),
            session.batch_started_at,
        );
    }
    session.batch_started_at
}
