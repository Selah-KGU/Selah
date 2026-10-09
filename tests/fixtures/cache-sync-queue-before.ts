/** Merge bursts and serialize reads/applies so an older asynchronous response
 * cannot overwrite a newer cache. Events during a read always get another read. */
export function createCacheSyncQueue(apply: (keys: string[], onlyIfStale: boolean) => Promise<void>) {
  let scheduled = false;
  let pendingKeys = new Set<string>();
  let pendingOnlyIfStale = true;
  let waiters: Array<{ resolve: () => void; reject: (error: unknown) => void }> = [];

  async function drain() {
    // Collect synchronous event bursts before starting one IPC batch.
    await Promise.resolve();
    while (pendingKeys.size) {
      const keys = [...pendingKeys];
      const onlyIfStale = pendingOnlyIfStale;
      const currentWaiters = waiters;
      pendingKeys = new Set();
      pendingOnlyIfStale = true;
      waiters = [];
      try {
        await apply(keys, onlyIfStale);
        currentWaiters.forEach(waiter => waiter.resolve());
      } catch (error) {
        currentWaiters.forEach(waiter => waiter.reject(error));
      }
    }
    scheduled = false;
  }

  return (keys: string[], onlyIfStale = false): Promise<void> => {
    const nonempty = keys.filter(Boolean);
    if (!nonempty.length) return Promise.resolve();
    nonempty.forEach(key => pendingKeys.add(key));
    pendingOnlyIfStale &&= onlyIfStale;
    const completion = new Promise<void>((resolve, reject) => waiters.push({ resolve, reject }));
    if (!scheduled) {
      scheduled = true;
      void drain();
    }
    return completion;
  };
}
