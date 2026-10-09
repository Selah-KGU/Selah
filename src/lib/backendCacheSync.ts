import { invoke } from "@tauri-apps/api/core";
import type { ScheduleResponse } from "./types";
import { createCacheSyncQueue } from "./cacheSyncQueue";
import { BACKEND_CACHE_DB_KEYS as BACKEND_CACHE_DB_KEY } from "./backendCacheKeys";
import { cacheStatus, aiNotifStore, aiTodoStore } from "./stores";
import { discardCacheEntry, replaceCacheEntry, getCached, isCacheFresh, getCacheGeneration,
  isEmptyNotificationsPayload, hasMemoryCache, getCacheStamp, touchCacheTimestamp,
  rememberRawCache, readRawCache, hasRawCache, knownRawRevision } from "./cacheStore";
import { DETAIL_GENERATED_TODO_KEY, LIVE_GENERATED_TODO_KEY, mergeDetailTodosIntoLunaTodos,
  mergeGeneratedTodosIntoLunaTodos, mergeGeneratedTodosIntoSchedule,
  repairDetailGeneratedTodoSourceUrls } from "./generatedTodoSupport";

const AI_TODO_ANALYSIS_TTL_SECS = 12 * 3600;
function _isDemo(): boolean {
  try { return localStorage.getItem("selah-demo-mode") === "1"; } catch { return false; }
}
function readDataCache(key: string): Promise<string | null> {
  return invoke("get_data_cache", { key });
}
async function writeDataCache(key: string, json: string): Promise<void> {
  await invoke("save_data_cache", { key, json });
}

const queuedGenerations = new Map<string, number>();
const syncQueue = createCacheSyncQueue((keys, onlyIfStale) => {
  const generations = new Map(keys.map(key => [key, queuedGenerations.get(key)]));
  keys.forEach(key => queuedGenerations.delete(key));
  return applyBackendManagedKeys(keys, onlyIfStale, generations);
});

export function syncBackendManagedKeys(keys: string[], onlyIfStale = false): Promise<void> {
  keys.filter(Boolean).forEach(key => queuedGenerations.set(key, getCacheGeneration(key)));
  return syncQueue(keys, onlyIfStale);
}

interface CacheBatchRow {
  key: string;
  updated_at: number;
  revision: number;
  unchanged: boolean;
  json?: string | null;
}

interface FrontendCacheBatch {
  rows: CacheBatchRow[];
  schedule_updated_at: number;
  live_todo_updated_at: number;
  schedule_revision: number;
  live_todo_revision: number;
  schedule_unchanged: boolean;
  schedule_stamp: string;
  schedule?: ScheduleResponse | null;
}

function cacheDbKey(key: string): string {
  return BACKEND_CACHE_DB_KEY[key] ?? key;
}

function parseCacheJson<T>(json: string | null | undefined, key: string): T | null {
  if (!json) return null;
  try {
    return JSON.parse(json) as T;
  } catch (e) {
    console.warn("[Selah] backend cache parse failed for " + key + ":", e);
    return null;
  }
}

function parseTodoArray(json: string | null | undefined): any[] {
  const parsed = parseCacheJson<unknown>(json, "generated_todo");
  return Array.isArray(parsed) ? parsed : [];
}

function rowByKey(rows: CacheBatchRow[], key: string): CacheBatchRow | undefined {
  return rows.find((row) => row.key === key);
}

function knownScheduleStamp(): string | null {
  if (!hasMemoryCache("schedule_data")) return null;
  const stamp = getCacheStamp("schedule_data");
  return typeof stamp === "string" ? stamp : null;
}

function knownRevisionFor(dbKey: string, memoryKey: string): number | null {
  if (!hasMemoryCache(memoryKey) || !hasRawCache(dbKey)) return null;
  return knownRawRevision(dbKey);
}

function liveTodosFromRow(row: CacheBatchRow | undefined): any[] {
  if (row && row.unchanged && hasRawCache(LIVE_GENERATED_TODO_KEY)) {
    return readRawCache<any[]>(LIVE_GENERATED_TODO_KEY) ?? [];
  }
  if (row && !row.unchanged && row.revision === 0) {
    rememberRawCache(LIVE_GENERATED_TODO_KEY, 0, [], 0);
    return [];
  }
  if (!row || row.json == null) return readRawCache<any[]>(LIVE_GENERATED_TODO_KEY) ?? [];
  const parsed = parseTodoArray(row.json);
  rememberRawCache(LIVE_GENERATED_TODO_KEY, row.updated_at, parsed, row.revision);
  return parsed;
}

async function detailTodosFromRow(row: CacheBatchRow | undefined, isCurrent: () => boolean): Promise<any[]> {
  if (row && row.unchanged && hasRawCache(DETAIL_GENERATED_TODO_KEY)) {
    return readRawCache<any[]>(DETAIL_GENERATED_TODO_KEY) ?? [];
  }
  if (row && !row.unchanged && row.revision === 0) {
    rememberRawCache(DETAIL_GENERATED_TODO_KEY, 0, [], 0);
    return [];
  }
  if (!row || row.json == null) return readRawCache<any[]>(DETAIL_GENERATED_TODO_KEY) ?? [];
  const parsed = parseTodoArray(row.json);
  const repaired = await repairDetailGeneratedTodoSourceUrls(parsed,
    key => isCurrent() ? readDataCache(key) : Promise.resolve(null),
    (key, json) => isCurrent() ? writeDataCache(key, json) : Promise.resolve());
  if (!isCurrent()) return [];
  rememberRawCache(DETAIL_GENERATED_TODO_KEY, row.updated_at, repaired, row.revision);
  return repaired;
}

async function applyBackendManagedKeys(keys: string[], onlyIfStale: boolean, generations: Map<string, number | undefined>): Promise<void> {
  const isCurrent = (key: string) => generations.get(key) === getCacheGeneration(key);
  const uniqueKeys = [...new Set(keys.filter(Boolean))];
  if (!uniqueKeys.length || _isDemo()) return;
  const pending = uniqueKeys.filter((key) => isCurrent(key) && !(onlyIfStale && isCacheFresh(key, 5 * 60 * 1000)));
  if (!pending.length) return;

  const includeSchedule = pending.includes("schedule_data");
  const needsLiveTodos = includeSchedule || pending.includes("luna_todo");
  const needsDetailTodos = pending.includes("luna_todo");
  const queries: Array<{ key: string; knownRevision: number | null }> = [];
  const seenQuery = new Set<string>();
  const pushQuery = (dbKey: string, knownRevision: number | null) => {
    if (seenQuery.has(dbKey)) return;
    seenQuery.add(dbKey);
    queries.push({ key: dbKey, knownRevision });
  };

  for (const key of pending) {
    if (key === "schedule_data") continue;
    pushQuery(cacheDbKey(key), knownRevisionFor(cacheDbKey(key), key));
  }
  if (needsLiveTodos) {
    pushQuery(
      LIVE_GENERATED_TODO_KEY,
      hasRawCache(LIVE_GENERATED_TODO_KEY) ? knownRawRevision(LIVE_GENERATED_TODO_KEY) : null,
    );
  }
  if (needsDetailTodos) {
    pushQuery(
      DETAIL_GENERATED_TODO_KEY,
      hasRawCache(DETAIL_GENERATED_TODO_KEY) ? knownRawRevision(DETAIL_GENERATED_TODO_KEY) : null,
    );
  }

  const batch = await invoke<FrontendCacheBatch>("get_frontend_cache_batch", {
    queries,
    includeSchedule,
    knownScheduleStamp: includeSchedule ? knownScheduleStamp() : null,
  });
  const rows = batch.rows ?? [];

  if (includeSchedule && isCurrent("schedule_data")) {
    const stamp = batch.schedule_stamp;
    if (batch.schedule_unchanged && hasMemoryCache("schedule_data")) {
      touchCacheTimestamp("schedule_data");
    } else if (batch.schedule) {
      const generated = liveTodosFromRow(rowByKey(rows, LIVE_GENERATED_TODO_KEY));
      replaceCacheEntry(
        "schedule_data",
        mergeGeneratedTodosIntoSchedule(batch.schedule, generated),
        Date.now(),
        stamp,
      );
    }
  }

  if (pending.includes("luna_todo") && isCurrent("luna_todo")) {
    const lunaRow = rowByKey(rows, "luna_todo");
    const liveRow = rowByKey(rows, LIVE_GENERATED_TODO_KEY);
    const detailRow = rowByKey(rows, DETAIL_GENERATED_TODO_KEY);
    const liveChanged = !!liveRow && !liveRow.unchanged;
    const detailChanged = !!detailRow && !detailRow.unchanged;
    if (lunaRow?.unchanged && !liveChanged && !detailChanged && hasMemoryCache("luna_todo")) {
      touchCacheTimestamp("luna_todo");
    } else {
      const generated = liveTodosFromRow(liveRow);
      const detail = await detailTodosFromRow(detailRow, () => isCurrent("luna_todo"));
      if (isCurrent("luna_todo")) {
        let base: unknown = [];
        if (lunaRow?.unchanged && hasMemoryCache("luna_todo")) {
          base = readRawCache("luna_todo") ?? [];
        } else if (lunaRow?.json) {
          const parsed = parseCacheJson<unknown>(lunaRow.json, "luna_todo");
          base = Array.isArray(parsed) ? parsed : [];
          if (parsed != null) rememberRawCache("luna_todo", lunaRow.updated_at, parsed, lunaRow.revision);
        } else if (lunaRow?.revision === 0) {
          rememberRawCache("luna_todo", 0, [], 0);
        }
        const nothingStored = !lunaRow?.json && !lunaRow?.unchanged && generated.length === 0 && detail.length === 0;
        if (!nothingStored || hasMemoryCache("luna_todo")) {
          const merged = mergeDetailTodosIntoLunaTodos(
            mergeGeneratedTodosIntoLunaTodos(base, generated),
            detail,
          );
          const stamp = "v2:" + String(lunaRow?.revision ?? 0) + ":" + String(liveRow?.revision ?? 0) + ":" + String(detailRow?.revision ?? 0);
          replaceCacheEntry("luna_todo", merged, Date.now(), stamp);
        }
      }
    }
  }

  for (const key of pending) {
    if (!isCurrent(key)) continue;
    if (key === "schedule_data" || key === "luna_todo") continue;
    const dbKey = cacheDbKey(key);
    const row = rowByKey(rows, dbKey);
    if (!row) continue;
    if (row.unchanged && hasMemoryCache(key)) {
      touchCacheTimestamp(key);
      if (key === "ai_todo_analysis") {
        const ageSecs = row.updated_at ? Date.now() / 1000 - row.updated_at : Infinity;
        if (ageSecs > AI_TODO_ANALYSIS_TTL_SECS) aiTodoStore.set(null);
      }
      continue;
    }
    if (!row.json) {
      if (!row.unchanged && row.revision === 0) {
        discardCacheEntry(key);
        if (key === "ai_notif_analysis") aiNotifStore.set(null);
        if (key === "ai_todo_analysis") aiTodoStore.set(null);
      }
      continue;
    }
    const data = parseCacheJson<any>(row.json, key);
    if (data == null) continue;
    if (
      key === "notifications"
      && isEmptyNotificationsPayload(data)
      && hasMemoryCache(key)
      && !isEmptyNotificationsPayload(getCached(key))
    ) {
      // A parser failure must not make the retained preview look up-to-date.
      touchCacheTimestamp(key, 0);
      continue;
    }
    rememberRawCache(dbKey, row.updated_at, data, row.revision);
    if (key === "ai_notif_analysis") {
      aiNotifStore.set({
        result: data.result ?? data,
        sources: Array.isArray(data.sources) ? data.sources : [],
        timestamp: typeof data.generated_at === "number" ? data.generated_at * 1000 : Date.now(),
      });
      replaceCacheEntry(key, data, Date.now(), row.revision);
      continue;
    }
    if (key === "ai_todo_analysis") {
      const ageSecs = row.updated_at ? Date.now() / 1000 - row.updated_at : Infinity;
      replaceCacheEntry(key, data, Date.now(), row.revision);
      if (ageSecs > AI_TODO_ANALYSIS_TTL_SECS) {
        aiTodoStore.set(null);
        continue;
      }
      const result = { ...(data as Record<string, unknown>) };
      delete result._cache_fingerprint;
      aiTodoStore.set({ result, timestamp: row.updated_at ? row.updated_at * 1000 : Date.now() });
      continue;
    }
    replaceCacheEntry(key, data, Date.now(), row.revision);
  }

  if (pending.some(isCurrent)) cacheStatus.update((s) => ({ ...s, lastUpdated: Date.now() }));
}
