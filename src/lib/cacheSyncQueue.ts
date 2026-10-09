type SyncBatch = {
  keys: Set<string>;
  onlyIfStale: boolean;
  completion: Promise<void>;
  resolve: () => void;
  reject: (error: unknown) => void;
};

function createBatch(): SyncBatch {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const completion = new Promise<void>((done, failed) => {
    resolve = done;
    reject = failed;
  });
  return { keys: new Set(), onlyIfStale: true, completion, resolve, reject };
}

/** Merge bursts and serialize reads/applies so an older asynchronous response
 * cannot overwrite a newer cache. Events during a read always get another read.
 * Each queued batch owns one completion, shared by all requests joining it. */
export function createCacheSyncQueue(apply: (keys: string[], onlyIfStale: boolean) => Promise<void>) {
  let running = false;
  let pending: SyncBatch | null = null;

  async function drain() {
    // Collect synchronous event bursts before starting one IPC batch.
    await Promise.resolve();
    while (pending) {
      const batch = pending;
      // Reentrant calls and notifications during apply belong to a new batch.
      pending = null;
      try {
        await apply([...batch.keys], batch.onlyIfStale);
        batch.resolve();
      } catch (error) {
        batch.reject(error);
      }
    }
    running = false;
  }

  return (keys: string[], onlyIfStale = false): Promise<void> => {
    let batch = pending;
    let hasKey = false;
    for (const key of keys) {
      if (!key) continue;
      hasKey = true;
      if (!batch) pending = batch = createBatch();
      batch.keys.add(key);
    }
    // Empty requests neither join an existing batch nor force its refresh.
    if (!hasKey || !batch) return Promise.resolve();
    batch.onlyIfStale &&= onlyIfStale;
    if (!running) {
      running = true;
      void drain();
    }
    return batch.completion;
  };
}
