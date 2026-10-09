import { createMarkdownRenderer } from "../../markdownRenderer";
import DOMPurify from "dompurify";

const renderMdCache = createMarkdownRenderer((html) => DOMPurify.sanitize(html), {
  maxEntries: 128,
  maxBytes: 4 * 1024 * 1024,
});

export function renderMd(text: string): string {
  return renderMdCache.render(text);
}

/**
 * Pull just the bulleted headlines out of a stage-summary body so the compact
 * card can tick through them one at a time. The body is a short bullet list of
 * headlines, then `---`, then `**term**: explanation` detail blocks. Only the
 * bullets are ticked; the detail blocks are reserved for the detail page.
 */
export function splitSummaryHeadlines(body: string): string[] {
  const points: string[] = [];
  for (const block of body.split(/\n{2,}/)) {
    const lines = block.trim().split("\n");
    if (lines.length === 0 || !lines.every((l) => /^\s*[-*]\s+/.test(l))) continue;
    for (const line of lines) {
      const point = line.replace(/^\s*[-*]\s+/, "").trim();
      if (point) points.push(point);
    }
  }
  return points;
}

export interface SummaryDetail {
  /** Leading bullet headlines — the at-a-glance index. */
  overview: string[];
  /** `**heading**: explanation` blocks — the elaboration. */
  sections: { heading: string; text: string }[];
}

/**
 * Parse a stage-summary body into its index (overview bullets) and the detail
 * sections, so the detail page can render a clear primary/secondary hierarchy
 * instead of a flat markdown blob. `structured` is false for bodies that don't
 * follow the chunk shape (e.g. the overall summary) — caller falls back to raw
 * markdown then.
 */
export function parseSummaryDetail(body: string): SummaryDetail & { structured: boolean } {
  const overview: string[] = [];
  const sections: { heading: string; text: string }[] = [];
  for (const block of body.split(/\n{2,}/)) {
    const trimmed = block.trim();
    if (!trimmed || /^-{3,}$/.test(trimmed)) continue;
    const lines = trimmed.split("\n");
    if (lines.every((l) => /^\s*[-*]\s+/.test(l))) {
      for (const line of lines) {
        const point = line.replace(/^\s*[-*]\s+/, "").trim();
        if (point) overview.push(point);
      }
      continue;
    }
    const m = trimmed.match(/^\*\*(.+?)\*\*\s*[:：]\s*([\s\S]+)$/);
    if (m) {
      sections.push({ heading: m[1].trim(), text: m[2].trim() });
    } else {
      sections.push({ heading: "", text: trimmed });
    }
  }
  // Only treat as structured when we actually found titled detail blocks.
  const structured = sections.some((s) => s.heading);
  return { overview, sections, structured };
}

export { extractOverallSummary } from "../../liveSavedSummary";
