export interface WeightedLruCacheOptions {
  maxEntries?: number;
  maxBytes?: number;
}

/** Bound retained values by both entry count and a caller-supplied byte estimate. */
export class WeightedLruCache<K, V> {
  private entries = new Map<K, { value: V; bytes: number }>();
  private bytes = 0;
  private readonly maxEntries: number;
  private readonly maxBytes: number;

  constructor(options: WeightedLruCacheOptions = {}) {
    this.maxEntries = options.maxEntries ?? 256;
    this.maxBytes = options.maxBytes ?? 8 * 1024 * 1024;
  }

  get size(): number { return this.entries.size; }
  get estimatedBytes(): number { return this.bytes; }
  keys(): IterableIterator<K> { return this.entries.keys(); }
  has(key: K): boolean { return this.entries.has(key); }

  get(key: K): V | undefined {
    const cached = this.entries.get(key);
    if (!cached) return undefined;
    this.entries.delete(key);
    this.entries.set(key, cached);
    return cached.value;
  }

  set(key: K, value: V, bytes: number): boolean {
    this.delete(key);
    // Oversized values can still be used by the caller in full. Do not evict
    // other entries for a value that cannot fit in the cache on its own.
    if (!Number.isFinite(bytes) || bytes < 0 || !(this.maxEntries > 0) || !(bytes <= this.maxBytes)) return false;
    while (this.entries.size >= this.maxEntries || this.bytes + bytes > this.maxBytes) {
      const oldest = this.entries.keys().next();
      if (oldest.done) break;
      this.delete(oldest.value);
    }
    this.entries.set(key, { value, bytes });
    this.bytes += bytes;
    return true;
  }

  delete(key: K): boolean {
    const old = this.entries.get(key);
    if (!old) return false;
    this.bytes -= old.bytes;
    return this.entries.delete(key);
  }

  clear(): void {
    this.entries.clear();
    this.bytes = 0;
  }
}
