import { Marked } from "marked";
import { RenderedTextCache, type RenderedTextCacheOptions } from "./renderedTextCache";

export function createMarkdownRenderer(
  sanitize: (html: string) => string,
  options?: RenderedTextCacheOptions,
): RenderedTextCache {
  // Keep Agent/LIVE options local; visiting a view must not change the global
  // parser used by a document reader in the same WebView.
  const parser = new Marked({ breaks: true, gfm: true });
  return new RenderedTextCache((source) => sanitize(parser.parse(source, { async: false })), options);
}
