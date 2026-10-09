import { CoalescedStatusRead } from "./coalescedStatusRead";

interface TaskTimestampKey { key: string; cacheKey: string }
export interface BackendTaskTimestampBatch {
  rows: Array<{ key: string; updated_at: number | null }>;
  schedule_updated_at: number | null;
}

/** A single metadata batch supplies independent, versioned task indicators. */
export class BackendTaskStatusReader {
  private reads = new Map<string, CoalescedStatusRead<number | null>>();
  private version = 0;
  private pending: Promise<Map<string, number | null>> | null = null;

  constructor(
    private tasks: TaskTimestampKey[],
    private read: (keys: string[], includeSchedule: boolean) => Promise<BackendTaskTimestampBatch>,
    apply: (key: string, timestamp: number | null) => void,
  ) {
    for (const task of tasks) {
      this.reads.set(task.key, new CoalescedStatusRead(
        async () => (await this.batch()).get(task.key) ?? null,
        timestamp => apply(task.key, timestamp),
      ));
    }
  }

  async refresh(): Promise<void> {
    await Promise.all([...this.reads.values()].map(read => read.refresh()));
  }

  invalidate(keys?: string[]): void {
    let affected = false;
    for (const key of keys ?? this.reads.keys()) {
      const read = this.reads.get(key);
      if (read) { read.invalidate(); affected = true; }
    }
    if (affected) {
      // A single pushed key retires its own indicator, not the other readers
      // sharing an as-yet unissued batch. Only a full lifetime change cancels IO.
      if (keys === undefined) this.version += 1;
      this.pending = null;
    }
  }

  private batch(): Promise<Map<string, number | null>> {
    if (this.pending) return this.pending;
    const version = this.version;
    const request = Promise.resolve().then(async () => {
      if (version !== this.version) return new Map<string, number | null>();
      const cacheKeys = this.tasks.filter(task => task.key !== "schedule_data").map(task => task.cacheKey);
      const batch = await this.read(cacheKeys, this.reads.has("schedule_data"));
      const stamps = new Map(batch.rows.map(row => [row.key, row.updated_at]));
      return new Map(this.tasks.map(task => {
        const seconds = task.key === "schedule_data" ? batch.schedule_updated_at : stamps.get(task.cacheKey);
        return [task.key, seconds != null && seconds > 0 ? seconds * 1000 : null];
      }));
    }).catch(() => new Map<string, number | null>()).finally(() => {
      if (this.pending === request) this.pending = null;
    });
    this.pending = request;
    return request;
  }
}
