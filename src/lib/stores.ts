import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { initializeThemePreference, type ThemePreference } from "./themePreference";
import type { LiveTodoSuggestion } from "./api";
import {
  DETAIL_GENERATED_TODO_KEY,
  LIVE_GENERATED_TODO_KEY,
  mergeDetailTodosIntoLunaTodos,
  mergeGeneratedTodosIntoLunaTodos,
  mergeGeneratedTodosIntoSchedule,
  repairDetailGeneratedTodoSourceUrls,
} from "./generatedTodoSupport";

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

/** True while a user-visible university login flow is in progress. */
export const reloginInProgress = writable(false);

/** True when Luna or KWIC is unavailable and user action may be required. */
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
  max_pages?: number;
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

// ============ Live → TODO handoff ============
// When a LIVE session is saved, TODO/DDL judgment runs in the background. The
// suggestions land here (via the `live-todo-suggestions` event) and the TODO
// page renders them as drafts to add. `liveTodoPending` flags the in-between
// "判定中" state so the page can show progress instead of looking empty.
export const liveTodoDrafts = writable<{ suggestions: LiveTodoSuggestion[]; sourcePath: string } | null>(null);
export const liveTodoPending = writable<boolean>(false);
export type SettingsPanel = "ai" | "session" | "mail" | "calendar" | "notification" | "download" | "about" | "debug";
export const activeSettingsPanel = writable<SettingsPanel>("ai");
export const unreadNotifCount = writable<number>(0);
export const unreadMailCount = writable<number>(0);
export const requestedMailMessageId = writable<string | null>(null);

// ============ Backend AI Analysis ============
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

export const theme = writable<ThemePreference>(initializeThemePreference());

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

export type CacheStamp = string | number;

interface MemoryCacheEntry {
  data: any;
  ts: number;
  stamp?: CacheStamp;
}

const cache = new Map<string, MemoryCacheEntry>();
const rawCache = new Map<string, { updatedAt: number; parsed: unknown }>();
const inflight = new Map<string, Promise<any>>();

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

function stableCacheJson(data: unknown): string | null {
  try {
    return JSON.stringify(data);
  } catch {
    return null;
  }
}

export function isCacheFresh(key: string, ttl?: number): boolean {
  const entry = cache.get(key);
  if (!entry) return false;
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

export function rememberRawCache(key: string, updatedAt: number, parsed: unknown): void {
  rawCache.set(key, { updatedAt, parsed });
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
  cache.set(key, { data, ts, stamp: stamp !== undefined ? stamp : prev?.stamp });
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

async function loadBackendManagedCache<T>(key: string): Promise<{ data: T; ts: number } | null> {
  try {
    if (key === "schedule_data") {
      const data = await invoke<any>("get_schedule_snapshot");
      const generated = await loadLiveGeneratedTodos();
      return { data: mergeGeneratedTodosIntoSchedule(data, generated) as T, ts: Date.now() };
    }
    const dbKey = BACKEND_CACHE_DB_KEYS[key] ?? key;
    const json = await invoke<string | null>("get_data_cache", { key: dbKey });
    if (key === "luna_todo") {
      const generated = await loadLiveGeneratedTodos();
      const detail = await loadDetailGeneratedTodos();
      if (!json && generated.length === 0 && detail.length === 0) return null;
      const parsed = json ? JSON.parse(json) : [];
      const withLive = mergeGeneratedTodosIntoLunaTodos(parsed, generated);
      const withDetail = mergeDetailTodosIntoLunaTodos(withLive, detail);
      return { data: withDetail as T, ts: Date.now() };
    }
    if (!json) return null;
    const parsed = JSON.parse(json);
    return { data: parsed as T, ts: Date.now() };
  } catch {
    return null;
  }
}

async function loadLiveGeneratedTodos(): Promise<any[]> {
  try {
    const json = await invoke<string | null>("get_data_cache", { key: LIVE_GENERATED_TODO_KEY });
    if (!json) return [];
    const parsed = JSON.parse(json);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

async function readDataCache(key: string): Promise<string | null> {
  return invoke<string | null>("get_data_cache", { key });
}

async function writeDataCache(key: string, json: string): Promise<void> {
  await invoke("save_data_cache", { key, json });
}

async function loadDetailGeneratedTodos(): Promise<any[]> {
  try {
    const json = await readDataCache(DETAIL_GENERATED_TODO_KEY);
    if (!json) return [];
    const parsed = JSON.parse(json);
    return Array.isArray(parsed)
      ? await repairDetailGeneratedTodoSourceUrls(parsed, readDataCache, writeDataCache)
      : [];
  } catch {
    return [];
  }
}

async function backendRowAgeMs(key: string, data?: unknown): Promise<number | null> {
  if (key === "schedule_data") {
    const updated = (data as { snapshot_updated_at?: number } | null)?.snapshot_updated_at;
    if (typeof updated === "number" && updated > 0) return Date.now() - updated * 1000;
    return null;
  }
  const dbKey = BACKEND_CACHE_DB_KEYS[key] ?? key;
  try {
    const updatedAt = await invoke<number | null>("get_data_cache_updated_at", { key: dbKey });
    if (typeof updatedAt === "number" && updatedAt > 0) return Date.now() - updatedAt * 1000;
  } catch {
    return null;
  }
  return null;
}

async function sqliteRowNewerThan(dbKey: string, diskTs: number): Promise<boolean> {
  try {
    const updatedAt = await invoke<number | null>("get_data_cache_updated_at", { key: dbKey });
    if (typeof updatedAt !== "number" || updatedAt <= 0) return false;
    // updated_at is whole seconds. A row written after this localStorage
    // snapshot is strictly newer; the same write is not.
    return updatedAt * 1000 > diskTs;
  } catch {
    return false;
  }
}

async function sqliteCacheNewerThan(key: string, diskTs: number): Promise<boolean> {
  const dbKeys = key === "luna_todo"
    ? [BACKEND_CACHE_DB_KEYS[key] ?? key, LIVE_GENERATED_TODO_KEY, DETAIL_GENERATED_TODO_KEY]
    : [BACKEND_CACHE_DB_KEYS[key] ?? key];
  for (const dbKey of dbKeys) {
    if (await sqliteRowNewerThan(dbKey, diskTs)) return true;
  }
  return false;
}

function refreshBackendIfStale<T>(key: string, data: T, ttl: number) {
  void backendRowAgeMs(key, data).then((age) => {
    if (age != null && age < ttl) return;
    void queueBackendManagedRefresh<T>(key, false, data);
  });
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
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") {
    const entry = cache.get(key);
    if (entry) return entry.data as T;
    const disk = readAnyDiskCache<T>(key);
    if (disk) {
      persistCacheValue(key, disk.data, disk.ts, false);
      return disk.data;
    }
  }

  const effectiveTtl = ttl ?? CACHE_TTLS[key] ?? DEFAULT_TTL;
  const entry = cache.get(key);
  if (key === "schedule_data") {
    // Memory is enough while fresh. Backend writes emit backend-cache-updated,
    // which reloads the snapshot; revisiting home should not rebuild it.
    if (entry && Date.now() - entry.ts < effectiveTtl) {
      return entry.data as T;
    }
    const loaded = await loadBackendManagedCache<T>(key);
    if (!loaded) {
      if (entry) return entry.data as T;
      throw new Error("時間割スナップショットを読み込めませんでした");
    }
    persistCacheValue(key, loaded.data, Date.now(), true);
    refreshBackendIfStale(key, loaded.data, effectiveTtl);
    return loaded.data;
  }
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
      if (Date.now() - disk.ts < effectiveTtl && !(await sqliteCacheNewerThan(key, disk.ts))) {
        persistCacheValue(key, disk.data as T, disk.ts, false);
        return disk.data as T;
      }
      // localStorage can lag SQLite. A fresh backend row must replace the
      // stale disk payload even when that row is too new to need a network refresh.
      const loaded = await loadBackendManagedCache<T>(key);
      if (loaded) {
        persistCacheValue(key, loaded.data, loaded.ts, true);
        refreshBackendIfStale(key, loaded.data, effectiveTtl);
        return loaded.data;
      }
      persistCacheValue(key, disk.data as T, disk.ts, false);
      void queueBackendManagedRefresh<T>(key, false, disk.data as T);
      return disk.data as T;
    }
  }

  const loaded = await loadBackendManagedCache<T>(key);
  if (loaded) {
    persistCacheValue(key, loaded.data, loaded.ts, false);
    refreshBackendIfStale(key, loaded.data, effectiveTtl);
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
          if (isEmptySchedulePayload(sr)) {
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
    rawCache.delete(key);
    rawCache.delete(BACKEND_CACHE_DB_KEYS[key] ?? key);
    localStorage.removeItem(DISK_PREFIX + key);
  } else {
    cache.clear();
    inflight.clear();
    rawCache.clear();
    for (const k of DISK_CACHE_KEYS) localStorage.removeItem(DISK_PREFIX + k);
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
  provider: "local" | "openai" | "openrouter" | "deepseek" | "gemini";
  local_model: string;
  api_key: string;
  model: string;
  base_url: string;
  max_tokens: number;
  temperature: number;
  reply_language: string;
  ai_refresh_interval: number; // minutes, 0 = disabled
  live_summary_interval_minutes: number; // minutes, minimum 5
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
