// Thin typed wrapper around the global WhiteboardLayout module
// (static/whiteboard-layout.js). Views request it when they receive a board;
// importing this module alone does not insert a script. Callers inside
// $derived must also read whiteboardLayoutReady; a late script load does not
// otherwise invalidate the derived.

import { writable } from "svelte/store";
import type { LiveWhiteboard } from "./api";

export const whiteboardLayoutReady = writable(false);

let whiteboardLayoutPromise: Promise<void> | null = null;

function hasWhiteboardLayout(): boolean {
  const impl = window.WhiteboardLayout;
  return typeof impl?.compute === "function" && typeof impl?.topics === "function";
}

export function ensureWhiteboardLayout(): Promise<void> {
  if (typeof window === "undefined" || typeof document === "undefined") return Promise.resolve();
  if (hasWhiteboardLayout()) {
    whiteboardLayoutReady.set(true);
    return Promise.resolve();
  }
  if (whiteboardLayoutPromise) return whiteboardLayoutPromise;
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
  // Publish the attempt before touching the DOM: synchronous insertion errors
  // or load events must not leave a rejected/completed promise cached forever.
  whiteboardLayoutPromise = promise;
  let script: HTMLScriptElement | null = null;
  let settled = false;
  const finish = (error: Error | null) => {
    if (settled) return;
    settled = true;
    if (whiteboardLayoutPromise === promise) whiteboardLayoutPromise = null;
    script?.removeEventListener("load", onLoad);
    script?.removeEventListener("error", onError);
    if (error) {
      script?.remove();
      reject(error);
    } else {
      resolve();
      whiteboardLayoutReady.set(true);
    }
  };
  const onLoad = () => {
    if (settled) return;
    finish(hasWhiteboardLayout() ? null : new Error("whiteboard-layout.js loaded without a valid WhiteboardLayout"));
  };
  const onError = () => finish(new Error("whiteboard-layout.js failed to load"));
  try {
    whiteboardLayoutReady.set(false);
    // Without a pending attempt owned by this module, a tagged script may have
    // already failed/completed. Waiting for its old event would never finish.
    document.querySelector('script[data-whiteboard-layout="1"]')?.remove();
    script = document.createElement("script");
    script.src = "/whiteboard-layout.js";
    script.async = true;
    script.dataset.whiteboardLayout = "1";
    script.addEventListener("load", onLoad, { once: true });
    script.addEventListener("error", onError, { once: true });
    document.head.appendChild(script);
  } catch (error) {
    finish(error instanceof Error ? error : new Error(String(error)));
  }
  return promise;
}

// A board arriving or a view reopening gets one retry after a failed attempt.
// Concurrent views share both attempts through ensureWhiteboardLayout. This
// does not poll or retry indefinitely after a persistent asset failure.
export async function prepareWhiteboardLayout(): Promise<void> {
  try {
    await ensureWhiteboardLayout();
  } catch {
    await ensureWhiteboardLayout();
  }
}

export type WhiteboardLayoutChip = {
  label: string;
  detail: string;
  sourceType: string;
  sourceLabel: string;
};

export type WhiteboardLayoutNode = {
  id: string;
  label: string;
  detail: string;
  nodeType: string;
  kind: string;
  role: string;
  parentId: string;
  sourceType: string;
  sourceLabel: string;
  x: number;
  y: number;
  // Term annotations folded into this node, rendered as in-card chips.
  chips?: WhiteboardLayoutChip[];
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
  trunk: boolean;
  redundant: boolean;
};

export type WhiteboardLayoutResult = {
  title: string;
  nodes: WhiteboardLayoutNode[];
  edges: WhiteboardLayoutEdge[];
  // Pixel canvas the layout was computed against; the renderer sizes the
  // stage to match so the 0..100 coordinates scale without distortion.
  stage?: { width: number; height: number } | null;
};

export type WhiteboardLayoutTopic = {
  id: string;
  label: string;
};

export type WhiteboardLayoutOptions = {
  fallbackBoardTitle?: string;
  externalNodeLabel?: string;
  // When set, only nodes belonging to these main topics are laid out.
  topicIds?: string[];
};

declare global {
  interface Window {
    WhiteboardLayout?: {
      compute(
        board: unknown,
        options?: WhiteboardLayoutOptions,
      ): WhiteboardLayoutResult | null;
      topics(board: unknown): WhiteboardLayoutTopic[];
    };
  }
}

// Cache layouts by whiteboard object identity. The upstream `snapshot` object
// is replaced on every transcript chunk in Live mode, but the per-summary
// `whiteboard` reference is stable as long as the summary itself hasn't
// changed — so we can skip the layout and label scoring entirely when nothing
// material is different. WeakMap means cached entries are GC'd as soon as
// the underlying summary is dropped from the snapshot, no manual bookkeeping.
const layoutCache = new WeakMap<object, Map<string, WhiteboardLayoutResult | null>>();

function makeOptionsKey(options?: WhiteboardLayoutOptions): string {
  // Preserve field / topic boundaries even when labels and IDs contain the
  // old delimiters. Omitted and empty options have the same compute defaults.
  return JSON.stringify([
    options?.fallbackBoardTitle ?? "",
    options?.externalNodeLabel ?? "",
    options?.topicIds ?? [],
  ]);
}

export function whiteboardTopics(
  board: LiveWhiteboard | null,
): WhiteboardLayoutTopic[] {
  if (!board) return [];
  const impl = typeof window !== "undefined" ? window.WhiteboardLayout : undefined;
  if (!impl || typeof impl.topics !== "function") return [];
  return impl.topics(board);
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
  const cached = layoutCache.get(boardKey) ?? new Map<string, WhiteboardLayoutResult | null>();
  if (cached.has(optsKey)) return cached.get(optsKey)!;
  let result: WhiteboardLayoutResult | null = null;
  try {
    result = impl.compute(board, options);
  } catch (err) {
    console.warn("[Selah] whiteboard layout failed:", err);
  }
  // Preview and expanded board use different options. Retain both instead of
  // evicting one another on each render; bound topic combinations per board.
  if (cached.size >= 8) cached.delete(cached.keys().next().value!);
  cached.set(optsKey, result);
  layoutCache.set(boardKey, cached);
  return result;
}
