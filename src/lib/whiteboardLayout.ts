// Thin typed wrapper around the global WhiteboardLayout module
// (static/whiteboard-layout.js). The module is loaded by a <script> tag in
// index.html before the Svelte bundle, so window.WhiteboardLayout is
// guaranteed to exist by the time anything imports this file.

import type { LiveWhiteboard } from "./api";

export type WhiteboardLayoutNode = {
  id: string;
  label: string;
  detail: string;
  kind: string;
  role: string;
  parentId: string;
  sourceType: string;
  sourceLabel: string;
  x: number;
  y: number;
};

export type WhiteboardLayoutEdge = {
  id: string;
  from: string;
  to: string;
  label: string;
  colorKind: string;
  colorSourceType: string;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  cx: number;
  cy: number;
  lx: number;
  ly: number;
  labelWidth: number;
  trunk: boolean;
  redundant: boolean;
};

export type WhiteboardLayoutResult = {
  title: string;
  nodes: WhiteboardLayoutNode[];
  edges: WhiteboardLayoutEdge[];
};

export type WhiteboardLayoutOptions = {
  maxNodes?: number;
  maxEdges?: number;
  fallbackBoardTitle?: string;
  externalNodeLabel?: string;
};

declare global {
  interface Window {
    WhiteboardLayout?: {
      compute(
        board: unknown,
        options?: WhiteboardLayoutOptions,
      ): WhiteboardLayoutResult | null;
    };
  }
}

// Cache layouts by whiteboard object identity. The upstream `snapshot` object
// is replaced on every transcript chunk in Live mode, but the per-summary
// `whiteboard` reference is stable as long as the summary itself hasn't
// changed — so we can skip the (expensive) relaxation entirely when nothing
// material is different. WeakMap means cached entries are GC'd as soon as
// the underlying summary is dropped from the snapshot, no manual bookkeeping.
const layoutCache = new WeakMap<object, { optsKey: string; result: WhiteboardLayoutResult | null }>();

function makeOptionsKey(options?: WhiteboardLayoutOptions): string {
  if (!options) return "";
  // Manual concat — faster than JSON.stringify on a hot path that runs per
  // reactive read.
  return (
    (options.maxNodes ?? "") + "|" +
    (options.maxEdges ?? "") + "|" +
    (options.fallbackBoardTitle ?? "") + "|" +
    (options.externalNodeLabel ?? "")
  );
}

export function computeWhiteboardLayout(
  board: LiveWhiteboard | null,
  options?: WhiteboardLayoutOptions,
): WhiteboardLayoutResult | null {
  if (!board) return null;
  const impl = typeof window !== "undefined" ? window.WhiteboardLayout : undefined;
  if (!impl) return null;
  const optsKey = makeOptionsKey(options);
  const boardKey = board as unknown as object;
  const cached = layoutCache.get(boardKey);
  if (cached && cached.optsKey === optsKey) return cached.result;
  const result = impl.compute(board, options);
  layoutCache.set(boardKey, { optsKey, result });
  return result;
}
