import type { LiveSessionUpdate, LiveSummaryChunk, LiveSurfaceSnapshot } from "../../liveSessionApi";
import { applyLiveSessionUpdate } from "./liveTranscript";

type NotificationChunk = LiveSummaryChunk & { whiteboard_from_summary?: number };
export type LiveSessionNotification = LiveSessionUpdate | (Omit<LiveSessionUpdate, "latest_summary"> & {
  whiteboard_delta_version: 1;
  latest_summary?: NotificationChunk;
});

/** A reference belongs to the immutable history of this recording only. */
export function applyLiveSessionNotification(
  current: LiveSurfaceSnapshot,
  notification: LiveSessionNotification,
  minimumRevision = current.update_revision ?? 0,
): { snapshot: LiveSurfaceSnapshot; needsResync: boolean } {
  if (!notification || typeof notification !== "object") return { snapshot: current, needsResync: true };
  // Delayed references must not trigger recovery after a replacement recording.
  if (notification.update_revision <= minimumRevision) return { snapshot: current, needsResync: false };
  if (!("whiteboard_delta_version" in notification)) {
    const chunk = notification.latest_summary;
    if (chunk && typeof chunk === "object" && "whiteboard_from_summary" in chunk) {
      return { snapshot: current, needsResync: true };
    }
    return applyLiveSessionUpdate(current, notification, minimumRevision);
  }
  if (notification.whiteboard_delta_version !== 1) return { snapshot: current, needsResync: true };
  const { whiteboard_delta_version: _version, latest_summary: chunk, ...metadata } = notification;
  if (chunk == null) return applyLiveSessionUpdate(current, metadata, minimumRevision);
  if (typeof chunk !== "object" || Array.isArray(chunk)) return { snapshot: current, needsResync: true };
  if (!("whiteboard_from_summary" in chunk)) {
    return applyLiveSessionUpdate(current, { ...metadata, latest_summary: chunk }, minimumRevision);
  }
  const { whiteboard_from_summary: reference, ...summary } = chunk;
  if ("whiteboard" in chunk || typeof reference !== "number" || !Number.isSafeInteger(reference) || reference < 0
    || reference >= notification.summary_count - 1) return { snapshot: current, needsResync: true };
  // If a summary is missing, the regular count/revision logic applies metadata
  // and asks for a complete page. Never insert a chunk with an unresolved board.
  const canAppend = current.active && current.session_id === notification.session_id
    && notification.summary_count === current.summaries.length + 1;
  const board = canAppend ? current.summaries[reference]?.whiteboard : null;
  return applyLiveSessionUpdate(current, board ? { ...metadata, latest_summary: { ...summary, whiteboard: board } }
    : metadata, minimumRevision);
}
