export type RefinedCaption = {
  segment_id: number;
  corrected: string;
  translation: string;
  ready: boolean;
};

export type RefinedCaptionEvent = {
  session_id?: string;
  segment_id?: number;
  source?: string;
  corrected?: string;
  translation?: string | null;
  ready?: boolean;
  show_overlay?: boolean;
  clear?: boolean;
};

const DISPLAY_LIMIT = 12;

export function applyRefinedCaption(
  items: RefinedCaption[],
  event: RefinedCaptionEvent,
): { items: RefinedCaption[]; followDelta: number } {
  if (event.clear) return { items, followDelta: 0 };
  const corrected = (event.corrected || event.source || "").trim();
  if (!corrected || !event.segment_id) return { items, followDelta: 0 };
  const translation = (event.translation || "").trim();
  const next: RefinedCaption = {
    segment_id: event.segment_id,
    corrected,
    translation,
    ready: Boolean(event.ready),
  };
  const index = items.findIndex((item) => item.segment_id === next.segment_id);
  if (index < 0) {
    return { items: [...items, next].slice(-DISPLAY_LIMIT), followDelta: 1 };
  }
  const followDelta = !items[index].translation && translation ? 1 : 0;
  const copy = items.slice();
  copy[index] = next;
  return { items: copy, followDelta };
}
