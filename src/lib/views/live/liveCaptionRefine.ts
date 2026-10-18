export type RefinedCaption = {
  segment_id: number;
  sentence_index?: number;
  corrected: string;
  translation: string;
  ready: boolean;
};

export type RefinedCaptionEvent = {
  session_id?: string;
  segment_id?: number;
  source_seq?: number;
  source?: string;
  corrected?: string;
  translation?: string | null;
  sentences?: {
    source: string;
    corrected: string;
    translation?: string | null;
  }[];
  ready?: boolean;
  show_overlay?: boolean;
  clear?: boolean;
};

const BATCH_DISPLAY_LIMIT = 12;
const SENTENCE_DISPLAY_LIMIT = BATCH_DISPLAY_LIMIT * 4;

export function applyRefinedCaption(
  items: RefinedCaption[],
  event: RefinedCaptionEvent,
): { items: RefinedCaption[]; followDelta: number } {
  if (event.clear) return { items, followDelta: 0 };
  if (!event.segment_id) return { items, followDelta: 0 };
  // Originals already have their own complete transcript. A timeout or an
  // unchanged result must not insert a second copy into translation history.
  const normalized = (text: string) => text.trim().replace(/\s+/g, " ");
  // A model call may contain several unrelated sentences. Its transport batch
  // must never become a paragraph, or erase the original/translation pairing.
  const sentences = event.sentences?.length ? event.sentences : [event];
  let copy = items;
  let followDelta = 0;
  for (const [sentence_index, sentence] of sentences.entries()) {
    const corrected = (sentence.corrected || sentence.source || "").trim();
    const translation = (sentence.translation || "").trim();
    if (!corrected || (!translation && sentence.source && normalized(corrected) === normalized(sentence.source))) continue;
    const next: RefinedCaption = {
      segment_id: event.segment_id,
      sentence_index,
      corrected,
      translation,
      ready: Boolean(event.ready),
    };
    const index = copy.findIndex((item) => item.segment_id === next.segment_id
      && (item.sentence_index ?? 0) === sentence_index);
    if (index >= 0 && copy[index].translation && !translation && !next.ready) continue;
    if (copy === items) copy = items.slice();
    if (index < 0) {
      copy.push(next);
      followDelta += 1;
    } else {
      followDelta += !copy[index].translation && translation ? 1 : 0;
      copy[index] = next;
    }
  }
  if (copy === items) return { items, followDelta };
  // Keep the previous twelve-batch history capacity while displaying each
  // sentence separately. Bound rows too, even for an oversized event.
  const recentBatches = [...new Set(copy.map(item => item.segment_id))].slice(-BATCH_DISPLAY_LIMIT);
  return { items: copy.filter(item => recentBatches.includes(item.segment_id)).slice(-SENTENCE_DISPLAY_LIMIT), followDelta };
}
