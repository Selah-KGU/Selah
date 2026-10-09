import { invoke } from "@tauri-apps/api/core";
import { BACKEND_CACHE_DB_KEYS } from "./backendCacheKeys";

// ============ Data Cache ============
// Unified caching layer: memory + disk (localStorage) + stale-while-revalidate
//
// Usage:  data = await cachedFetch("key", fetcher)
// SWR:    onCacheUpdate("key", (fresh) => { data = fresh })
//
// To add a new cached endpoint:
//   1. Add TTL to CACHE_TTLS (optional, defaults to 5 min)
//   2. Add key to DISK_CACHE_KEYS if it should persist across restarts

export type CacheStamp = string | number;

interface MemoryCacheEntry {
  data: any;
  ts: number;
  stamp?: CacheStamp;
}

const cache = new Map<string, MemoryCacheEntry>();
const rawCache = new Map<string, { updatedAt: number; revision?: number; parsed: unknown }>();
const inflight = new Map<string, Promise<any>>();
const backendLoads = new Map<string, Promise<MemoryCacheEntry | null>>();
const backendRefreshes = new Map<string, { force: boolean; promise: Promise<any> }>();
let nextGeneration = 0;
let globalGeneration = 0;
const keyGenerations = new Map<string, number>();

/** Invalidations advance the generation; late reads must not restore old state. */
export function getCacheGeneration(key: string): number {
  return keyGenerations.get(key) ?? globalGeneration;
}

function ensureGeneration(key: string, generation: number): void {
  if (getCacheGeneration(key) !== generation) throw new Error(`Cache invalidated while loading "${key}"`);
}

const DEFAULT_TTL = 5 * 60 * 1000; // 5 minutes
// Memory TTL mirrors the Rust cache max-age. SQLite is authoritative; a shorter
// TTL here only causes backend_refresh_now calls that the backend then skips.
// Keep these in sync with background_refresh.rs / notifier.rs.
const CACHE_TTLS: Record<string, number> = {
  // KG-Course. schedule: SCHEDULE_CACHE_MAX_AGE_SECS (6h)
  schedule_data: 6 * 60 * 60 * 1000,
  grades: 72 * 60 * 60 * 1000,
  exams: 12 * 60 * 60 * 1000,
  registration: 72 * 60 * 60 * 1000,
  // STABLE_CACHE_MAX_AGE_SECS
  cancellations: 12 * 60 * 60 * 1000,
  makeup: 12 * 60 * 60 * 1000,
  rooms: 12 * 60 * 60 * 1000,
  // KGC_NOTIFICATION_MAX_AGE_SECS
  notifications: 12 * 60 * 60 * 1000,
  profile: 12 * 60 * 60 * 1000,
  student_profile: 12 * 60 * 60 * 1000,
  favorites: 10 * 60 * 1000,
  // FAST_CACHE_MAX_AGE_SECS / FAST_SOURCE_MAX_AGE_SECS
  luna_todo: 5 * 60 * 1000,
  luna_updates: 5 * 60 * 1000,
  weather: 60 * 60 * 1000,
  mail_inbox: 5 * 60 * 1000,
  kwic_home: 5 * 60 * 1000,
};

// Keys eligible for disk persistence (survive app restart, stale-while-revalidate)
// Only first-screen data needs synchronous localStorage; others rely on SQLite fallback.
const DISK_CACHE_KEYS = new Set([
  "schedule_data", "kwic_home",
  "notifications", "luna_updates", "luna_todo",
]);

// Keys eligible for SQLite DB persistence (async SWR).
// The Rust backend already saves these on successful fetch via save_data_cache,
// so we only need to *read* from DB on cold start — no frontend writes needed.
const DB_CACHE_KEYS = new Set([
  "grades", "registration",
  "kwic_home", "notifications", "luna_updates", "luna_todo",
  "cancellations", "makeup", "rooms", "mail_inbox",
  "weather", "student_profile", "ai_notif_analysis", "ai_todo_analysis",
]);
const DISK_PREFIX = "selah_cache_";
const DISK_CACHE_VERSION = 1;
const DISK_MAX_AGE = 7 * 24 * 60 * 60 * 1000;

interface DiskEntry { v: number; data: any; ts: number }

function loadDiskCache(key: string): { data: any; ts: number } | null {
  try {
    const raw = localStorage.getItem(DISK_PREFIX + key);
    if (!raw) return null;
    const parsed: DiskEntry = JSON.parse(raw);
    if (parsed.v !== DISK_CACHE_VERSION) return null;
    if (Date.now() - parsed.ts > DISK_MAX_AGE) return null;
    return { data: parsed.data, ts: parsed.ts };
  } catch { return null; }
}

function saveDiskCache(key: string, data: any, ts: number) {
  try {
    const entry: DiskEntry = { v: DISK_CACHE_VERSION, data, ts };
    localStorage.setItem(DISK_PREFIX + key, JSON.stringify(entry));
  } catch { /* quota exceeded */ }
}

// SWR update listeners: components subscribe to be notified when background refresh completes
const swrListeners = new Map<string, Set<(data: any) => void>>();
export function onCacheUpdate<T>(key: string, cb: (data: T) => void): () => void {
  if (!swrListeners.has(key)) swrListeners.set(key, new Set());
  swrListeners.get(key)!.add(cb as (data: any) => void);
  return () => {
    const set = swrListeners.get(key);
    if (set) {
      set.delete(cb as (data: any) => void);
      if (set.size === 0) swrListeners.delete(key);
    }
  };
}

function notifySwr(key: string, data: any) {
  swrListeners.get(key)?.forEach((cb) => { try { cb(data); } catch { /* ignore */ } });
}

function stableCacheJson(data: unknown): string | null {
  try {
    return JSON.stringify(data);
  } catch {
    return null;
  }
}

export function isEmptyNotificationsPayload(data: unknown): boolean {
  if (!data || typeof data !== "object") return false;
  const entries = (data as { entries?: unknown }).entries;
  return Array.isArray(entries) && entries.length === 0;
}

export function isCacheFresh(key: string, ttl?: number): boolean {
  const entry = cache.get(key);
  if (!entry) return false;
  // An empty campus list is not a successful load. The 12h TTL would otherwise
  // keep a failed parse on screen until the next day.
  if (key === "notifications" && isEmptyNotificationsPayload(entry.data)) return false;
  const effectiveTtl = ttl ?? CACHE_TTLS[key] ?? DEFAULT_TTL;
  return Date.now() - entry.ts < effectiveTtl;
}

export function hasMemoryCache(key: string): boolean {
  return cache.has(key);
}

export function getCacheStamp(key: string): CacheStamp | null {
  const stamp = cache.get(key)?.stamp;
  return stamp == null ? null : stamp;
}

export function touchCacheTimestamp(key: string, ts = Date.now()): boolean {
  const entry = cache.get(key);
  if (!entry) return false;
  cache.set(key, { ...entry, ts });
  return true;
}

export function rememberRawCache(key: string, updatedAt: number, parsed: unknown, revision?: number): void {
  rawCache.set(key, { updatedAt, revision, parsed });
}

export function readRawCache<T>(key: string): T | null {
  const entry = rawCache.get(key);
  return entry ? entry.parsed as T : null;
}

export function hasRawCache(key: string): boolean {
  return rawCache.has(key);
}

export function knownRawUpdatedAt(key: string): number | null {
  const entry = rawCache.get(key);
  return entry ? entry.updatedAt : null;
}

export function knownRawRevision(key: string): number | null {
  return rawCache.get(key)?.revision ?? null;
}

function setUnversionedCache(key: string, data: unknown, ts: number): void {
  rawCache.delete(BACKEND_CACHE_DB_KEYS[key] ?? key);
  cache.set(key, { data, ts });
}

function persistCacheValue<T>(key: string, data: T, ts: number, notify: boolean, stamp?: CacheStamp) {
  const prev = cache.get(key);
  const stampUnchanged = stamp !== undefined && prev?.stamp !== undefined && prev.stamp === stamp;
  if (stampUnchanged && prev) {
    cache.set(key, { ...prev, ts });
    return;
  }
  const prevJson = prev ? stableCacheJson(prev.data) : null;
  const nextJson = stableCacheJson(data);
  const changed = prevJson == null || nextJson == null || prevJson !== nextJson;
  // A local/unversioned write cannot keep claiming the previous backend version.
  if (stamp === undefined) {
    rawCache.delete(BACKEND_CACHE_DB_KEYS[key] ?? key);
  }
  cache.set(key, { data, ts, stamp });
  if (changed && DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, ts);
  if (notify && changed) notifySwr(key, data);
}

function isEmptySchedulePayload(data: any): boolean {
  const raw = data?.raw;
  if (!raw) return true;
  const kgcEmpty = !Array.isArray(raw.kgc_entries_current) || raw.kgc_entries_current.length === 0;
  const lunaEmpty = !Array.isArray(raw.luna_courses) || raw.luna_courses.length === 0;
  const noWeek = !String(raw.current_week_label || "").trim();
  return kgcEmpty && lunaEmpty && noWeek;
}

function readAnyDiskCache<T>(key: string): { data: T; ts: number } | null {
  const disk = loadDiskCache(key);
  if (!disk) return null;
  return { data: disk.data as T, ts: disk.ts };
}

function loadBackendManagedCache<T>(key: string): Promise<MemoryCacheEntry | null> {
  const pending = backendLoads.get(key);
  if (pending) return pending;
  const generation = getCacheGeneration(key);
  const read = (async () => {
    // The adapter imports cacheStore. Import it on demand after module setup,
    // so cold reads and event reads use one queue without an initialization cycle.
    const { syncBackendManagedKeys } = await import("./backendCacheSync");
    ensureGeneration(key, generation);
    await syncBackendManagedKeys([key]);
    ensureGeneration(key, generation);
    return cache.get(key) ?? null;
  })().finally(() => {
    if (backendLoads.get(key) === read) backendLoads.delete(key);
  });
  backendLoads.set(key, read);
  return read;
}

async function backendRowAgeMs(key: string, data?: unknown): Promise<number | null> {
  if (key === "schedule_data") {
    const updated = (data as { snapshot_updated_at?: number } | null)?.snapshot_updated_at;
    if (typeof updated === "number" && updated > 0) return Date.now() - updated * 1000;
    return null;
  }
  const dbKey = BACKEND_CACHE_DB_KEYS[key] ?? key;
  const knownTimestamp = knownRawUpdatedAt(dbKey);
  if (knownTimestamp != null && knownTimestamp > 0) return Date.now() - knownTimestamp * 1000;
  try {
    const updatedAt = await invoke<number | null>("get_data_cache_updated_at", { key: dbKey });
    if (typeof updatedAt === "number" && updatedAt > 0) return Date.now() - updatedAt * 1000;
  } catch {
    return null;
  }
  return null;
}

function refreshBackendIfStale<T>(key: string, data: T, ttl: number) {
  if (key === "notifications" && (isEmptyNotificationsPayload(data) || !isCacheFresh(key, ttl))) {
    void queueBackendManagedRefresh<T>(key, true, data).catch(() => {});
    return;
  }
  const generation = getCacheGeneration(key);
  void backendRowAgeMs(key, data).then((age) => {
    if (getCacheGeneration(key) !== generation) return;
    if (age != null && age < ttl) return;
    void queueBackendManagedRefresh<T>(key, false, data).catch(() => {});
  }).catch(() => {});
}

function queueBackendManagedRefresh<T>(key: string, force: boolean, fallback?: T): Promise<T> {
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") {
    const entry = cache.get(key);
    if (entry) return Promise.resolve(entry.data as T);
    const disk = readAnyDiskCache<T>(key);
    if (disk) {
      persistCacheValue(key, disk.data, disk.ts, false);
      return Promise.resolve(disk.data);
    }
    if (fallback !== undefined) return Promise.resolve(fallback);
    return Promise.reject(new Error(`No demo cache available for "${key}"`));
  }

  const generation = getCacheGeneration(key);
  const pending = backendRefreshes.get(key);
  if (pending) {
    if (force && !pending.force) {
      const forceAfterCurrent = () => {
        ensureGeneration(key, generation);
        return queueBackendManagedRefresh<T>(key, true, fallback);
      };
      // A user-requested refresh must not inherit an automatic freshness skip.
      return pending.promise.then(forceAfterCurrent, forceAfterCurrent);
    }
    return pending.promise as Promise<T>;
  }

  const refreshPromise = invoke<string[]>("backend_refresh_now", { keys: [key], force })
    .then(async () => {
      ensureGeneration(key, generation);
      const loaded = await loadBackendManagedCache<T>(key);
      ensureGeneration(key, generation);
      if (!loaded) {
        if (fallback !== undefined) return fallback;
        throw new Error(`No backend cache available for "${key}"`);
      }
      if (
        key === "notifications"
        && isEmptyNotificationsPayload(loaded.data)
        && fallback !== undefined
        && !isEmptyNotificationsPayload(fallback)
      ) {
        return fallback;
      }
      return loaded.data;
    })
    .catch((err) => {
      ensureGeneration(key, generation);
      if (fallback !== undefined) return fallback;
      throw err;
    })
    .finally(() => {
      if (backendRefreshes.get(key)?.promise === refreshPromise) backendRefreshes.delete(key);
    });
  backendRefreshes.set(key, { force, promise: refreshPromise });
  return refreshPromise;
}

export async function cachedBackendFetch<T>(key: string, ttl?: number): Promise<T> {
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") {
    const demo = getCached<T>(key);
    if (demo != null) return demo;
    throw new Error(`No demo cache available for "${key}"`);
  }

  const effectiveTtl = ttl ?? CACHE_TTLS[key] ?? DEFAULT_TTL;
  const generation = getCacheGeneration(key);
  const entry = cache.get(key);
  // A disk preview has no backend stamp. Validate it once, even when its
  // local timestamp looks fresh; second-resolution SQLite times prove no equality.
  if (entry?.stamp !== undefined && isCacheFresh(key, effectiveTtl)) return entry.data as T;
  if (entry?.stamp !== undefined && key !== "schedule_data") {
    void queueBackendManagedRefresh<T>(key, key === "notifications" && isEmptyNotificationsPayload(entry.data), entry.data)
      .catch(() => {});
    return entry.data as T;
  }

  const preview = entry ?? loadDiskCache(key);
  if (preview && !entry) persistCacheValue(key, preview.data, preview.ts, false);
  try {
    const loaded = await loadBackendManagedCache<T>(key);
    ensureGeneration(key, generation);
    if (loaded) {
      refreshBackendIfStale(key, loaded.data, effectiveTtl);
      return loaded.data as T;
    }
  } catch (error) {
    ensureGeneration(key, generation);
    // Keep the offline preview when the local IPC read fails. Successful reads
    // are authoritative and already applied by the shared synchronizer.
    if (preview) {
      if (!cache.has(key)) persistCacheValue(key, preview.data, preview.ts, false);
      void queueBackendManagedRefresh<T>(key, false, preview.data).catch(() => {});
      return preview.data as T;
    }
    throw error;
  }
  return queueBackendManagedRefresh<T>(key, true, preview?.data);
}

export function refreshBackendManagedCache<T>(key: string): Promise<T> {
  return queueBackendManagedRefresh<T>(key, true);
}

/**
 * Fetch data with caching, dedup, and optional stale-while-revalidate.
 *
 * Flow:
 * 1. If memory cache hit and fresh → return immediately
 * 2. If disk cache available (cold start) → return stale, revalidate in background
 * 3. Otherwise → fetch, cache result, return
 *
 * Background SWR refresh errors are silently swallowed (stale data is kept).
 * Components should subscribe via onCacheUpdate() for live refreshes.
 */
export function cachedFetch<T>(key: string, fetcher: () => Promise<T>, ttl?: number): Promise<T> {
  const generation = getCacheGeneration(key);
  // Demo mode: always serve from cache, never hit network
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") {
    const entry = cache.get(key);
    if (entry) return Promise.resolve(entry.data as T);
    const disk = loadDiskCache(key);
    if (disk) { cache.set(key, disk); return Promise.resolve(disk.data as T); }
    return fetcher().then((data) => {
      ensureGeneration(key, generation);
      const now = Date.now();
      setUnversionedCache(key, data, now);
      if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, now);
      return data;
    });
  }

  const effectiveTtl = ttl ?? CACHE_TTLS[key] ?? DEFAULT_TTL;
  const entry = cache.get(key);
  if (entry && Date.now() - entry.ts < effectiveTtl) {
    return Promise.resolve(entry.data as T);
  }
  // Dedup: if the same key is already being fetched, share the promise
  // but if it resolves with no data (background refresh failed), do our own fetch
  const pending = inflight.get(key);
  if (pending) return (pending as Promise<T>).then((data) => {
    ensureGeneration(key, generation);
    if (data != null) return data;
    // Re-enter deduplication so failure does not fan out into one request per waiter.
    return cachedFetch<T>(key, fetcher, 0);
  });

  // Stale-while-revalidate: if disk cache exists, return stale data immediately
  if (DISK_CACHE_KEYS.has(key) && !entry) {
    const disk = loadDiskCache(key);
    if (disk) {
      cache.set(key, disk);
      // Background refresh (fire-and-forget, errors are swallowed)
      const bg = fetcher().then((data) => {
        ensureGeneration(key, generation);
        // Guard: don't overwrite good cache with empty schedule data
        if (key === "schedule_data") {
          const sr = data as any;
          if (isEmptySchedulePayload(sr)) {
            console.warn(`[Selah] SWR: "${key}" returned empty data, keeping stale cache`);
            return disk.data as T;
          }
        }
        const now = Date.now();
        setUnversionedCache(key, data, now);
        saveDiskCache(key, data, now);
        notifySwr(key, data);
        return data;
      }).catch((err) => {
        if (getCacheGeneration(key) !== generation) return undefined as unknown as T;
        console.warn(`[Selah] SWR background refresh failed for "${key}":`, err);
        // Still notify listeners with the stale data so UI stays consistent
        return disk.data as T;
      }).finally(() => { if (inflight.get(key) === bg) inflight.delete(key); });
      inflight.set(key, bg);
      return Promise.resolve(disk.data as T);
    }
  }

  // SQLite SWR: async DB read → return stale, revalidate in background
  if (DB_CACHE_KEYS.has(key) && !entry) {
    const dbSwr = invoke<string | null>("get_data_cache", { key }).then((json) => {
      if (!json) return null;
      try { return JSON.parse(json) as T; } catch { return null; }
    }).catch(() => null).then((dbData) => {
      ensureGeneration(key, generation);
      if (dbData != null) {
        const now = Date.now();
        setUnversionedCache(key, dbData, now);
        // Persist to localStorage so getCached() can find it synchronously next time
        if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, dbData, now);
        // Background refresh (Rust saves to DB on success automatically)
        // Replace the inflight entry with the bg promise so further callers
        // dedup against the refresh, not the already-resolved DB read.
        const bg = fetcher().then((freshData) => {
          ensureGeneration(key, generation);
          const ts = Date.now();
          setUnversionedCache(key, freshData, ts);
          if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, freshData, ts);
          notifySwr(key, freshData);
          return freshData;
        }).catch((err) => {
          if (getCacheGeneration(key) !== generation) return undefined as unknown as T;
          console.warn(`[Selah] DB-SWR background refresh failed for "${key}":`, err);
          return dbData;
        }).finally(() => { if (inflight.get(key) === bg) inflight.delete(key); });
        inflight.set(key, bg);
        return dbData;
      }
      // No DB cache — fall through to normal fetch
      return fetcher().then((data) => {
        ensureGeneration(key, generation);
        const ts = Date.now();
        setUnversionedCache(key, data, ts);
        if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, ts);
        return data;
      });
    });
    // Store outer promise immediately so refreshCache deduplicates against it.
    // Must capture the .finally() promise in a variable so the === check works
    // (.finally() creates a new promise object, different from dbSwr).
    const inflightEntry = dbSwr.finally(() => {
      if (inflight.get(key) === inflightEntry) inflight.delete(key);
    });
    inflight.set(key, inflightEntry);
    return inflightEntry;
  }

  const p = fetcher().then((data) => {
    ensureGeneration(key, generation);
    const now = Date.now();
    setUnversionedCache(key, data, now);
    if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, now);
    return data;
  }).finally(() => {
    if (inflight.get(key) === p) inflight.delete(key);
  });
  inflight.set(key, p);
  return p;
}

export function getCacheTimestamp(key: string): number | null {
  const entry = cache.get(key);
  return entry ? entry.ts : null;
}

/** Read cached data (memory or disk) without triggering a fetch */
export function getCached<T>(key: string): T | null {
  const entry = cache.get(key);
  if (entry) return entry.data as T;
  if (DISK_CACHE_KEYS.has(key)) {
    const disk = loadDiskCache(key);
    if (disk) {
      cache.set(key, disk);
      return disk.data as T;
    }
  }
  return null;
}

/** A successful backend deletion drops data without canceling its own reader. */
export function discardCacheEntry(key: string) {
  cache.delete(key);
  rawCache.delete(key);
  rawCache.delete(BACKEND_CACHE_DB_KEYS[key] ?? key);
  try { localStorage.removeItem(DISK_PREFIX + key); } catch {}
}

export function invalidateCache(key?: string) {
  const generation = ++nextGeneration;
  if (key) {
    keyGenerations.set(key, generation);
    inflight.delete(key);
    backendLoads.delete(key);
    backendRefreshes.delete(key);
    discardCacheEntry(key);
  } else {
    globalGeneration = generation;
    keyGenerations.clear();
    cache.clear();
    inflight.clear();
    backendLoads.clear();
    backendRefreshes.clear();
    rawCache.clear();
    for (const k of DISK_CACHE_KEYS) { try { localStorage.removeItem(DISK_PREFIX + k); } catch {} }
  }
}

/** Update a cached entry in-place and notify SWR listeners. */
export function updateCacheEntry<T>(key: string, updater: (data: T) => T): void {
  const entry = cache.get(key);
  if (!entry) return;
  const updated = updater(entry.data as T);
  const now = Date.now();
  cache.set(key, { data: updated, ts: now, stamp: entry.stamp });
  if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, updated, now);
  notifySwr(key, updated);
}

export function replaceCacheEntry<T>(key: string, data: T, ts: number = Date.now(), stamp?: CacheStamp): void {
  persistCacheValue(key, data, ts, true, stamp);
}

/**
 * Force-refresh a cached key in the background. Deduped with inflight map.
 * On success, updates cache + disk + notifies SWR listeners.
 * On failure, silently swallowed (stale data retained).
 */
export function refreshCache<T>(key: string, fetcher: () => Promise<T>): Promise<T> | null {
  const generation = getCacheGeneration(key);
  if (inflight.has(key)) return null; // already refreshing
  const p = fetcher().then((data) => {
    ensureGeneration(key, generation);
    const now = Date.now();
    setUnversionedCache(key, data, now);
    if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, now);
    notifySwr(key, data);
    return data;
  }).catch((err) => {
    if (getCacheGeneration(key) !== generation) return undefined as unknown as T;
    console.warn(`[Selah] Background refresh failed for "${key}":`, err);
    return undefined as unknown as T;
  }).finally(() => { if (inflight.get(key) === p) inflight.delete(key); });
  inflight.set(key, p);
  return p;
}
