import { ResourceScope } from "./resourceScope";
import { createCacheSyncQueue } from "./cacheSyncQueue";

/** Keep writes ordered across stop/start generations. A queued obsolete write
 * never publishes, and equal successful items do not reset the native cycle. */
export class TrayStatusWriter {
  private tail = Promise.resolve();
  private lastItems: string[] | null = null;

  constructor(private publish: (items: string[]) => Promise<void>) {}

  write(items: string[], current: () => boolean): Promise<boolean> {
    const captured = [...items];
    const result = this.tail.then(async () => {
      if (!current()) return false;
      if (this.lastItems && captured.length === this.lastItems.length &&
          captured.every((item, index) => item === this.lastItems![index])) return true;
      await this.publish(captured);
      this.lastItems = captured;
      return true;
    });
    // Failure must not poison the next update or the stop-time clear.
    this.tail = result.then(() => {}, () => {});
    return result;
  }
}

interface RefreshOptions<Status> {
  read: () => Promise<Status>;
  build: (status: Status) => string[];
  writer: TrayStatusWriter;
  activity: (patch: { running: boolean; lastRunTs?: number; lastOk?: boolean }) => void;
  failed?: (error: unknown) => void;
}

/** Each run owns its timers and reads. Events invalidate an in-flight read
 * immediately, and the queue merges any burst into one subsequent refresh. */
export class TrayStatusRefresh<Status> {
  private version = 0;
  private cancelTimer: (() => void) | null = null;
  private enqueue: ReturnType<typeof createCacheSyncQueue>;

  constructor(private scope: ResourceScope, private options: RefreshOptions<Status>) {
    this.enqueue = createCacheSyncQueue(async () => {
      if (!scope.active) return;
      const version = this.version;
      const current = () => scope.active && version === this.version;
      options.activity({ running: true });
      try {
        const status = await options.read();
        if (!current()) return;
        const written = await options.writer.write(options.build(status), current);
        if (current() && written) options.activity({ running: false, lastRunTs: Date.now(), lastOk: true });
      } catch (error) {
        if (current()) {
          options.activity({ running: false, lastRunTs: Date.now(), lastOk: false });
          options.failed?.(error);
        }
      } finally {
        if (scope.active) options.activity({ running: false });
      }
    });
    scope.own(() => { this.cancelTimer?.(); this.cancelTimer = null; });
  }

  request(): void {
    if (!this.scope.active) return;
    this.version += 1;
    this.cancelTimer?.();
    this.cancelTimer = this.scope.schedule(() => {
      this.cancelTimer = null;
      void this.enqueue(["tray"]);
    }, 300);
  }
}
