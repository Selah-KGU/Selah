export type DeckCard<T> = { item: T; index: number; offset: number };

/** Project the existing cyclic stack without mounting its hidden cards. */
export function visibleDeckCards<T>(items: readonly T[], front: number): DeckCard<T>[] {
  const total = items.length;
  if (!total || !Number.isSafeInteger(front)) return [];
  const start = ((front % total) + total) % total;
  const cards: DeckCard<T>[] = [];
  for (let step = 0; step < Math.min(total, 3); step++) {
    const index = (start + step) % total;
    // Use the original remainder rule, including temporary out-of-range state
    // before a segment-change effect resets/clamps the front card.
    const offset = (index - front + total) % total;
    if (offset >= 0 && offset <= 2) cards.push({ item: items[index], index, offset });
  }
  // Preserve the original DOM order and keys across the cyclic wrap.
  return cards.sort((a, b) => a.index - b.index);
}
