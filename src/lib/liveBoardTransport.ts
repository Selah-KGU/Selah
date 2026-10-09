import type { LiveSummaryChunk, LiveSurfaceSnapshot, LiveSurfaceSaveResult, LiveWhiteboard } from "./liveSessionApi";

interface CompactSummary extends Omit<LiveSummaryChunk, "whiteboard"> {
  whiteboard_ref?: number;
}
export interface CompactLiveSurfaceSnapshot extends Omit<LiveSurfaceSnapshot, "summaries"> {
  whiteboard_table_version: 1;
  whiteboards: LiveWhiteboard[];
  summaries: CompactSummary[];
}
export interface CompactLiveSurfaceSaveResult extends Omit<LiveSurfaceSaveResult, "snapshot"> {
  snapshot: CompactLiveSurfaceSnapshot;
}

/** Each reply owns its board table. Restored chunks share these exact objects,
 * so layout caches reuse a carried version without a global board cache. */
export function expandLiveSurface(wire: CompactLiveSurfaceSnapshot): LiveSurfaceSnapshot {
  if (wire.whiteboard_table_version !== 1 || !Array.isArray(wire.whiteboards) || !Array.isArray(wire.summaries)) {
    throw new Error("LIVE 白板データの形式が不正です");
  }
  const { whiteboard_table_version, whiteboards, summaries, ...metadata } = wire;
  if (whiteboards.some(board => board === null || typeof board !== "object" || Array.isArray(board))) {
    throw new Error("LIVE 白板データの形式が不正です");
  }
  const restored = summaries.map(chunk => {
    if (chunk === null || typeof chunk !== "object" || Array.isArray(chunk) || "whiteboard" in chunk) {
      throw new Error("LIVE 白板データの形式が不正です");
    }
    const { whiteboard_ref, ...summary } = chunk;
    if (whiteboard_ref === undefined) return summary;
    if (!Number.isSafeInteger(whiteboard_ref) || whiteboard_ref < 0 || whiteboard_ref >= whiteboards.length) {
      throw new Error("LIVE 白板の参照が不正です");
    }
    return { ...summary, whiteboard: whiteboards[whiteboard_ref] };
  });
  return { ...metadata, summaries: restored };
}

export function expandLiveSurfaceSave(wire: CompactLiveSurfaceSaveResult): LiveSurfaceSaveResult {
  return { ...wire, snapshot: expandLiveSurface(wire.snapshot) };
}
