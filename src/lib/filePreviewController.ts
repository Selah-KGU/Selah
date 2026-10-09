import { WeightedLruCache } from "./weightedLruCache";

export interface DownloadPreview {
  kind: string;
  mime: string;
  data_url?: string | null;
  text?: string | null;
}

export interface FilePreviewRequest { path: string; key: string }

export function filePreviewRequest(record: {
  path: string; id: string; size_bytes: number; downloaded_at: number; file_exists: boolean;
}): FilePreviewRequest {
  return { path: record.path, key: JSON.stringify([record.path, record.id, record.size_bytes, record.downloaded_at, record.file_exists]) };
}

/** UTF-16 string estimate only; excludes objects, decoded images, DOM and IPC. */
export function previewBytes(key: string, preview: DownloadPreview | null): number {
  return 2 * (key.length + (preview?.kind?.length ?? 0) + (preview?.mime?.length ?? 0)
    + (preview?.data_url?.length ?? 0) + (preview?.text?.length ?? 0));
}

interface Consumer extends FilePreviewRequest {
  references: number;
  preview: DownloadPreview | null | undefined;
}
interface Job extends FilePreviewRequest { running: boolean; valid: boolean }

/** Own the visible consumers separately from the bounded cache of completed IO. */
export class FilePreviewController {
  private readonly cache: WeightedLruCache<string, DownloadPreview | null>;
  private readonly consumers = new Map<string, Consumer>();
  private readonly jobs = new Map<string, Job>();
  private queue: Job[] = [];
  private active = 0;
  private disposed = false;
  private allowed: Set<string> | undefined;
  private readonly maxConcurrent: number;

  constructor(
    private readonly load: (path: string) => Promise<DownloadPreview | null>,
    private readonly publish: (key: string, preview: DownloadPreview | null | undefined) => void,
    options: { maxEntries?: number; maxBytes?: number; maxConcurrent?: number } = {},
  ) {
    this.cache = new WeightedLruCache({ maxEntries: options.maxEntries ?? 128, maxBytes: options.maxBytes ?? 32 * 1024 * 1024 });
    this.maxConcurrent = Math.max(1, Math.floor(options.maxConcurrent ?? 4));
  }

  get stats() {
    let retainedBytes = this.cache.estimatedBytes;
    for (const consumer of this.consumers.values()) {
      if (consumer.preview !== undefined && !this.cache.has(consumer.key)) retainedBytes += previewBytes(consumer.key, consumer.preview);
    }
    return { cacheEntries: this.cache.size, cacheBytes: this.cache.estimatedBytes,
      consumers: this.consumers.size, active: this.active, queued: this.queue.length, retainedBytes };
  }

  hasCached(key: string): boolean { return this.cache.has(key); }

  retain(request: FilePreviewRequest): () => void {
    if (this.disposed || !request.path || (this.allowed && !this.allowed.has(request.key))) return () => {};
    let consumer = this.consumers.get(request.key);
    if (!consumer) {
      consumer = { ...request, references: 0, preview: this.cache.get(request.key) };
      this.consumers.set(request.key, consumer);
      if (consumer.preview !== undefined) this.publish(request.key, consumer.preview);
    }
    consumer.references++;
    if (consumer.preview === undefined && !this.jobs.has(request.key)) {
      const job: Job = { ...request, running: false, valid: true };
      this.jobs.set(job.key, job);
      this.queue.push(job);
      this.pump();
    }
    const owned = consumer;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (this.consumers.get(request.key) !== owned) return;
      if (--owned.references > 0) return;
      this.consumers.delete(request.key);
      this.publish(request.key, undefined);
      const job = this.jobs.get(request.key);
      if (job && !job.running) this.cancel(job);
    };
  }

  private cancel(job: Job): void {
    job.valid = false;
    if (this.jobs.get(job.key) === job) this.jobs.delete(job.key);
    if (!job.running) this.queue = this.queue.filter(item => item !== job);
  }

  private pump(): void {
    if (this.disposed) return;
    while (this.active < this.maxConcurrent && this.queue.length) {
      const job = this.queue.shift()!;
      if (!job.valid || !this.consumers.has(job.key)) { this.cancel(job); continue; }
      job.running = true;
      this.active++;
      void this.fetch(job);
    }
  }

  private async fetch(job: Job): Promise<void> {
    let preview: DownloadPreview | null;
    try { preview = await this.load(job.path) ?? null; }
    catch { preview = null; }
    try {
      if (this.disposed || !job.valid || this.jobs.get(job.key) !== job) return;
      this.cache.set(job.key, preview, previewBytes(job.key, preview));
      const consumer = this.consumers.get(job.key);
      if (consumer) {
        consumer.preview = preview;
        this.publish(job.key, preview);
      }
    } finally {
      if (this.jobs.get(job.key) === job) this.jobs.delete(job.key);
      this.active--;
      this.pump();
    }
  }

  /** Removed records and new versions must not retain or publish old images. */
  prune(keys: Iterable<string>): void {
    if (this.disposed) return;
    this.allowed = new Set(keys);
    for (const key of this.cache.keys()) if (!this.allowed.has(key)) this.cache.delete(key);
    for (const [key] of this.consumers) {
      if (this.allowed.has(key)) continue;
      this.consumers.delete(key);
      this.publish(key, undefined);
    }
    for (const job of this.jobs.values()) if (!this.allowed.has(job.key)) this.cancel(job);
  }

  /** List mode stops queued IO and releases full previews from the display. */
  clearConsumers(): void {
    for (const key of this.consumers.keys()) this.publish(key, undefined);
    this.consumers.clear();
    for (const job of this.queue) {
      job.valid = false;
      this.jobs.delete(job.key);
    }
    this.queue = [];
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.clearConsumers();
    this.cache.clear();
    for (const job of this.jobs.values()) job.valid = false;
    this.jobs.clear();
    this.allowed = undefined;
  }
}
