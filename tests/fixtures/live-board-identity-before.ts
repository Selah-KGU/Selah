// Frozen update/merge algorithms immediately before board identity sharing.
// Only relative imports are adapted; excluded from product bundles.
import type { LiveSessionSnapshot, LiveSessionUpdate, LiveTranscriptUpdate, LiveSurfaceSnapshot, LiveSaveResult, LiveSurfaceSaveResult } from "../../src/lib/liveSessionApi";
import { extractOverallSummary } from "../../src/lib/liveSavedSummary";

import { LIVE_VISIBLE_LINE_WINDOW, liveSurfaceSnapshot, emptyLiveSurfaceSnapshot } from "../../src/lib/liveSurfaceSnapshot";
export { LIVE_VISIBLE_LINE_WINDOW, liveSurfaceSnapshot, emptyLiveSurfaceSnapshot } from "../../src/lib/liveSurfaceSnapshot";
export type { LiveSurfaceSnapshot } from "../../src/lib/liveSessionApi";

export type LiveSessionIdentity = Pick<LiveSessionSnapshot, "active" | "session_id">;
export type LiveSavedPreview = Pick<LiveSurfaceSaveResult, "summary_markdown">;

/** The save badge/preview needs the summary, not the archived record graph. */
export function liveSavedPreview(result: LiveSaveResult | LiveSurfaceSaveResult): LiveSavedPreview | null {
  return result.saved ? { summary_markdown: "summary_markdown" in result
    ? result.summary_markdown : extractOverallSummary(result.markdown) } : null;
}

export function isCurrentLiveSessionEvent(
  snapshot: LiveSessionIdentity,
  event: { session_id?: string | null },
): boolean {
  return snapshot.active && !!snapshot.session_id && event.session_id === snapshot.session_id;
}

/** Scope every STT event to the recording that owned the microphone. */
export function isCurrentLiveSttEvent(
  snapshot: LiveSessionIdentity,
  event: { caller: string; live_session_id?: string },
): boolean {
  return event.caller === "live"
    && isCurrentLiveSessionEvent(snapshot, { session_id: event.live_session_id });
}

export function applyTranscriptDelta(
  snapshot: LiveSurfaceSnapshot,
  update: LiveTranscriptUpdate,
): { snapshot: LiveSurfaceSnapshot; needsResync: boolean } {
  if (!snapshot.active || snapshot.session_id !== update.session_id) {
    return { snapshot, needsResync: !snapshot.session_id };
  }
  const count = snapshot.transcript_line_count;
  if (update.line_count <= count) return { snapshot, needsResync: false };
  if (update.line_count !== count + 1) return { snapshot, needsResync: true };
  const visible_lines = snapshot.visible_lines.slice(-(LIVE_VISIBLE_LINE_WINDOW - 1));
  visible_lines.push(update.line);
  return {
    snapshot: {
      ...snapshot,
      transcript_line_count: update.line_count,
      visible_lines,
    },
    needsResync: false,
  };
}

/** A summary event or recovery read may have been captured before a newer delta. */
export function mergeLiveSnapshot(
  current: LiveSurfaceSnapshot,
  incoming: LiveSessionSnapshot | LiveSurfaceSnapshot,
  minimumRevision = current.update_revision ?? 0,
): LiveSurfaceSnapshot {
  const incomingSurface = "visible_lines" in incoming ? incoming : {
    ...liveSurfaceSnapshot(incoming),
    // Full RPCs define coverage by their pending array; the optional client
    // prefix belongs to a UI state, not a newer full-record recovery reply.
    pending_from_line: incoming.transcript_lines.length - incoming.pending_lines.length,
  };
  const olderUpdate = (incomingSurface.update_revision ?? 0) < minimumRevision;
  if (!current.session_id || current.session_id !== incomingSurface.session_id) {
    if (olderUpdate) return current;
    // A passive inactive read must not erase the completed record/preview.
    if (!current.active && !incomingSurface.active && incomingSurface.session_id == null
      && incomingSurface.course == null && incomingSurface.transcript_line_count === 0
      && incomingSurface.summaries.length === 0) {
      return { ...current, update_revision: incomingSurface.update_revision };
    }
    return incomingSurface;
  }
  const olderFinish = (incomingSurface.finish_revision ?? 0) < (current.finish_revision ?? 0);
  const keepTranscript = current.transcript_line_count >= incomingSurface.transcript_line_count;
  const pendingFrom = olderUpdate
    ? current.pending_from_line
    : incomingSurface.pending_from_line;
  const metadata = olderUpdate ? current : incomingSurface;
  return {
    ...metadata,
    finish_phase: olderFinish ? current.finish_phase : metadata.finish_phase,
    finish_revision: olderFinish ? current.finish_revision : metadata.finish_revision,
    next_summary_at_ms: olderFinish ? current.next_summary_at_ms : metadata.next_summary_at_ms,
    summarizing: olderFinish ? current.summarizing : metadata.summarizing,
    transcript_line_count: Math.max(current.transcript_line_count, incomingSurface.transcript_line_count),
    visible_lines: keepTranscript ? current.visible_lines : incomingSurface.visible_lines,
    pending_from_line: pendingFrom,
    // Chunks are immutable and append-only within a backend session. Preserve
    // their identity so speech ticks do not recalculate the whiteboard layout.
    summaries: current.summaries.length >= incomingSurface.summaries.length
      ? current.summaries
      : incomingSurface.summaries,
  };
}

/** Apply lightweight state and one new immutable chunk; recover only gaps. */
export function applyLiveSessionUpdate(
  current: LiveSurfaceSnapshot,
  update: LiveSessionUpdate,
  minimumRevision = current.update_revision ?? 0,
): { snapshot: LiveSurfaceSnapshot; needsResync: boolean } {
  if (update.update_revision <= minimumRevision) return { snapshot: current, needsResync: false };
  const counts = [update.transcript_line_count, update.pending_line_count, update.summary_count];
  if (!Number.isSafeInteger(update.update_revision) || update.update_revision <= 0
    || counts.some((count) => !Number.isSafeInteger(count) || count < 0)
    || update.pending_line_count > update.transcript_line_count
    || (update.active && !update.session_id)) {
    return { snapshot: current, needsResync: true };
  }
  if (!update.active) {
    const empty = { transcript_line_count: 0, visible_lines: [], pending_from_line: 0, summaries: [] };
    const base = current.active
      ? { ...empty, session_id: null, course: null, started_at: null }
      : current;
    return { snapshot: { ...base, active: false, update_revision: update.update_revision,
      finish_phase: null, summarizing: false, next_summary_at_ms: null }, needsResync: false };
  }
  const sameRecording = current.active && current.session_id === update.session_id;
  const base: LiveSurfaceSnapshot = sameRecording ? current : {
    ...emptyLiveSurfaceSnapshot(), active: true,
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
    },
    needsResync: base.transcript_line_count < update.transcript_line_count
      || summaries.length < update.summary_count,
  };
}
