import type { LiveSummaryChunk, LiveWhiteboard } from "../../liveSessionApi";

// Replies and notifications are JSON trees. Compare every field, including
// future fields; node/edge array order matters, object property order does not.
// Unusually deep or non-JSON input simply keeps its own identity. The bound
// also prevents a malformed cyclic fixture from overflowing the call stack.
function sameJsonValue(left: unknown, right: unknown, depth = 0): boolean {
  if (Object.is(left, right)) return true;
  if (!left || !right || typeof left !== "object" || typeof right !== "object" || depth >= 64) return false;
  const array = Array.isArray(left);
  if (array !== Array.isArray(right)) return false;
  if (array && left.length !== (right as unknown[]).length) return false;
  if (!array) {
    const a = Object.getPrototypeOf(left), b = Object.getPrototypeOf(right);
    if ((a !== Object.prototype && a !== null) || (b !== Object.prototype && b !== null)) return false;
  }
  const keys = Object.keys(left);
  if (keys.length !== Object.keys(right).length) return false;
  const a = left as Record<string, unknown>, b = right as Record<string, unknown>;
  for (const key of keys) {
    if (!Object.hasOwn(b, key) || !sameJsonValue(a[key], b[key], depth + 1)) return false;
  }
  return true;
}

function sharedChunk(chunk: LiveSummaryChunk, previous?: LiveWhiteboard | null): LiveSummaryChunk {
  const board = chunk.whiteboard;
  return board && previous && board !== previous && sameJsonValue(board, previous)
    ? { ...chunk, whiteboard: previous }
    : chunk;
}

/** Reuse the nearest complete board for one append, without retaining a cache. */
export function shareLiveChunkBoard(chunk: LiveSummaryChunk, history: LiveSummaryChunk[]): LiveSummaryChunk {
  if (!chunk.whiteboard) return chunk;
  for (let index = history.length - 1; index >= 0; index--) {
    if (history[index].whiteboard) return sharedChunk(chunk, history[index].whiteboard);
  }
  return chunk;
}

/** Recovery reads can contain independently parsed copies of earlier boards. */
export function shareLiveHistoryBoards(incoming: LiveSummaryChunk[], current: LiveSummaryChunk[] = []): LiveSummaryChunk[] {
  let result = incoming;
  let previous: LiveWhiteboard | null | undefined;
  for (let index = 0; index < incoming.length; index++) {
    const chunk = incoming[index];
    if (!chunk.whiteboard) continue;
    // Reuse an equal board at the same known index first. Chunk metadata still
    // belongs to the incoming read, even when its board is reused.
    let shared = sharedChunk(chunk, current[index]?.whiteboard);
    if (shared === chunk && chunk.whiteboard !== current[index]?.whiteboard) shared = sharedChunk(chunk, previous);
    previous = shared.whiteboard;
    if (shared !== chunk) {
      if (result === incoming) result = incoming.slice();
      result[index] = shared;
    }
  }
  return result;
}
