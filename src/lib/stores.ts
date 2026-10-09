import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { initializeThemePreference, type ThemePreference } from "./themePreference";
import type { LiveTodoSuggestion } from "./api";

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

let readStateTail: Promise<void> = Promise.resolve();
let pendingReadIds: Promise<void> | null = null;
let readStateVersion = 0;

function readStateDemo(): boolean {
  return typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1";
}

function queueReadState(work: () => Promise<void>): Promise<void> {
  const request = readStateTail.then(work);
  readStateTail = request.catch(() => {});
  return request;
}

function mutateReadIds(write: () => Promise<void>, apply: () => void): Promise<void> {
  pendingReadIds = null;
  const version = readStateVersion;
  const demo = readStateDemo();
  const current = () => version === readStateVersion && (demo || !readStateDemo());
  const work = async () => {
    if (!current()) return;
    try {
      if (typeof localStorage !== "undefined" && !demo) await write();
    } catch (error) {
      if (current()) throw error;
      return;
    }
    if (current()) apply();
  };
  return demo ? work() : queueReadState(work);
}

/** Canonical key for dedup: normalized title + date */
export function notifKey(title: string, date: string): string {
  return `${title.trim().replace(/\s+/g, "")}|${date}`;
}

/** Load read IDs from DB into the store. Call once on app init. */
export function loadReadIds(): Promise<void> {
  if (readStateDemo()) {
    readStateVersion += 1;
    pendingReadIds = null;
    readIdsStore.set({ kgc: [], luna: [], kwic: [] });
    return Promise.resolve();
  }
  if (pendingReadIds) return pendingReadIds;
  const version = readStateVersion;
  const request = queueReadState(async () => {
    if (version !== readStateVersion || readStateDemo()) return;
    try {
      const data = await invoke<ReadIdsData>("get_read_notifications");
      if (version === readStateVersion && !readStateDemo()) readIdsStore.set(data);
    } catch (error) {
      if (version === readStateVersion && !readStateDemo()) throw error;
    }
  }).finally(() => {
    if (pendingReadIds === request) pendingReadIds = null;
  });
  pendingReadIds = request;
  return request;
}

/** Mark a single notification as read. DB-first, then update store. */
export function markRead(source: string, id: string): Promise<void> {
  return mutateReadIds(() => invoke<void>("mark_notification_read", { source, id }), () => {
    readIdsStore.update(store => {
      const key = source as keyof ReadIdsData;
      if (store[key].includes(id)) return store;
      return { ...store, [source]: [...store[key], id] };
    });
  });
}

/** Mark multiple notifications as read. DB-first, then update store. */
export function markBatchRead(source: string, ids: string[]): Promise<void> {
  const accepted = ids.slice();
  return mutateReadIds(() => invoke<void>("mark_batch_notification_read", { source, ids: accepted }), () => {
    readIdsStore.update(store => {
      const key = source as keyof ReadIdsData;
      const existing = new Set(store[key]);
      const fresh: string[] = [];
      for (const id of accepted) {
        if (!existing.has(id)) {
          existing.add(id);
          fresh.push(id);
        }
      }
      if (fresh.length === 0) return store;
      return { ...store, [source]: [...store[key], ...fresh] };
    });
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

// Cache state is independent of authentication, theme and UI stores.
export * from "./cacheStore";

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
