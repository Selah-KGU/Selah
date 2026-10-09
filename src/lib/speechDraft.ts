/** Finals are VAD segments, so each one extends the voice draft. */
export function appendSttFinal(committed: string, segment: string): string {
  const next = segment.trim();
  if (!next) return committed;
  return [committed.trim(), next].filter(Boolean).join(" ");
}

export function mergeSttText(base: string, committed: string, partial: string): string {
  const spoken = [committed.trim(), partial.trim()].filter(Boolean).join(" ");
  if (!spoken) return base;
  if (!base) return spoken;
  return /\s$/.test(base) ? `${base}${spoken}` : `${base}\n${spoken}`;
}
