import { WeightedLruCache } from "./weightedLruCache";

export interface RenderedTextCacheOptions {
  maxEntries?: number;
  /** UTF-16 source + rendered string estimate, excluding JS/DOM overhead. */
  maxBytes?: number;
}

/** Cache complete rendered messages; streaming prefixes bypass this cache. */
export class RenderedTextCache {
  private entries: WeightedLruCache<string, string>;

  constructor(private renderer: (source: string) => string, options: RenderedTextCacheOptions = {}) {
    this.entries = new WeightedLruCache(options);
  }

  get size(): number { return this.entries.size; }
  get estimatedBytes(): number { return this.entries.estimatedBytes; }

  renderTransient(source: string): string {
    return this.renderer(source);
  }

  render(source: string): string {
    const cached = this.entries.get(source);
    if (cached !== undefined) return cached;
    const html = this.renderer(source);
    const bytes = 2 * (source.length + html.length);
    // Large answers still render in full; they do not evict the working set
    // only to exceed the cache budget on their own.
    this.entries.set(source, html, bytes);
    return html;
  }

  clear(): void {
    this.entries.clear();
  }
}
