import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  TimetableData,
  GradesData,
  CancellationsData,
  MakeupData,
  RoomChangesData,
  RegistrationData,
  ExamTimetableData,
  NotificationsData,
  StudentInfo,
  SyllabusSearchParams,
  SyllabusSearchResult,
  CourseDetail,
  AiConfig,
  AiChatMessage,
} from "./stores";
import { authState, lunaAuthState, invalidateCache, reloginInProgress, refreshCache } from "./stores";
import { get } from "svelte/store";

// Global listener: Rust backend emits this after standalone Luna SAML login or Phase 2 of full login
listen("luna-login-success", () => {
  lunaAuthState.set({ authenticated: true });
});

export interface SessionStatus {
  valid: boolean;
  username: string;
  display_name: string;
  student_id: string;
  faculty: string;
  department: string;
}

// ============ Unified Session Management ============
//
// All services share a single SSO (Okta) layer. Recovery strategy:
//   1. Try headless refresh via hidden WebView (reuses Okta cookies)
//   2. If Okta SSO itself expired, open visible login window
//
// To add a new service:
//   1. Add an entry to `serviceRegistry`
//   2. Add backend support for `sync_session` with the new key
//   3. Use `withSessionGuard(() => invoke(...))` for its API calls

interface ServiceConfig {
  /** Error substrings that indicate this service's session expired */
  expiredMarkers: string[];
  /** Called after successful session recovery */
  onRecovered: () => void;
  /** Called when recovery fails completely */
  onReset: () => void;
}

const serviceRegistry: Record<string, ServiceConfig> = {
  kwic: {
    expiredMarkers: [
      "セッションが期限切れです",
      "セッションがタイムアウト",
      "セッション切れ",
      "認証されていません",
      "ログインしてください",
      "再ログインしてください",
      "不正なアクセスです",
      "SSO redirect detected",
    ],
    onRecovered: () => refreshKwicAuthState().catch(() => {}),
    onReset: () => {
      authState.set({
        authenticated: false, username: "", displayName: "",
        studentId: "", faculty: "", department: "",
        loading: false, error: "",
      });
    },
  },
  luna: {
    expiredMarkers: [
      "Lunaセッションが期限切れです",
      "Lunaにログインしてください",
    ],
    onRecovered: () => lunaAuthState.set({ authenticated: true }),
    onReset: () => lunaAuthState.set({ authenticated: false }),
  },
};

function isSessionExpiredError(err: unknown): boolean {
  const msg = typeof err === "string" ? err : (err as any)?.message ?? String(err);
  for (const svc of Object.values(serviceRegistry)) {
    if (svc.expiredMarkers.some(m => msg.includes(m))) {
      console.log("[Selah] Session expired detected:", msg);
      return true;
    }
  }
  return false;
}

const TRANSIENT_PATTERNS = [
  "リクエスト失敗", "connection", "timeout", "timed out",
  "network", "ECONNRESET", "ENOTFOUND", "リダイレクト失敗",
];

function isTransientError(msg: string): boolean {
  const lower = msg.toLowerCase();
  return TRANSIENT_PATTERNS.some(p => lower.includes(p.toLowerCase()));
}

export function setAuthFromSession(session: { username: string; display_name?: string; student_id?: string; faculty?: string; department?: string }) {
  authState.set({
    authenticated: true,
    username: session.username,
    displayName: session.display_name || session.username,
    studentId: session.student_id || "",
    faculty: session.faculty || "",
    department: session.department || "",
    loading: false,
    error: "",
  });
}

/** Re-fetch KWIC user info and update authState store */
async function refreshKwicAuthState(): Promise<boolean> {
  const status = await checkSession();
  if (!status.valid) return false;
  setAuthFromSession(status);
  return true;
}

export interface SessionStates {
  kwic: boolean;
  luna: boolean;
  [key: string]: boolean;
}

export async function getSessionStates(): Promise<SessionStates> {
  return invoke<SessionStates>("get_session_states");
}

export async function syncSession(service: string): Promise<boolean> {
  return invoke<boolean>("sync_session", { service });
}

// --- Recovery flow (shared by all services) ---

let recoveryPromise: Promise<void> | null = null;

/**
 * Unified session recovery: headless refresh all services -> visible login.
 * Multiple concurrent callers share the same promise (only one recovery at a time).
 */
export function triggerRelogin(): Promise<void> {
  if (recoveryPromise) return recoveryPromise;

  recoveryPromise = (async () => {
    // Phase 1: Headless refresh (Okta SSO may still be alive)
    console.log("[Selah] Session expired, trying headless refresh...");
    try {
      const ok = await syncSession("all");
      if (ok) {
        console.log("[Selah] Headless refresh succeeded (all services)");
        // onRecovered for kwic will re-fetch session info via refreshKwicAuthState
        await Promise.all(
          Object.values(serviceRegistry).map(svc => {
            try { return Promise.resolve(svc.onRecovered()); } catch { return; }
          })
        );
        return;
      }
    } catch (e) {
      console.warn("[Selah] Headless refresh error:", e);
    }

    // Phase 2: Okta SSO expired, open visible login window
    console.log("[Selah] Okta expired, opening visible login...");
    await openVisibleLogin();
  })().finally(() => { recoveryPromise = null; });

  return recoveryPromise;
}

function openVisibleLogin(): Promise<void> {
  return new Promise<void>(async (resolve, reject) => {
    reloginInProgress.set(true);

    let unlisten: (() => void) | null = null;
    let unlistenErr: (() => void) | null = null;
    let unlistenCancel: (() => void) | null = null;
    const cleanup = () => {
      unlisten?.();
      unlistenErr?.();
      unlistenCancel?.();
      reloginInProgress.set(false);
    };

    try {
      unlisten = await listen<{ username: string; display_name: string; student_id: string; faculty: string; department: string }>(
        "login-success",
        (event) => {
          setAuthFromSession(event.payload);
          // Skip kwic.onRecovered (authState already set above), recover other services
          for (const [key, svc] of Object.entries(serviceRegistry)) {
            if (key !== "kwic") svc.onRecovered();
          }
          cleanup();
          resolve();
        },
      );

      unlistenErr = await listen<string>("login-error", (_event) => {
        cleanup();
        reject(new Error("再ログインに失敗しました"));
      });

      unlistenCancel = await listen<string>("login-cancelled", (_event) => {
        cleanup();
        reject(new Error("__login_cancelled__"));
      });

      await openLoginWindow();
    } catch (e) {
      cleanup();
      reject(e);
    }
  });
}

// --- API call wrappers ---

/**
 * Wrap any API call with automatic session recovery + retry.
 * Handles all registered services (KWIC, Luna, future).
 */
async function withSessionGuard<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (err) {
    const msg = typeof err === "string" ? err : (err as any)?.message ?? String(err);

    // Transient network errors: retry once without recovery
    if (isTransientError(msg)) {
      console.log("[Selah] Transient error, retrying once...");
      try { return await fn(); } catch (retryErr) {
        if (!isSessionExpiredError(retryErr)) throw retryErr;
        // Fall through to recovery
      }
    }

    if (isSessionExpiredError(err)) {
      try {
        await triggerRelogin();
        return await fn();
      } catch (recoveryErr: any) {
        console.log("[Selah] Recovery failed:", recoveryErr);
        for (const svc of Object.values(serviceRegistry)) svc.onReset();
        throw recoveryErr;
      }
    }
    throw err;
  }
}

/**
 * Restore all sessions on app startup.
 * Returns KWIC session status, or null if KWIC session is invalid.
 */
export async function restoreAllSessions(): Promise<SessionStatus | null> {
  const status = await checkSession();
  if (!status.valid) return null;

  setAuthFromSession(status);

  // Restore secondary services
  try {
    const states = await getSessionStates();
    for (const [key, config] of Object.entries(serviceRegistry)) {
      if (key === "kwic") continue; // already handled above
      if (states[key]) {
        config.onRecovered();
      } else {
        // Primary (KWIC) is valid, so Okta SSO likely alive — try headless sync
        syncSession(key)
          .then(ok => { if (ok) config.onRecovered(); })
          .catch(e => console.warn(`[Selah] Startup ${key} sync failed:`, e));
      }
    }
  } catch (e) {
    console.warn("[Selah] Secondary session restore failed:", e);
  }

  return status;
}

/** Convenience wrapper for Luna invoke calls with session guard */
export async function lunaInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return withSessionGuard(() => invoke<T>(command, args));
}

// ---------- Public API ----------

export async function openLoginWindow(): Promise<void> {
  await invoke("open_login_window");
}

export async function logout(): Promise<void> {
  stopBackgroundPolling();
  await invoke("logout");
  // Reset all services and clear cache
  for (const svc of Object.values(serviceRegistry)) svc.onReset();
  invalidateCache();
}

export async function checkSession(): Promise<SessionStatus> {
  return await invoke<SessionStatus>("check_session");
}

export async function validateSession(): Promise<SessionStatus> {
  return await invoke<SessionStatus>("validate_session");
}

export async function fetchTimetable(): Promise<TimetableData> {
  return withSessionGuard(() => invoke<TimetableData>("fetch_timetable"));
}

export async function fetchTimetableWeek(direction: "prev" | "next"): Promise<TimetableData> {
  return withSessionGuard(() => invoke<TimetableData>("fetch_timetable_week", { direction }));
}

export async function fetchCourseDetail(path: string): Promise<CourseDetail> {
  return withSessionGuard(() => invoke<CourseDetail>("fetch_course_detail", { path }));
}

export async function fetchGrades(): Promise<GradesData> {
  return withSessionGuard(() => invoke<GradesData>("fetch_grades"));
}

export async function fetchCancellations(): Promise<CancellationsData> {
  return withSessionGuard(() => invoke<CancellationsData>("fetch_cancellations"));
}

export async function fetchMakeupClasses(): Promise<MakeupData> {
  return withSessionGuard(() => invoke<MakeupData>("fetch_makeup_classes"));
}

export async function fetchRoomChanges(): Promise<RoomChangesData> {
  return withSessionGuard(() => invoke<RoomChangesData>("fetch_room_changes"));
}

export async function fetchRegistration(): Promise<RegistrationData> {
  return withSessionGuard(() => invoke<RegistrationData>("fetch_registration"));
}

export async function fetchExamTimetable(): Promise<ExamTimetableData> {
  return withSessionGuard(() => invoke<ExamTimetableData>("fetch_exam_timetable"));
}

export async function fetchNotifications(): Promise<NotificationsData> {
  return withSessionGuard(() => invoke<NotificationsData>("fetch_notifications"));
}

export async function fetchPage(path: string): Promise<string> {
  return withSessionGuard(() => invoke<string>("fetch_page", { path }));
}

export async function fetchStudentProfile(): Promise<StudentInfo> {
  return withSessionGuard(() => invoke<StudentInfo>("fetch_student_profile"));
}

export async function debugInfo(): Promise<any> {
  return await invoke("debug_info");
}

export async function debugPing(target: string): Promise<any> {
  return await invoke("debug_ping", { target });
}

export async function searchSyllabus(params: SyllabusSearchParams): Promise<SyllabusSearchResult> {
  return withSessionGuard(() => invoke<SyllabusSearchResult>("search_syllabus", { params }));
}

export async function fetchSyllabusFavorites(): Promise<SyllabusSearchResult> {
  return withSessionGuard(() => invoke<SyllabusSearchResult>("fetch_syllabus_favorites"));
}

export async function toggleSyllabusBookmark(classCode: string): Promise<boolean> {
  return withSessionGuard(() => invoke<boolean>("toggle_syllabus_bookmark", { classCode }));
}

export async function openSyllabusDetail(classCode: string, courseName: string): Promise<void> {
  return withSessionGuard(() => invoke<void>("open_syllabus_detail", { classCode, courseName }));
}

// ---------- AI API ----------

export async function getAiConfig(): Promise<AiConfig> {
  return invoke<AiConfig>("get_ai_config");
}

export async function saveAiConfig(config: AiConfig): Promise<void> {
  return invoke<void>("save_ai_config", { config });
}

export async function aiChat(messages: AiChatMessage[]): Promise<string> {
  return invoke<string>("ai_chat", { messages });
}

export async function aiTestConnection(): Promise<string> {
  return invoke<string>("ai_test_connection");
}

export async function openSettingsWindow(): Promise<void> {
  return invoke<void>("open_settings_window");
}

export async function openProfileEditWindow(): Promise<void> {
  return invoke<void>("open_profile_edit_window");
}

// ============ Background Polling ============
// Two tiers:
//   - Volatile (5 min): notifications, todo, change info
//   - Stable (12 hours): timetable, grades, exams, registration, luna_timetable

const POLL_INTERVAL = 5 * 60 * 1000; // 5 minutes
const STABLE_POLL_INTERVAL = 12 * 60 * 60 * 1000; // 12 hours
let pollTimer: ReturnType<typeof setInterval> | null = null;
let stablePollTimer: ReturnType<typeof setInterval> | null = null;
let initialPollTimeout: ReturnType<typeof setTimeout> | null = null;

interface PollTarget {
  key: string;
  fetcher: () => Promise<any>;
  /** Only poll when this returns true */
  guard?: () => boolean;
}

function getVolatileTargets(): PollTarget[] {
  return [
    { key: "notifications", fetcher: fetchNotifications },
    { key: "cancellations", fetcher: fetchCancellations },
    { key: "makeup", fetcher: fetchMakeupClasses },
    { key: "rooms", fetcher: fetchRoomChanges },
    { key: "luna_todo", fetcher: () => lunaInvoke<any>("luna_fetch_todo"), guard: () => get(lunaAuthState).authenticated },
    { key: "luna_updates", fetcher: () => lunaInvoke<any>("luna_fetch_updates"), guard: () => get(lunaAuthState).authenticated },
  ];
}

function getStableTargets(): PollTarget[] {
  return [
    { key: "timetable", fetcher: fetchTimetable },
    { key: "grades", fetcher: fetchGrades },
    { key: "exams", fetcher: fetchExamTimetable },
    { key: "registration", fetcher: fetchRegistration },
    { key: "luna_timetable", fetcher: () => lunaInvoke<any>("luna_fetch_timetable", {}), guard: () => get(lunaAuthState).authenticated },
  ];
}

function doPoll() {
  if (!get(authState).authenticated || get(reloginInProgress)) return;
  for (const t of getVolatileTargets()) {
    if (t.guard && !t.guard()) continue;
    refreshCache(t.key, t.fetcher);
  }
}

function doStablePoll() {
  if (!get(authState).authenticated || get(reloginInProgress)) return;
  for (const t of getStableTargets()) {
    if (t.guard && !t.guard()) continue;
    refreshCache(t.key, t.fetcher);
  }
}

export function startBackgroundPolling() {
  stopBackgroundPolling();
  // Initial volatile poll after a short delay (let views mount first)
  initialPollTimeout = setTimeout(doPoll, 10_000);
  pollTimer = setInterval(() => {
    if (document.visibilityState === "visible") doPoll();
  }, POLL_INTERVAL);
  // Stable data: refresh every 12 hours
  stablePollTimer = setInterval(() => {
    doStablePoll();
  }, STABLE_POLL_INTERVAL);
  // Also poll when window becomes visible after being hidden
  document.addEventListener("visibilitychange", handlePollVisibility);
}

export function stopBackgroundPolling() {
  if (initialPollTimeout) { clearTimeout(initialPollTimeout); initialPollTimeout = null; }
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
  if (stablePollTimer) { clearInterval(stablePollTimer); stablePollTimer = null; }
  document.removeEventListener("visibilitychange", handlePollVisibility);
}

function handlePollVisibility() {
  if (document.visibilityState === "visible") doPoll();
}
