import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { setAppTheme } from "./system";

interface AuthState {
  authenticated: boolean;
  username: string;
  displayName: string;
  studentId: string;
  faculty: string;
  department: string;
  loading: boolean;
  error: string;
}

export const authState = writable<AuthState>({
  authenticated: false,
  username: "",
  displayName: "",
  studentId: "",
  faculty: "",
  department: "",
  loading: false,
  error: "",
});

/** True while an automatic re-login flow is in progress (session expired mid-use) */
export const reloginInProgress = writable(false);

/** True when session has expired but user hasn't re-authenticated yet (soft expired state) */
export const sessionExpired = writable(false);

/** Luna LMS authentication state */
export const lunaAuthState = writable<{ authenticated: boolean }>({
  authenticated: false,
});

/** KWIC Portal authentication state */
export const kwicAuthState = writable<{ authenticated: boolean }>({
  authenticated: false,
});

/** Microsoft 365 Mail authentication state */
export const mailAuthState = writable<{ authenticated: boolean; email: string; displayName: string }>({
  authenticated: false,
  email: "",
  displayName: "",
});

/** Google Calendar authentication state */
interface GoogleCalState {
  authenticated: boolean;
  calendarExists: boolean;
  syncedEvents: number;
}
export const gcalAuthState = writable<GoogleCalState>({
  authenticated: false,
  calendarExists: false,
  syncedEvents: 0,
});

// ============ Data Types ============

export interface StudentInfo {
  student_id: string;
  name: string;
  name_en: string;
  student_type: string;
  affiliation_type: string;
  status: string;
  class: string;
  faculty: string;
  department: string;
  major: string;
  address: string;
}

export interface CurriculumRow {
  category: string;
  level: number;
  required_credits: string;
  enrolled_acquired_credits: string;
  enrolled_credits: string;
  earned_credits: string;
  is_deficit: boolean;
}

export interface GradesData {
  student: StudentInfo;
  curriculum: CurriculumRow[];
}

interface CancellationEntry {
  date: string;
  period: string;
  campus: string;
  department: string;
  course_code: string;
  year: string;
  course_name: string;
  instructor: string;
  room: string;
  comment: string;
}

export interface CancellationsData {
  student: StudentInfo;
  entries: CancellationEntry[];
}

interface MakeupEntry {
  date: string;
  period: string;
  campus: string;
  department: string;
  course_code: string;
  year: string;
  course_name: string;
  instructor: string;
  room: string;
  comment: string;
}

export interface MakeupData {
  student: StudentInfo;
  entries: MakeupEntry[];
}

interface RoomChangeEntry {
  date: string;
  department: string;
  course_code: string;
  year: string;
  course_name: string;
  room: string;
  instructor: string;
  schedule: string;
  comment: string;
}

export interface RoomChangesData {
  student: StudentInfo;
  entries: RoomChangeEntry[];
}

interface CreditSummary {
  semester: string;
  enrolled: string;
  limit: string;
}

interface LanguageOption {
  name: string;
  value: string;
}

interface RegisteredCourse {
  period: string;
  day: string;
  semester: string;
  course_name: string;
  course_code: string;
  instructor: string;
  campus: string;
  credits: string;
  room: string;
  status: string;
}

export interface RegistrationData {
  student: StudentInfo;
  credit_summary: CreditSummary[];
  courses: RegisteredCourse[];
  year_semester: string;
  last_applied: string;
  language_options: LanguageOption[];
}

export interface ExamEntry {
  day: string;
  period: number;
  course_name: string;
  room: string;
}

export interface ExamTimetableData {
  student: StudentInfo;
  entries: ExamEntry[];
}

export interface NotificationEntry {
  id: string;
  title: string;
  date: string;
  category: string;
}

export interface NotificationsData {
  entries: NotificationEntry[];
}

// ============ Syllabus Types ============

export interface SyllabusSearchParams {
  year_from: string;
  year_to: string;
  term: string;
  campus: string;
  department: string;
  class_code: string;
  day_period: string;
  keyword: string;
  instructor: string;
  language: string;
}

export interface SyllabusEntry {
  academic_year: string;
  department: string;
  class_code: string;
  course_title: string;
  instructor: string;
  term: string;
  day_period: string;
  campus: string;
  credits: string;
  bookmarked: boolean;
  refer_index: string;
  register_index: string;
}

export interface SyllabusSearchResult {
  entries: SyllabusEntry[];
  total_count: number;
  current_page: number;
  total_pages: number;
}

// ============ Syllabus Search Cache ============
// Persists search form state and results across tab switches

interface SyllabusSearchState {
  params: SyllabusSearchParams;
  result: SyllabusSearchResult | null;
  favorites: SyllabusSearchResult | null;
  searched: boolean;
  collapsed: boolean;
}

const defaultSyllabusParams: SyllabusSearchParams = {
  year_from: new Date().getFullYear().toString(),
  year_to: new Date().getFullYear().toString(),
  term: "",
  campus: "",
  department: "",
  class_code: "",
  day_period: "",
  keyword: "",
  instructor: "",
  language: "",
};

const SYLLABUS_STORAGE_KEY = "kgc-syllabus-state";

function loadSyllabusState(): SyllabusSearchState {
  if (typeof localStorage !== "undefined") {
    try {
      const raw = localStorage.getItem(SYLLABUS_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw);
        return {
          params: { ...defaultSyllabusParams, ...parsed.params },
          result: parsed.result ?? null,
          favorites: parsed.favorites ?? null,
          searched: parsed.searched ?? false,
          collapsed: parsed.collapsed ?? false,
        };
      }
    } catch { /* ignore corrupt data */ }
  }
  return {
    params: { ...defaultSyllabusParams },
    result: null,
    favorites: null,
    searched: false,
    collapsed: false,
  };
}

export const syllabusSearchState = writable<SyllabusSearchState>(loadSyllabusState());

// Persist on change (debounced to avoid excessive writes)
let syllabusWriteTimer: ReturnType<typeof setTimeout> | null = null;
syllabusSearchState.subscribe((state) => {
  if (typeof localStorage !== "undefined") {
    if (syllabusWriteTimer) clearTimeout(syllabusWriteTimer);
    syllabusWriteTimer = setTimeout(() => {
      try {
        localStorage.setItem(SYLLABUS_STORAGE_KEY, JSON.stringify(state));
      } catch { /* quota exceeded etc */ }
    }, 500);
  }
});

export const activeTab = writable<string>("home");
export type SettingsPanel = "ai" | "session" | "mail" | "calendar" | "notification" | "download" | "about" | "debug";
export const activeSettingsPanel = writable<SettingsPanel>("ai");
export const aiRefreshRequested = writable<boolean>(false);
export const unreadNotifCount = writable<number>(0);
export const unreadMailCount = writable<number>(0);

// ============ AI Refresh Scheduler ============
export const aiNotifStore = writable<{ result: any; sources: any[]; timestamp: number } | null>(null);
export const aiTodoStore = writable<{ result: any; timestamp: number } | null>(null);
export const aiRefreshing = writable<{ notif: boolean; todo: boolean }>({ notif: false, todo: false });

// ============ Cache Status (for titlebar indicator) ============
export interface RefreshItemStatus {
  key: string;
  label: string;
  platform: string;
  status: "pending" | "running" | "done" | "error";
}

export interface CacheStatusData {
  /** Timestamp of the last completed poll cycle (volatile or stable) */
  lastUpdated: number;
  /** Number of cache entries currently refreshing */
  refreshingCount: number;
  /** Whether a full manual refresh is in progress */
  fullRefreshing: boolean;
  /** Per-item refresh status for the current full refresh */
  items: RefreshItemStatus[];
}
export const cacheStatus = writable<CacheStatusData>({
  lastUpdated: 0,
  refreshingCount: 0,
  fullRefreshing: false,
  items: [],
});

// ============ Read State (DB is source of truth) ============
export interface ReadIdsData { kgc: string[]; luna: string[]; kwic: string[] }
export const readIdsStore = writable<ReadIdsData>({ kgc: [], luna: [], kwic: [] });

/** Canonical key for dedup: normalized title + date */
export function notifKey(title: string, date: string): string {
  return `${title.trim().replace(/\s+/g, "")}|${date}`;
}

/** Load read IDs from DB into the store. Call once on app init. */
export async function loadReadIds(): Promise<void> {
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") {
    readIdsStore.set({ kgc: [], luna: [], kwic: [] });
    return;
  }
  const data = await invoke<ReadIdsData>("get_read_notifications");
  readIdsStore.set(data);
}

/** Mark a single notification as read. DB-first, then update store. */
export async function markRead(source: string, id: string): Promise<void> {
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") !== "1") {
    await invoke<void>("mark_notification_read", { source, id });
  }
  readIdsStore.update(store => {
    const key = source as keyof ReadIdsData;
    if (store[key].includes(id)) return store;
    return { ...store, [source]: [...store[key], id] };
  });
}

/** Mark multiple notifications as read. DB-first, then update store. */
export async function markBatchRead(source: string, ids: string[]): Promise<void> {
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") !== "1") {
    await invoke<void>("mark_batch_notification_read", { source, ids });
  }
  readIdsStore.update(store => {
    const key = source as keyof ReadIdsData;
    const existing = new Set(store[key]);
    const fresh = ids.filter(id => !existing.has(id));
    if (fresh.length === 0) return store;
    return { ...store, [source]: [...store[key], ...fresh] };
  });
}

function initTheme(): "system" | "light" | "dark" {
  if (typeof localStorage !== "undefined") {
    const saved = localStorage.getItem("selah-theme");
    if (saved === "light" || saved === "dark") {
      document.documentElement.setAttribute("data-theme", saved);
      // Sync initial theme to Rust so child webviews can read it
      setAppTheme(saved).catch(() => {});
      return saved;
    }
  }
  return "system";
}
export const theme = writable<"system" | "light" | "dark">(initTheme());

// Dev mode: unlocked by 7-tap on About panel version label.
// In-memory only — resets to false every app launch.
export const devModeActive = writable<boolean>(false);

// ============ Task Registry (for debug panel task observer) ============

export interface TaskInfo {
  key: string;
  label: string;
  /** "volatile" = frequent, "stable" = infrequent, "system" = internal timers */
  tier: "volatile" | "stable" | "system";
  intervalMs: number;
  lastRunTs: number | null;
  lastOk: boolean | null;
  running: boolean;
}

const taskMap = new Map<string, TaskInfo>();
const taskListeners = new Set<() => void>();

export function registerTask(key: string, label: string, tier: TaskInfo["tier"], intervalMs: number) {
  if (!taskMap.has(key)) {
    taskMap.set(key, { key, label, tier, intervalMs, lastRunTs: null, lastOk: null, running: false });
    notifyTaskListeners();
  }
}

export function updateTask(key: string, patch: Partial<Pick<TaskInfo, "running" | "lastRunTs" | "lastOk">>) {
  const t = taskMap.get(key);
  if (!t) return;
  Object.assign(t, patch);
  notifyTaskListeners();
}

export function updateTaskInterval(key: string, intervalMs: number) {
  const t = taskMap.get(key);
  if (!t) return;
  t.intervalMs = intervalMs;
  notifyTaskListeners();
}

export function getTaskSnapshot(): TaskInfo[] {
  return [...taskMap.values()];
}

export function onTaskChange(cb: () => void): () => void {
  taskListeners.add(cb);
  return () => { taskListeners.delete(cb); };
}

function notifyTaskListeners() {
  for (const cb of taskListeners) cb();
}

// ============ Data Cache ============
// Unified caching layer: memory + disk (localStorage) + stale-while-revalidate
//
// Usage:  data = await cachedFetch("key", fetcher)
// SWR:    onCacheUpdate("key", (fresh) => { data = fresh })
//
// To add a new cached endpoint:
//   1. Add TTL to CACHE_TTLS (optional, defaults to 5 min)
//   2. Add key to DISK_CACHE_KEYS if it should persist across restarts

const cache = new Map<string, { data: any; ts: number }>();
const inflight = new Map<string, Promise<any>>();

const DEFAULT_TTL = 5 * 60 * 1000; // 5 minutes
const CACHE_TTLS: Record<string, number> = {
  // KG-Course
  schedule_data: 30 * 60 * 1000,
  grades: 30 * 60 * 1000,
  exams: 30 * 60 * 1000,
  registration: 30 * 60 * 1000,
  cancellations: 5 * 60 * 1000,
  makeup: 5 * 60 * 1000,
  rooms: 5 * 60 * 1000,
  notifications: 5 * 60 * 1000,
  profile: 60 * 60 * 1000,
  favorites: 10 * 60 * 1000,
  // Luna
  luna_todo: 5 * 60 * 1000,
  luna_updates: 5 * 60 * 1000,
  // Weather
  weather: 60 * 60 * 1000,
  // Mail
  mail_inbox: 5 * 60 * 1000,
  // KWIC
  kwic_home: 12 * 60 * 60 * 1000,
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
  "weather", "student_profile", "mail_profile",
]);
const BACKEND_CACHE_DB_KEYS: Record<string, string> = {
  exams: "exam_timetable",
};
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

function persistCacheValue<T>(key: string, data: T, ts: number, notify: boolean) {
  cache.set(key, { data, ts });
  if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, ts);
  if (notify) notifySwr(key, data);
}

async function loadBackendManagedCache<T>(key: string): Promise<{ data: T; ts: number } | null> {
  try {
    if (key === "schedule_data") {
      const data = await invoke<T>("get_schedule_snapshot");
      return { data, ts: Date.now() };
    }
    const dbKey = BACKEND_CACHE_DB_KEYS[key] ?? key;
    const json = await invoke<string | null>("get_data_cache", { key: dbKey });
    if (!json) return null;
    return { data: JSON.parse(json) as T, ts: Date.now() };
  } catch {
    return null;
  }
}

function queueBackendManagedRefresh<T>(key: string, force: boolean, fallback?: T): Promise<T> {
  const pending = inflight.get(key);
  if (pending) return pending as Promise<T>;

  const refreshPromise = invoke<string[]>("backend_refresh_now", { keys: [key], force })
    .then(async () => {
      const loaded = await loadBackendManagedCache<T>(key);
      if (!loaded) {
        if (fallback !== undefined) return fallback;
        throw new Error(`No backend cache available for "${key}"`);
      }
      persistCacheValue(key, loaded.data, loaded.ts, true);
      return loaded.data;
    })
    .catch((err) => {
      if (fallback !== undefined) return fallback;
      throw err;
    })
    .finally(() => {
      if (inflight.get(key) === refreshPromise) inflight.delete(key);
    });
  inflight.set(key, refreshPromise);
  return refreshPromise;
}

export async function cachedBackendFetch<T>(key: string, ttl?: number): Promise<T> {
  const effectiveTtl = ttl ?? CACHE_TTLS[key] ?? DEFAULT_TTL;
  const entry = cache.get(key);
  if (entry && Date.now() - entry.ts < effectiveTtl) {
    return entry.data as T;
  }

  if (entry) {
    void queueBackendManagedRefresh<T>(key, false, entry.data as T);
    return entry.data as T;
  }

  if (DISK_CACHE_KEYS.has(key)) {
    const disk = loadDiskCache(key);
    if (disk) {
      persistCacheValue(key, disk.data as T, disk.ts, false);
      void queueBackendManagedRefresh<T>(key, false, disk.data as T);
      return disk.data as T;
    }
  }

  const loaded = await loadBackendManagedCache<T>(key);
  if (loaded) {
    persistCacheValue(key, loaded.data, loaded.ts, false);
    void queueBackendManagedRefresh<T>(key, false, loaded.data);
    return loaded.data;
  }

  return queueBackendManagedRefresh<T>(key, true);
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
  // Demo mode: always serve from cache, never hit network
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") {
    const entry = cache.get(key);
    if (entry) return Promise.resolve(entry.data as T);
    const disk = loadDiskCache(key);
    if (disk) { cache.set(key, disk); return Promise.resolve(disk.data as T); }
    return fetcher().then((data) => {
      const now = Date.now();
      cache.set(key, { data, ts: now });
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
    if (data != null) return data;
    // Background refresh failed and returned undefined — fall through to fresh fetch
    return fetcher().then((freshData) => {
      const now = Date.now();
      cache.set(key, { data: freshData, ts: now });
      if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, freshData, now);
      return freshData;
    });
  });

  // Stale-while-revalidate: if disk cache exists, return stale data immediately
  if (DISK_CACHE_KEYS.has(key) && !entry) {
    const disk = loadDiskCache(key);
    if (disk) {
      cache.set(key, disk);
      // Background refresh (fire-and-forget, errors are swallowed)
      const bg = fetcher().then((data) => {
        // Guard: don't overwrite good cache with empty schedule data
        if (key === "schedule_data") {
          const sr = data as any;
          if (sr && sr.raw && Array.isArray(sr.raw.kgc_entries_current) && sr.raw.kgc_entries_current.length === 0 && !sr.raw.current_week_label) {
            console.warn(`[Selah] SWR: "${key}" returned empty data, keeping stale cache`);
            return disk.data as T;
          }
        }
        const now = Date.now();
        cache.set(key, { data, ts: now });
        saveDiskCache(key, data, now);
        notifySwr(key, data);
        return data;
      }).catch((err) => {
        console.warn(`[Selah] SWR background refresh failed for "${key}":`, err);
        // Still notify listeners with the stale data so UI stays consistent
        return disk.data as T;
      }).finally(() => inflight.delete(key));
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
      if (dbData != null) {
        const now = Date.now();
        cache.set(key, { data: dbData, ts: now });
        // Persist to localStorage so getCached() can find it synchronously next time
        if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, dbData, now);
        // Background refresh (Rust saves to DB on success automatically)
        // Replace the inflight entry with the bg promise so further callers
        // dedup against the refresh, not the already-resolved DB read.
        const bg = fetcher().then((freshData) => {
          const ts = Date.now();
          cache.set(key, { data: freshData, ts });
          if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, freshData, ts);
          notifySwr(key, freshData);
          return freshData;
        }).catch((err) => {
          console.warn(`[Selah] DB-SWR background refresh failed for "${key}":`, err);
          return dbData;
        }).finally(() => inflight.delete(key));
        inflight.set(key, bg);
        return dbData;
      }
      // No DB cache — fall through to normal fetch
      return fetcher().then((data) => {
        const ts = Date.now();
        cache.set(key, { data, ts });
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
    return dbSwr;
  }

  const p = fetcher().then((data) => {
    const now = Date.now();
    cache.set(key, { data, ts: now });
    if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, now);
    return data;
  }).finally(() => {
    inflight.delete(key);
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

export function invalidateCache(key?: string) {
  if (key) {
    cache.delete(key);
    inflight.delete(key);
    localStorage.removeItem(DISK_PREFIX + key);
  } else {
    cache.clear();
    inflight.clear();
    for (const k of DISK_CACHE_KEYS) localStorage.removeItem(DISK_PREFIX + k);
  }
}

/** Update a cached entry in-place and notify SWR listeners. */
export function updateCacheEntry<T>(key: string, updater: (data: T) => T): void {
  const entry = cache.get(key);
  if (!entry) return;
  const updated = updater(entry.data as T);
  const now = Date.now();
  cache.set(key, { data: updated, ts: now });
  if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, updated, now);
  notifySwr(key, updated);
}

export function replaceCacheEntry<T>(key: string, data: T, ts: number = Date.now()): void {
  cache.set(key, { data, ts });
  if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, ts);
  notifySwr(key, data);
}

/**
 * Force-refresh a cached key in the background. Deduped with inflight map.
 * On success, updates cache + disk + notifies SWR listeners.
 * On failure, silently swallowed (stale data retained).
 */
export function refreshCache<T>(key: string, fetcher: () => Promise<T>): Promise<T> | null {
  if (inflight.has(key)) return null; // already refreshing
  const p = fetcher().then((data) => {
    const now = Date.now();
    cache.set(key, { data, ts: now });
    if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, now);
    notifySwr(key, data);
    return data;
  }).catch((err) => {
    console.warn(`[Selah] Background refresh failed for "${key}":`, err);
    return undefined as unknown as T;
  }).finally(() => { inflight.delete(key); });
  inflight.set(key, p);
  return p;
}

// ============ Faculty Filter ============

/** Check if a department string is related to the user's faculty */
function isRelatedDept(dept: string, faculty: string): boolean {
  if (!faculty) return false;
  return dept.includes(faculty) || faculty.includes(dept);
}

/** Split entries into related (matching faculty) and others */
export function splitByFaculty<T extends { department: string }>(
  entries: T[] | undefined,
  faculty: string,
): { related: T[]; others: T[] } {
  if (!entries?.length || !faculty) return { related: [], others: entries ?? [] };
  const related = entries.filter((e) => isRelatedDept(e.department, faculty));
  const others = entries.filter((e) => !isRelatedDept(e.department, faculty));
  return { related, others };
}

// ============ AI Config Types ============

export interface AiConfig {
  ai_enabled: boolean;
  provider: "local" | "openai" | "gemini";
  local_model: string;
  api_key: string;
  model: string;
  base_url: string;
  max_tokens: number;
  temperature: number;
  reply_language: string;
  ai_refresh_interval: number; // minutes, 0 = disabled
  live_summary_interval_minutes: number; // minutes, 5..30
}

export interface AiChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

// ============ Agent (Selah) ============

export interface AgentConversationSummary {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
}

export const agentConversations = writable<AgentConversationSummary[]>([]);
export const agentActiveConvId = writable<string | null>(null);

// ============ AI Readiness (reactive) ============

/** General AI readiness: ai_enabled + provider properly configured */
export const aiReady = writable<boolean>(false);
/** Agent entry readiness: ai_enabled + selected provider is usable (local or API). */
export const agentReady = writable<boolean>(false);
