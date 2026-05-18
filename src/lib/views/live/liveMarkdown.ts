import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({ breaks: true, gfm: true });

const renderMdCache = new Map<string, string>();
const RENDER_MD_CACHE_MAX = 128;

export function renderMd(text: string): string {
  const cached = renderMdCache.get(text);
  if (cached !== undefined) return cached;
  const out = DOMPurify.sanitize(marked.parse(text) as string);
  if (renderMdCache.size >= RENDER_MD_CACHE_MAX) {
    const firstKey = renderMdCache.keys().next().value;
    if (firstKey !== undefined) renderMdCache.delete(firstKey);
  }
  renderMdCache.set(text, out);
  return out;
}

export function extractOverallSummary(md: string): string {
  const start = md.indexOf("### 全体要約");
  if (start < 0) return "";
  const afterHeader = md.indexOf("\n", start);
  if (afterHeader < 0) return "";
  const nextSection = md.indexOf("\n###", afterHeader + 1);
  const end = nextSection >= 0 ? nextSection : md.indexOf("\n## ", afterHeader + 1);
  return (end >= 0 ? md.slice(afterHeader + 1, end) : md.slice(afterHeader + 1)).trim();
}
