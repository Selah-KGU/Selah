import { writable } from "svelte/store";

export interface AuthState {
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

/** Luna LMS authentication state */
export const lunaAuthState = writable<{ authenticated: boolean }>({
  authenticated: false,
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

export interface TimetableEntry {
  day: string;
  period: number;
  course_name: string;
  room: string;
  course_code: string;
  is_cancelled: boolean;
  is_makeup: boolean;
  is_room_changed: boolean;
  detail_path: string;
}

export interface TimetableData {
  student: StudentInfo;
  entries: TimetableEntry[];
  week_label: string;
  struts_token: string;
  form_fields: Record<string, string>;
}

export interface CurriculumRow {
  category: string;
  required_credits: string;
  enrolled_credits: string;
  earned_credits: string;
}

export interface GradesData {
  student: StudentInfo;
  curriculum: CurriculumRow[];
}

export interface CancellationEntry {
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

export interface MakeupEntry {
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

export interface RoomChangeEntry {
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

export interface CreditSummary {
  semester: string;
  enrolled: string;
  limit: string;
}

export interface RegisteredCourse {
  period: string;
  day: string;
  semester: string;
  course_name: string;
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

export interface CourseDetail {
  fields: [string, string][];
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

export interface SyllabusSearchState {
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

const SYLLABUS_STORAGE_KEY = "kwic-syllabus-state";

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
export const aiRefreshRequested = writable<boolean>(false);
function initTheme(): "system" | "light" | "dark" {
  if (typeof localStorage !== "undefined") {
    const saved = localStorage.getItem("selah-theme");
    if (saved === "light" || saved === "dark") {
      document.documentElement.setAttribute("data-theme", saved);
      return saved;
    }
  }
  return "system";
}
export const theme = writable<"system" | "light" | "dark">(initTheme());
export const debugVisible = writable<boolean>(false);

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
  // KG (KWIC)
  timetable: 30 * 60 * 1000,
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
  luna_timetable: 30 * 60 * 1000,
  luna_todo: 5 * 60 * 1000,
  luna_updates: 5 * 60 * 1000,
};

// Keys eligible for disk persistence (survive app restart, stale-while-revalidate)
const DISK_CACHE_KEYS = new Set([
  "timetable", "grades", "exams", "registration",
  "luna_timetable",
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
  return () => { swrListeners.get(key)?.delete(cb as (data: any) => void); };
}

function notifySwr(key: string, data: any) {
  swrListeners.get(key)?.forEach((cb) => { try { cb(data); } catch { /* ignore */ } });
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
  const effectiveTtl = ttl ?? CACHE_TTLS[key] ?? DEFAULT_TTL;
  const entry = cache.get(key);
  if (entry && Date.now() - entry.ts < effectiveTtl) {
    return Promise.resolve(entry.data as T);
  }
  // Dedup: if the same key is already being fetched, share the promise
  const pending = inflight.get(key);
  if (pending) return pending as Promise<T>;

  // Stale-while-revalidate: if disk cache exists, return stale data immediately
  if (DISK_CACHE_KEYS.has(key) && !entry) {
    const disk = loadDiskCache(key);
    if (disk) {
      cache.set(key, disk);
      // Background refresh (fire-and-forget, errors are swallowed)
      const bg = fetcher().then((data) => {
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

/**
 * Force-refresh a cached key in the background. Deduped with inflight map.
 * On success, updates cache + disk + notifies SWR listeners.
 * On failure, silently swallowed (stale data retained).
 */
export function refreshCache<T>(key: string, fetcher: () => Promise<T>): void {
  if (inflight.has(key)) return; // already refreshing
  const p = fetcher().then((data) => {
    const now = Date.now();
    cache.set(key, { data, ts: now });
    if (DISK_CACHE_KEYS.has(key)) saveDiskCache(key, data, now);
    notifySwr(key, data);
  }).catch((err) => {
    console.warn(`[Selah] Background refresh failed for "${key}":`, err);
  }).finally(() => { inflight.delete(key); });
  inflight.set(key, p);
}

// ============ Faculty Filter ============

/** Check if a department string is related to the user's faculty */
export function isRelatedDept(dept: string, faculty: string): boolean {
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
  provider: "openai" | "gemini" | "custom";
  api_key: string;
  model: string;
  base_url: string;
  max_tokens: number;
  temperature: number;
  reply_language: string;
}

export interface AiChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}
