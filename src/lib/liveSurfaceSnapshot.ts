import type { LiveSessionSnapshot, LiveSurfaceSnapshot, LiveSaveResult, LiveSurfaceSaveResult } from "./liveSessionApi";
import { extractOverallSummary } from "./liveSavedSummary";

/** Existing LIVE display window; never a record/storage limit. */
export const LIVE_VISIBLE_LINE_WINDOW = 120;

export function liveSurfaceSnapshot(full: LiveSessionSnapshot): LiveSurfaceSnapshot {
  const { transcript_lines, pending_lines, ...metadata } = full;
  return {
    ...metadata,
    transcript_line_count: transcript_lines.length,
    visible_lines: transcript_lines.length <= LIVE_VISIBLE_LINE_WINDOW
      ? transcript_lines : transcript_lines.slice(-LIVE_VISIBLE_LINE_WINDOW),
    pending_from_line: full.pending_from_line ?? transcript_lines.length - pending_lines.length,
  };
}
export function emptyLiveSurfaceSnapshot(): LiveSurfaceSnapshot {
  return {
    active: false, course: null, started_at: null,
    transcript_line_count: 0, visible_lines: [], pending_from_line: 0, summaries: [],
  };
}

export function liveSurfaceSaveResult(full: LiveSaveResult): LiveSurfaceSaveResult {
  const { snapshot, markdown, ...metadata } = full;
  return { ...metadata, snapshot: liveSurfaceSnapshot(snapshot),
    summary_markdown: extractOverallSummary(markdown) };
}
