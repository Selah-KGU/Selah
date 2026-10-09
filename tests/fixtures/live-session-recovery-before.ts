  let sessionEventVersion = 0;
  // Keep this watermark when the displayed snapshot becomes a course preview.
  let sessionUpdateRevision = 0;

  function mergeSessionRead(fresh: LiveSessionSnapshot | LiveSurfaceSnapshot) {
    snapshot = mergeLiveSnapshot(snapshot, fresh, isDemoActive() ? 0 : sessionUpdateRevision);
    sessionUpdateRevision = Math.max(sessionUpdateRevision, fresh.update_revision ?? 0);
  }

  const sessionRecovery = createCacheSyncQueue(async () => {
    if (!resources.active) return;
    try {
      const fresh = await liveGetSurface();
      if (!resources.active) return;
      // Full reads and events share capture order, including stop/start.
      mergeSessionRead(fresh);
    } catch (error) {
      if (resources.active) console.warn("[Live] session resync failed:", error);
      throw error;
    }
  });

  function resyncSession(): Promise<void> {
    if (!resources.active) return Promise.resolve();
    // A gap queued during a failed read still gets its own following batch.
    return sessionRecovery(["live_session"]).catch(() => {});
  }
