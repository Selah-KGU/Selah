// Frozen full-record update implementation before the display projection.
// Algorithm restored from its retained original source copy.
import type { LiveSessionSnapshot, LiveSessionUpdate, LiveTranscriptUpdate } from "../../src/lib/liveSessionApi";

export function isCurrentLiveSessionEvent(
  snapshot: LiveSessionSnapshot,
  event: { session_id?: string | null },
): boolean {
  return snapshot.active && !!snapshot.session_id && event.session_id === snapshot.session_id;
}

/** Scope every STT event to the recording that owned the microphone. */
export function isCurrentLiveSttEvent(
  snapshot: LiveSessionSnapshot,
  event: { caller: string; live_session_id?: string },
): boolean {
  return event.caller === "live"
    && isCurrentLiveSessionEvent(snapshot, { session_id: event.live_session_id });
}

export function applyTranscriptDelta(
  snapshot: LiveSessionSnapshot,
  update: LiveTranscriptUpdate,
): { snapshot: LiveSessionSnapshot; needsResync: boolean } {
  if (!snapshot.active || snapshot.session_id !== update.session_id) {
    return { snapshot, needsResync: !snapshot.session_id };
  }
  const count = snapshot.transcript_lines.length;
  if (update.line_count <= count) return { snapshot, needsResync: false };
  if (update.line_count !== count + 1) return { snapshot, needsResync: true };
  return {
    snapshot: {
      ...snapshot,
      transcript_lines: [...snapshot.transcript_lines, update.line],
      pending_lines: update.line_count > (snapshot.pending_from_line ?? 0)
        ? [...snapshot.pending_lines, update.line] : snapshot.pending_lines,
    },
    needsResync: false,
  };
}

/** A summary event or recovery read may have been captured before a newer delta. */
export function mergeLiveSnapshot(
  current: LiveSessionSnapshot,
  incoming: LiveSessionSnapshot,
  minimumRevision = current.update_revision ?? 0,
): LiveSessionSnapshot {
  const olderUpdate = (incoming.update_revision ?? 0) < minimumRevision;
  if (!current.session_id || current.session_id !== incoming.session_id) {
    if (olderUpdate) return current;
    // A passive inactive read must not erase the completed record/preview.
    if (!current.active && !incoming.active && incoming.session_id == null
      && incoming.course == null && incoming.transcript_lines.length === 0
      && incoming.summaries.length === 0) {
      return { ...current, update_revision: incoming.update_revision };
    }
    return incoming;
  }
  const olderFinish = (incoming.finish_revision ?? 0) < (current.finish_revision ?? 0);
  const transcript = current.transcript_lines.length >= incoming.transcript_lines.length
    ? current.transcript_lines : incoming.transcript_lines;
  const pendingFrom = olderUpdate
    ? current.pending_from_line ?? current.transcript_lines.length - current.pending_lines.length
    : incoming.transcript_lines.length - incoming.pending_lines.length;
  const metadata = olderUpdate ? current : incoming;
  return {
    ...metadata,
    finish_phase: olderFinish ? current.finish_phase : metadata.finish_phase,
    finish_revision: olderFinish ? current.finish_revision : metadata.finish_revision,
    next_summary_at_ms: olderFinish ? current.next_summary_at_ms : metadata.next_summary_at_ms,
    summarizing: olderFinish ? current.summarizing : metadata.summarizing,
    transcript_lines: transcript,
    pending_from_line: pendingFrom,
    pending_lines: transcript === current.transcript_lines
      && pendingFrom === (current.pending_from_line ?? current.transcript_lines.length - current.pending_lines.length)
      ? current.pending_lines
      : transcript.slice(pendingFrom),
    // Chunks are immutable and append-only within a backend session. Preserve
    // their identity so speech ticks do not recalculate the whiteboard layout.
    summaries: current.summaries.length >= incoming.summaries.length
      ? current.summaries
      : incoming.summaries,
  };
}

/** Apply lightweight state and one new immutable chunk; recover only gaps. */
export function applyLiveSessionUpdate(
  current: LiveSessionSnapshot,
  update: LiveSessionUpdate,
  minimumRevision = current.update_revision ?? 0,
): { snapshot: LiveSessionSnapshot; needsResync: boolean } {
  if (update.update_revision <= minimumRevision) return { snapshot: current, needsResync: false };
  const counts = [update.transcript_line_count, update.pending_line_count, update.summary_count];
  if (!Number.isSafeInteger(update.update_revision) || update.update_revision <= 0
    || counts.some((count) => !Number.isSafeInteger(count) || count < 0)
    || update.pending_line_count > update.transcript_line_count
    || (update.active && !update.session_id)) {
    return { snapshot: current, needsResync: true };
  }
  if (!update.active) {
    const empty = { transcript_lines: [], pending_lines: [], summaries: [] };
    const base = current.active
      ? { ...empty, session_id: null, course: null, started_at: null }
      : current;
    return { snapshot: { ...base, active: false, update_revision: update.update_revision,
      finish_phase: null, summarizing: false, next_summary_at_ms: null }, needsResync: false };
  }
  const sameRecording = current.active && current.session_id === update.session_id;
  const base: LiveSessionSnapshot = sameRecording ? current : {
    active: true, transcript_lines: [], pending_lines: [], summaries: [],
    course: null, started_at: null,
  };
  let summaries = base.summaries;
  if (update.summary_count === summaries.length + 1 && update.latest_summary) {
    summaries = [...summaries, update.latest_summary];
  }
  const pendingFrom = update.transcript_line_count - update.pending_line_count;
  const olderFinish = sameRecording && update.finish_revision < (current.finish_revision ?? 0);
  const { transcript_line_count: _lines, pending_line_count: _pending,
    summary_count: _summaries, latest_summary: _chunk, ...metadata } = update;
  return {
    snapshot: {
      ...base, ...metadata,
      finish_phase: olderFinish ? current.finish_phase : update.finish_phase,
      finish_revision: olderFinish ? current.finish_revision : update.finish_revision,
      next_summary_at_ms: olderFinish ? current.next_summary_at_ms : update.next_summary_at_ms,
      summarizing: olderFinish ? current.summarizing : update.summarizing,
      summaries,
      pending_from_line: pendingFrom,
      pending_lines: pendingFrom === (base.pending_from_line ?? base.transcript_lines.length - base.pending_lines.length)
        ? base.pending_lines : base.transcript_lines.slice(pendingFrom),
    },
    needsResync: base.transcript_lines.length < update.transcript_line_count
      || summaries.length < update.summary_count,
  };
}
