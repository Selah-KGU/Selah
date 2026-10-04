import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isAuxiliarySurface } from "./surfaceKind";
import { openExternalUrl } from "./system";
import { startTrayStatus, stopTrayStatus } from "./trayStatus";
import {
  DETAIL_GENERATED_TODO_KEY,
  LIVE_GENERATED_TODO_KEY,
  generatedTodoIdentityKey,
  mergeDetailTodosIntoLunaTodos,
  mergeGeneratedTodosIntoLunaTodos,
  mergeGeneratedTodosIntoSchedule,
  repairDetailGeneratedTodoSourceUrls,
} from "./generatedTodoSupport";
import type {
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
  AiConfig,
  AiChatMessage,
} from "./stores";
import type { ScheduleResponse, AiScheduleResult, AiTodoAnalysis, LunaTodoItem } from "./types";
import { authState, lunaAuthState, kwicAuthState, mailAuthState, gcalAuthState, invalidateCache, reloginInProgress, sessionExpired, refreshBackendManagedCache, registerTask, updateTask, updateTaskInterval, cacheStatus, aiNotifStore, aiTodoStore, aiRefreshing, aiReady, agentReady, activeTab, activeSettingsPanel, replaceCacheEntry, getCached, isCacheFresh, isEmptyNotificationsPayload, hasMemoryCache, getCacheStamp, touchCacheTimestamp, rememberRawCache, readRawCache, hasRawCache, knownRawUpdatedAt, requestedMailMessageId } from "./stores";
import type { RefreshItemStatus } from "./stores";
import { get } from "svelte/store";
import type { LiveGeneratedTodo, LiveTodoSuggestion } from "./liveSessionApi";

/** Check if demo mode is active (no async import needed — just reads localStorage). */
function _isDemo(): boolean {
  try { return localStorage.getItem("selah-demo-mode") === "1"; } catch { return false; }
}

export function isDemoActive(): boolean {
  return _isDemo();
}

function debugLog(...args: unknown[]): void {
  try {
    if (localStorage.getItem("selah-debug-logs") === "1") console.log(...args);
  } catch { /* ignore */ }
}

const DEMO_AI_CONFIG_KEY = "selah-demo-ai-config";
const DEMO_GCAL_CONFIG_KEY = "selah-demo-gcal-config";

function readDemoJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw);
    return { ...fallback, ...parsed };
  } catch {
    return fallback;
  }
}

function writeDemoJson<T>(key: string, value: T): void {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch {}
}

// Global listeners — app-lifetime. Registration is idempotent so HMR
// re-imports do not stack duplicate handlers on the Tauri event bus.
const __SELAH_LISTENERS_KEY = Symbol.for("selah.api.globalListeners");
const __selahGlobal = globalThis as unknown as Record<symbol, boolean>;
// Each WebView is its own realm, so this guard does not stop auxiliary windows
// from subscribing. Those windows import api helpers but must not fan out
// app-wide cache sync on every backend emit.
if (!isAuxiliarySurface() && !__selahGlobal[__SELAH_LISTENERS_KEY]) {
  __selahGlobal[__SELAH_LISTENERS_KEY] = true;

  listen("luna-login-success", () => {
    lunaAuthState.set({ authenticated: true });
  });

  listen("kwic-login-success", () => {
    kwicAuthState.set({ authenticated: true });
  });

  // Handle login phase 2/3 failures — undo premature auth state
  listen("luna-login-error", () => {
    lunaAuthState.set({ authenticated: false });
  });

  listen("kwic-login-error", () => {
    kwicAuthState.set({ authenticated: false });
  });

  listen<UniversityLoginComplete>("university-login-complete", (event) => {
    lunaAuthState.set({ authenticated: event.payload.luna_authenticated });
    kwicAuthState.set({ authenticated: event.payload.kwic_authenticated });
    sessionExpired.set(!(event.payload.luna_authenticated && event.payload.kwic_authenticated));
  });

  listen<{ email: string; displayName: string }>("mail-login-success", (event) => {
    mailAuthState.set({
      authenticated: true,
      email: event.payload.email,
      displayName: event.payload.displayName,
    });
  });

  listen("gcal-login-success", () => {
    gcalAuthState.update(s => ({ ...s, authenticated: true }));
  });

  listen("gcal-login-error", () => {
    // A failed or timed-out retry must not clear a session that is still valid.
    gcalCheckSession()
      .then((status) => {
        gcalAuthState.update((s) => ({
          ...s,
          authenticated: status.authenticated,
          calendarExists: status.calendar_exists,
          syncedEvents: status.synced_events,
        }));
      })
      .catch(() => {});
  });

  listen("mail-login-error", () => {
    mailAuthState.set({ authenticated: false, email: "", displayName: "" });
  });

  // Refresh AI readiness whenever AI config/model state changes from any window.
  listen("ai-config-changed", () => {
    updateAiReadiness().catch(() => {
      resetAiReady();
      aiReady.set(false);
      agentReady.set(false);
    });
    refreshBackendAiTaskStatus().catch((err) => {
      console.warn("[Selah] backend AI status refresh failed:", err);
    });
  });

  listen<{ keys?: string[] }>("backend-cache-updated", (event) => {
    const keys = event.payload?.keys ?? [];
    if (!keys.length) return;
    markBackendTasksUpdated(keys, true);
    syncBackendManagedKeys(keys).catch((err) => {
      console.warn("[Selah] backend cache sync failed:", err);
    });
  });

  listen<BackendAiRefreshStatus>("backend-ai-refresh-status", (event) => {
    applyBackendAiRefreshStatus(event.payload);
  });

  listen<NotificationActivationTarget>("notification-activated", (event) => {
    handleNotificationActivation(event.payload).catch((err) => {
      console.warn("[Selah] notification activation failed:", err);
    });
  });
}

function applyBackendSessionStatus(status: BackendSessionStatus) {
  lunaAuthState.set({ authenticated: status.luna_authenticated });
  kwicAuthState.set({ authenticated: status.kwic_authenticated });
  sessionExpired.set(status.session_expired);
  mailAuthState.set({
    authenticated: status.mail_authenticated,
    email: status.mail_email,
    displayName: status.mail_display_name,
  });

  if (status.kgc_session_present) {
    setAuthFromSession({
      username: status.username,
      display_name: status.display_name,
      student_id: status.student_id,
      faculty: status.faculty,
      department: status.department,
    });
    return;
  }
}

const __SELAH_SESSION_STATUS_KEY = Symbol.for("selah.api.backendSessionStatus");
if (!(__selahGlobal as unknown as Record<symbol, boolean>)[__SELAH_SESSION_STATUS_KEY]) {
  (__selahGlobal as unknown as Record<symbol, boolean>)[__SELAH_SESSION_STATUS_KEY] = true;
  listen<BackendSessionStatus>("backend-session-status", (event) => {
    applyBackendSessionStatus(event.payload);
    markBackendTasksUpdated(["preemptive_renewal"], !event.payload.session_expired);
  });
}

interface SessionStatus {
  valid: boolean;
  username: string;
  display_name: string;
  student_id: string;
  faculty: string;
  department: string;
}

interface BackendSessionStatus {
  kgc_session_present: boolean;
  session_expired: boolean;
  username: string;
  display_name: string;
  student_id: string;
  faculty: string;
  department: string;
  luna_authenticated: boolean;
  kwic_authenticated: boolean;
  mail_authenticated: boolean;
  mail_email: string;
  mail_display_name: string;
}

interface UniversityLoginComplete {
  luna_authenticated: boolean;
  kwic_authenticated: boolean;
}

interface BackendAiRefreshItemStatus {
  key: string;
  label: string;
  status: "done" | "skipped" | "error" | string;
  error?: string;
}

interface BackendAiRefreshStatus {
  running: boolean;
  last_run: number | null;
  last_ok: boolean | null;
  last_error?: string;
  interval_minutes: number;
  items?: BackendAiRefreshItemStatus[];
}

function applyBackendAiRefreshStatus(status: BackendAiRefreshStatus) {
  updateTaskInterval("ai_scheduler", status.interval_minutes ? status.interval_minutes * 60 * 1000 : 0);
  updateTask("ai_scheduler", {
    running: status.running,
    lastRunTs: status.last_run ? status.last_run * 1000 : null,
    lastOk: status.last_ok ?? null,
  });
  aiRefreshing.set({ notif: status.running, todo: status.running });
}

interface NotificationActivationTarget {
  source: "kgc" | "luna" | "kwic" | "mail";
  id: string;
  title: string;
  date: string;
  category: string;
  tab?: string | null;
  url?: string | null;
  courseInfo?: string | null;
  informationType?: string | null;
  personCategoryCd?: string | null;
  categoryCd?: string | null;
}

async function handleNotificationActivation(target: NotificationActivationTarget): Promise<void> {
  if (!target?.source) {
    activeTab.set("notifications");
    return;
  }

  if (target.source === "mail") {
    activeTab.set("mail");
    if (target.id) requestedMailMessageId.set(target.id);
    return;
  }

  if (target.source === "luna") {
    activeTab.set("notifications");
    if (target.url) {
      await lunaInvoke("university_open_detail_window", {
        path: target.url,
        title: target.title || "Luna",
        courseName: target.courseInfo || null,
      });
    }
    return;
  }

  if (target.source === "kwic") {
    activeTab.set("notifications");
    if (target.id) {
      await kwicOpenDetail({
        id: target.id,
        title: target.title || "KWIC",
        information_type: target.informationType || "",
        person_category_cd: target.personCategoryCd || "",
        category_cd: target.categoryCd || "",
      });
    }
    return;
  }

  activeTab.set("notifications");
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

export const serviceRegistry: Record<string, ServiceConfig> = {
  // IMPORTANT: Luna/KWIC must be checked BEFORE kgc because kgc's
  // generic markers ("ログインしてください", "セッションが期限切れです") are
  // substrings of Luna/KWIC messages. identifyExpiredService() returns the
  // FIRST match, so specific services must come first.
  luna: {
    expiredMarkers: [
      "Lunaセッションが期限切れです",
      "Lunaにログインしてください",
    ],
    onRecovered: () => lunaAuthState.set({ authenticated: true }),
    onReset: () => lunaAuthState.set({ authenticated: false }),
  },
  kwic: {
    expiredMarkers: [
      "KWICセッションが期限切れです",
      "KWICポータルにログインしてください",
    ],
    onRecovered: () => kwicAuthState.set({ authenticated: true }),
    onReset: () => kwicAuthState.set({ authenticated: false }),
  },
  // IMPORTANT: mail must be checked BEFORE kgc because kgc's generic markers
  // ("ログインしてください", "セッションが期限切れです") are substrings of mail messages.
  mail: {
    expiredMarkers: [
      "メールセッションが期限切れです",
      "メールにログインしてください",
      "token lost after refresh",
    ],
    onRecovered: () => {}, // Mail uses OAuth — no headless recovery
    onReset: () => {
      mailAuthState.set({ authenticated: false, email: "", displayName: "" });
    },
  },
  kgc: {
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
    onRecovered: () => refreshKgcAuthState().catch(() => {}),
    onReset: () => {
      // KGC is an auxiliary service. Keep the cached user identity and app
      // shell available; explicit logout is the only action that clears it.
      debugLog("[Selah] kgc.onReset: keeping cached identity");
    },
  },
};

function isSessionExpiredError(err: unknown): boolean {
  const msg = typeof err === "string" ? err : (err as any)?.message ?? String(err);
  for (const svc of Object.values(serviceRegistry)) {
    if (svc.expiredMarkers.some(m => msg.includes(m))) {
      debugLog("[Selah] Session expired detected:", msg);
      return true;
    }
  }
  return false;
}

/** Identify which service's session expired from the error message */
function identifyExpiredService(err: unknown): string | null {
  const msg = typeof err === "string" ? err : (err as any)?.message ?? String(err);
  for (const [key, svc] of Object.entries(serviceRegistry)) {
    if (svc.expiredMarkers.some(m => msg.includes(m))) return key;
  }
  return null;
}

const TRANSIENT_PATTERNS = [
  "リクエスト失敗", "connection", "timeout", "timed out",
  "network", "ECONNRESET", "ENOTFOUND", "リダイレクト失敗",
];

function isTransientError(msg: string): boolean {
  const lower = msg.toLowerCase();
  return TRANSIENT_PATTERNS.some(p => lower.includes(p.toLowerCase()));
}

const EVER_AUTH_KEY = "selah-ever-auth";
const EVER_AUTH_SOURCE_KEY = "selah-ever-auth-source";

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
  // Persist the "ever logged in" flag so the app never shows Login after a restart
  // when cached data is available. Only cleared by explicit logout().
  try {
    localStorage.setItem(EVER_AUTH_KEY, "1");
    localStorage.setItem(EVER_AUTH_SOURCE_KEY, "real");
  } catch {}
}

/** Apply the KGC identity already verified and stored by a successful sync. */
async function refreshKgcAuthState(): Promise<boolean> {
  const status = await getKgcSessionSnapshot();
  if (!status.valid) return false;
  setAuthFromSession(status);
  return true;
}

interface SessionStates {
  kgc: boolean;
  luna: boolean;
  kwic: boolean;
  [key: string]: boolean;
}

export async function getStoredSessionStates(): Promise<SessionStates> {
  return invoke<SessionStates>("get_session_states");
}

export interface SavedCookieSummary {
  service: "kgc" | "luna" | "kwic";
  saved: boolean;
  saved_at: number | null;
  active_cookie_count: number;
  session_cookie_count: number;
  earliest_expiry_at: number | null;
}

export async function getSavedCookieSummaries(): Promise<SavedCookieSummary[]> {
  if (_isDemo()) return [];
  return invoke<SavedCookieSummary[]>("get_saved_cookie_summaries");
}

/** Only one sync per key runs at once. All SAML work is also serialized through
 * `_samlSyncTail`, because the services share one upstream identity provider. */
const _syncInFlight = new Map<string, Promise<boolean>>();
let _samlSyncTail: Promise<void> = Promise.resolve();

export async function syncSession(service: string): Promise<boolean> {
  if (_isDemo()) return true;
  const existing = _syncInFlight.get(service);
  if (existing) return existing;

  // Queue every SAML operation. Never reuse an "all" result for KGC: "all"
  // intentionally means Luna + KWIC only.
  const promise = _samlSyncTail
    .catch(() => {})
    .then(() => invoke<boolean>("sync_session", { service }));
  _samlSyncTail = promise.then(() => {}, () => {});
  promise.then(
    () => { _syncInFlight.delete(service); },
    () => { _syncInFlight.delete(service); },
  );
  _syncInFlight.set(service, promise);

  return await promise;
}

/**
 * User-initiated re-login from the titlebar badge.
 * Opens a visible login window and on success clears sessionExpired + refreshes all data.
 */
export async function initiateRelogin(): Promise<UniversityLoginComplete | null> {
  if (_isDemo()) {
    sessionExpired.set(false);
    return { luna_authenticated: true, kwic_authenticated: true };
  }
  try {
    const result = await openVisibleLogin();
    // The completion event is emitted after KGC, Luna, and KWIC phases finish.
    startBackgroundPolling();
    return result;
  } catch (e: any) {
    if (e?.message !== "__login_cancelled__") {
      console.warn("[Selah] User-initiated relogin failed:", e);
    }
    return null;
  }
}

/**
 * Remove all university sessions and native SSO cookies, then open a visible
 * login window. App settings and cached user data are intentionally preserved.
 */
export async function resetUniversityLogin(): Promise<{ deleted: number; core: UniversityLoginComplete | null }> {
  if (_isDemo()) {
    sessionExpired.set(false);
    return {
      deleted: 0,
      core: { luna_authenticated: true, kwic_authenticated: true },
    };
  }

  stopBackgroundPolling();
  const deleted = await invoke<number>("reset_university_login");
  serviceRegistry.kgc.onReset();
  serviceRegistry.luna.onReset();
  serviceRegistry.kwic.onReset();
  sessionExpired.set(true);
  const core = await initiateRelogin();
  startBackgroundPolling();
  return { deleted, core };
}

function openVisibleLogin(): Promise<UniversityLoginComplete> {
  return new Promise<UniversityLoginComplete>(async (resolve, reject) => {
    reloginInProgress.set(true);

    let unlisten: (() => void) | null = null;
    let unlistenComplete: (() => void) | null = null;
    let unlistenErr: (() => void) | null = null;
    let unlistenCancel: (() => void) | null = null;
    const cleanup = () => {
      unlisten?.();
      unlistenComplete?.();
      unlistenErr?.();
      unlistenCancel?.();
      reloginInProgress.set(false);
    };

    try {
      unlisten = await listen<{ username: string; display_name: string; student_id: string; faculty: string; department: string }>(
        "login-success",
        (event) => {
          setAuthFromSession(event.payload);
          // Core-service authentication continues in the same login window.
        },
      );

      unlistenComplete = await listen<UniversityLoginComplete>("university-login-complete", (event) => {
        cleanup();
        resolve(event.payload);
      });

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
 * Service-aware: Luna/KWIC errors only trigger that service's recovery,
 * not a full re-login that opens 3 headless WebViews.
 */
async function withSessionGuard<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (err) {
    const msg = typeof err === "string" ? err : (err as any)?.message ?? String(err);

    // Transient network errors: retry once without recovery
    if (isTransientError(msg)) {
      debugLog("[Selah] Transient error, retrying once...");
      try { return await fn(); } catch (retryErr) {
        if (!isSessionExpiredError(retryErr)) throw retryErr;
        // Fall through to recovery with the retry error
        err = retryErr;
      }
    }

    const expiredService = identifyExpiredService(err);
    if (!expiredService) throw err;

    // KGC is auxiliary. Recover it independently and never escalate its
    // isolated failure into the app-wide re-authentication state.
    if (expiredService === "kgc") {
      try {
        const ok = await syncSession("kgc");
        if (ok) {
          serviceRegistry.kgc.onRecovered();
          return await fn();
        }
      } catch (recoveryErr) {
        debugLog("[Selah] KGC targeted recovery failed:", recoveryErr);
      }
      serviceRegistry.kgc.onReset();
      throw err;
    }

    // Mail expired → OAuth token revoked, no headless recovery possible
    if (expiredService === "mail") {
      debugLog("[Selah] Mail auth expired, resetting mail state");
      serviceRegistry.mail.onReset();
      throw err;
    }

    // Secondary service (Luna/KWIC) expired → try headless sync for just that service
    const svc = serviceRegistry[expiredService];
    debugLog(`[Selah] ${expiredService} session expired, trying targeted refresh...`);
    try {
      const ok = await syncSession(expiredService);
      if (ok) {
        svc.onRecovered();
        return await fn();
      }
    } catch (e) {
      console.warn(`[Selah] ${expiredService} headless refresh failed:`, e);
    }
    // Targeted refresh failed — reset only this service, don't escalate to full recovery
    svc.onReset();
    // Luna and KWIC are core services. A confirmed failure of either one
    // exposes the global manual re-authentication action.
    sessionExpired.set(true);
    throw err;
  }
}

/**
 * Restore all sessions on app startup.
 * Returns the stored KGC identity snapshot, or null when no returning-user
 * evidence exists. KGC itself is not contacted during startup restoration.
 */
export async function restoreAllSessions(): Promise<SessionStatus | null> {
  if (_isDemo()) {
    return {
      valid: true,
      username: "demo_user",
      display_name: "関学 太郎",
      student_id: "12345678",
      faculty: "理工学部",
      department: "情報科学科",
    };
  }
  const [initialStatus, states] = await Promise.all([
    getKgcSessionSnapshot(),
    getStoredSessionStates().catch(() => ({ kgc: false, luna: false, kwic: false })),
  ]);
  let status = initialStatus;
  debugLog("[Selah] restoreAllSessions: stored KGC snapshot =", JSON.stringify(status));
  debugLog("[Selah] restoreAllSessions: session states =", JSON.stringify(states));

  // Restore only missing core services. KGC is never proactively renewed.
  const secondaryTasks = [
    { key: "luna" as const, hasSession: states.luna, validate: () => lunaCheckSession(), config: serviceRegistry.luna },
    { key: "kwic" as const, hasSession: states.kwic, validate: () => kwicCheckSession(), config: serviceRegistry.kwic },
  ];

  // Validate secondary services that have disk cookies (fast, no WebView)
  const secondaryValid: Record<string, boolean> = {};
  await Promise.allSettled(secondaryTasks.map(async ({ key, hasSession, validate }) => {
    if (hasSession) {
      // A request error (including 429) is not proof that the session expired.
      // The backend returns false only for a confirmed login redirect.
      secondaryValid[key] = await validate().catch(() => true);
    }
  }));

  // Collect services that need headless sync
  const syncNeeded: string[] = [];
  const hasSavedSession = states.kgc || states.luna || states.kwic
    || !!(status.username || status.display_name || status.student_id);
  for (const { key } of secondaryTasks) {
    if (hasSavedSession && secondaryValid[key] !== true) syncNeeded.push(key);
  }

  if (syncNeeded.length > 0) {
    debugLog(`[Selah] Disk sessions expired, syncing serially: ${syncNeeded.join(", ")}`);
    // syncSession queues core-service SAML flows because they share the same IdP.
    const results = await Promise.allSettled(syncNeeded.map(svc => syncSession(svc)));
    for (let i = 0; i < syncNeeded.length; i++) {
      const svc = syncNeeded[i];
      const res = results[i];
      const ok = res.status === "fulfilled" && res.value;
      const config = serviceRegistry[svc];
      secondaryValid[svc] = ok;
      if (ok) config.onRecovered();
      else config.onReset();
    }
  } else {
    // All disk cookies were valid — mark secondary services
    for (const { key, config } of secondaryTasks) {
      if (secondaryValid[key]) config.onRecovered();
    }
  }

  if (!status.valid) {
    const coreReady = secondaryValid.luna === true && secondaryValid.kwic === true;
    if (coreReady && !(status.username || status.display_name || status.student_id || states.kgc)) {
      authState.set({
        authenticated: true,
        username: "",
        displayName: "ユーザー",
        studentId: "",
        faculty: "",
        department: "",
        loading: false,
        error: "",
      });
      try { localStorage.setItem(EVER_AUTH_KEY, "1"); } catch {}
      sessionExpired.set(false);
      debugLog("[Selah] restoreAllSessions: core services ready without KGC identity");
      return status;
    }
    debugLog("[Selah] restoreAllSessions: no stored KGC identity; coreReady =", coreReady);
    // KGC may naturally expire between uses. Keep the shell available whenever
    // core services or cached identity prove this is a returning user.
    if (status.username || status.display_name || status.student_id || states.kgc) {
      if (status.username || status.display_name) {
        setAuthFromSession(status);
      } else {
        // Edge case: disk session existed (states.kgc) but user info fields were empty.
        // Set minimal auth so the dashboard with cached data is shown.
        authState.set({
          authenticated: true,
          username: "",
          displayName: "\u30e6\u30fc\u30b6\u30fc",
          studentId: "",
          faculty: "",
          department: "",
          loading: false,
          error: "",
        });
        try { localStorage.setItem(EVER_AUTH_KEY, "1"); } catch {}
      }
      sessionExpired.set(!coreReady);
      debugLog(
        "[Selah] restoreAllSessions: showing cached Dashboard; coreReady =",
        coreReady,
      );
      return status; // non-null: App.svelte will show Dashboard
    }
    debugLog("[Selah] restoreAllSessions: no disk session, returning null -> Login page");
    return null;
  }
  setAuthFromSession(status);

  // Restore mail session (OAuth token from disk)
  try {
    const mailStatus = await mailCheckSession();
    if (mailStatus.authenticated) {
      mailAuthState.set({
        authenticated: true,
        email: mailStatus.email,
        displayName: mailStatus.display_name,
      });
    }
  } catch (e) {
    console.warn("[Selah] Mail session restore failed:", e);
  }

  return status;
}

/** Convenience wrapper for Luna invoke calls with session guard */
export async function lunaInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (_isDemo()) {
    const demo = await import("./demo");
    switch (command) {
      case "luna_fetch_todo":
        return demo.demoLunaTodo() as T;
      case "luna_fetch_updates":
        return demo.demoLunaUpdates() as T;
      case "luna_fetch_detail":
        return demo.demoLunaDetail(String(args?.path ?? "")) as T;
      case "luna_fetch_page":
        return demo.demoLunaPage(String(args?.path ?? "/")) as T;
      case "university_open_detail_window":
      case "luna_open_detail_window":
        return undefined as T;
      default:
        throw new Error(`[Demo] Unsupported Luna command: ${command}`);
    }
  }
  return withSessionGuard(() => invoke<T>(command, args));
}

/** Convenience wrapper for KWIC Portal invoke calls with session guard */
async function kwicInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return withSessionGuard(() => invoke<T>(command, args));
}

// ---------- KWIC Portal API ----------

export interface KwicPortalNotification {
  id: string;
  title: string;
  date: string;
  category: string;
  important: boolean;
  information_type: string;
  person_category_cd: string;
  category_cd: string;
}

interface KwicPortalSection {
  title: string;
  items: KwicPortalItem[];
}

interface KwicPortalItem {
  id: string;
  title: string;
  date: string;
  category: string;
  url: string;
  important: boolean;
  information_type: string;
  person_category_cd: string;
  category_cd: string;
}

export interface KwicPortalHome {
  sections: KwicPortalSection[];
  raw_html_debug?: string;
}

interface KwicNotificationDetail {
  title: string;
  date: string;
  sender: string;
  body_html: string;
  attachments: { name: string; url: string }[];
}

interface KwicSubportalLink {
  title: string;
  url: string;
  icon_url: string;
  description: string;
}

export interface KwicSubportalData {
  title: string;
  links: KwicSubportalLink[];
  notifications: KwicPortalNotification[];
}

export interface KwicCabinetItem {
  cabinet_id: string;
  list_id: string;
  name: string;
  level: number;
  updated_at: string;
  is_new: boolean;
  url: string;
}

export interface KwicCabinetReference {
  title: string;
  items: KwicCabinetItem[];
  raw_html_debug?: string;
}

export async function lunaCheckSession(): Promise<boolean> {
  if (_isDemo()) return true;
  return invoke<boolean>("luna_check_session");
}

export async function kwicCheckSession(): Promise<boolean> {
  if (_isDemo()) return true;
  return invoke<boolean>("kwic_check_session");
}

export async function kwicFetchHome(): Promise<KwicPortalHome> {
  if (_isDemo()) {
    const { demoKwicHome } = await import("./demo");
    return demoKwicHome();
  }
  return kwicInvoke<KwicPortalHome>("kwic_fetch_home");
}

// ============ Weather (Open-Meteo, no auth) ============

export interface WeatherData {
  temperature: number;
  weatherCode: number;
  humidity: number;
  windSpeed: number;
  tomorrow: { tempMax: number; tempMin: number; weatherCode: number } | null;
}

export async function fetchWeather(): Promise<WeatherData> {
  if (_isDemo()) {
    const { demoWeather } = await import("./demo");
    return demoWeather();
  }
  return invoke<WeatherData>("fetch_weather");
}

export async function kwicFetchDetail(n: KwicPortalNotification): Promise<KwicNotificationDetail> {
  if (_isDemo()) {
    const { demoKwicDetail } = await import("./demo");
    return demoKwicDetail(n);
  }
  return kwicInvoke<KwicNotificationDetail>("kwic_fetch_detail", {
    informationId: n.id,
    informationType: n.information_type,
    personCategoryCd: n.person_category_cd,
    categoryCd: n.category_cd,
  });
}

export async function kwicFetchSubportal(tagCd: string): Promise<KwicSubportalData> {
  if (_isDemo()) {
    const { demoKwicSubportal } = await import("./demo");
    return demoKwicSubportal(tagCd);
  }
  return kwicInvoke<KwicSubportalData>("kwic_fetch_subportal", { tagCd });
}

export async function kwicFetchCabinetReference(): Promise<KwicCabinetReference> {
  if (_isDemo()) {
    const { demoKwicCabinetReference } = await import("./demo");
    return demoKwicCabinetReference();
  }
  return kwicInvoke<KwicCabinetReference>("kwic_fetch_cabinet_reference");
}

export async function kwicOpenCabinetReference(title = "学生キャビネット"): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("kwic_open_cabinet_window", { title });
}

export async function kwicOpenLink(url: string, title: string): Promise<void> {
  if (_isDemo()) {
    if (/^https?:\/\//i.test(url)) {
      await openExternalUrl(url, { allowInDemo: true }).catch(() => {});
    }
    return;
  }
  return invoke<void>("kwic_open_link", { url, title });
}

export async function kwicOpenDetail(item: { id: string; title: string; information_type: string; person_category_cd: string; category_cd: string }): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("kwic_open_detail_window", {
    title: item.title,
    informationId: item.id,
    informationType: item.information_type,
    personCategoryCd: item.person_category_cd,
    categoryCd: item.category_cd,
  });
}

const LUNA_WEB_BASE = "https://luna.kwansei.ac.jp";

function normalizeLunaWebUrl(path: string): string {
  const trimmed = path.trim();
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  if (trimmed.startsWith("/")) return `${LUNA_WEB_BASE}${trimmed}`;
  return `${LUNA_WEB_BASE}/${trimmed}`;
}

export function isLunaTestTodo(item: Pick<LunaTodoItem, "content_type" | "url">): boolean {
  const type = (item.content_type || "").trim().toLowerCase();
  const url = (item.url || "").trim().toLowerCase();
  return (
    type.includes("テスト") ||
    type.includes("小テスト") ||
    type.includes("test") ||
    type.includes("quiz") ||
    type.includes("exam") ||
    url.includes("/examination") ||
    url.includes("/quiz")
  );
}

export async function openLunaTodoItem(item: LunaTodoItem): Promise<void> {
  let path = item.url || "";
  if (item.source === "detail" || path.startsWith("detail-generated://")) {
    path = item.source_path || "";
  }
  if (item.source === "live" || path.startsWith("live-generated://")) {
    const livePath = item.source_path || "";
    if (!livePath) return;
    if (_isDemo()) return;
    await invoke<void>("open_markdown_file_window", { path: livePath });
    return;
  }
  if (path.startsWith("mail://")) {
    const mailId = decodeURIComponent(path.slice("mail://".length));
    if (!mailId) return;
    activeTab.set("mail");
    requestedMailMessageId.set(mailId);
    return;
  }
  if (!path) return;
  const title = item.content_name || item.content_type || "TODO";
  if (_isDemo()) return;
  if (isLunaTestTodo(item)) {
    await invoke<void>("open_external_url", { url: normalizeLunaWebUrl(path), title });
    return;
  }

  const params: Record<string, unknown> = {
    path,
    title,
    courseName: item.course_name || null,
  };
  const urlParts = new URLSearchParams(path.split("?")[1] || "");
  const idnumber = urlParts.get("idnumber") || undefined;
  if (path.includes("/report/submission")) {
    params.mode = "report";
    params.idnumber = idnumber;
    params.infoId = urlParts.get("reportId") || undefined;
  } else if (path.includes("/forums/themetop")) {
    params.mode = "discussion";
  } else if (path.includes("/forums/thread")) {
    params.mode = "thread";
  } else if (path.includes("/surveys/take") || path.includes("/course/surveys")) {
    params.mode = "survey";
  }
  await lunaInvoke("university_open_detail_window", params);
}

// ---------- Microsoft 365 Mail API ----------

interface MailSessionStatus {
  authenticated: boolean;
  email: string;
  display_name: string;
}

export interface MailAttachment {
  id: string;
  name: string | null;
  contentType: string | null;
  size: number | null;
}

export interface MailMessage {
  id: string;
  subject: string | null;
  bodyPreview: string | null;
  body?: { contentType: string | null; content: string | null } | null;
  from: { emailAddress: { name: string | null; address: string | null } } | null;
  receivedDateTime: string | null;
  isRead: boolean | null;
  hasAttachments: boolean | null;
}

export interface MailDetail {
  id: string;
  subject: string | null;
  body: { contentType: string | null; content: string | null } | null;
  from: { emailAddress: { name: string | null; address: string | null } } | null;
  receivedDateTime: string | null;
  isRead: boolean | null;
  hasAttachments: boolean | null;
  toRecipients: { emailAddress: { name: string | null; address: string | null } }[] | null;
  ccRecipients: { emailAddress: { name: string | null; address: string | null } }[] | null;
}

interface MailProfile {
  displayName: string | null;
  mail: string | null;
  userPrincipalName: string | null;
}

export async function mailCheckSession(): Promise<MailSessionStatus> {
  if (_isDemo()) return { authenticated: true, email: "taro@kwansei.ac.jp", display_name: "\u95A2\u5B66 \u592A\u90CE" };
  return invoke<MailSessionStatus>("mail_check_session");
}

export async function mailOpenLogin(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("mail_open_login");
}

export async function mailFetchProfile(): Promise<MailProfile> {
  if (_isDemo()) return { displayName: "\u95A2\u5B66 \u592A\u90CE", mail: "taro@kwansei.ac.jp", userPrincipalName: "taro@kwansei.ac.jp" };
  return invoke<MailProfile>("mail_fetch_profile");
}

export async function mailFetchInbox(top?: number, skip?: number): Promise<MailMessage[]> {
  if (_isDemo()) {
    const { demoMailInbox } = await import("./demo");
    const all = demoMailInbox();
    const start = skip ?? 0;
    const end = start + (top ?? 20);
    return all.slice(start, end);
  }
  return withSessionGuard(() => invoke<MailMessage[]>("mail_fetch_inbox", { top: top ?? 20, skip: skip ?? 0 }));
}

export async function mailFetchMessage(messageId: string): Promise<MailDetail> {
  if (_isDemo()) {
    const { demoMailInbox } = await import("./demo");
    const msg = demoMailInbox().find(m => m.id === messageId);
    return {
      id: messageId,
      subject: msg?.subject ?? null,
      body: { contentType: "text", content: msg?.bodyPreview ?? "(\u6F14\u793A\u30C7\u30FC\u30BF)" },
      from: msg?.from ?? null,
      receivedDateTime: msg?.receivedDateTime ?? null,
      isRead: true,
      hasAttachments: msg?.hasAttachments ?? false,
      toRecipients: [{ emailAddress: { name: "\u95A2\u5B66 \u592A\u90CE", address: "taro@kwansei.ac.jp" } }],
      ccRecipients: [],
    };
  }
  return withSessionGuard(() => invoke<MailDetail>("mail_fetch_message", { messageId }));
}

export async function mailFetchAttachments(messageId: string): Promise<MailAttachment[]> {
  if (_isDemo()) {
    const { demoMailAttachments } = await import("./demo");
    return demoMailAttachments(messageId);
  }
  return withSessionGuard(() => invoke<MailAttachment[]>("mail_fetch_attachments", { messageId }));
}

export async function mailDownloadAttachment(messageId: string, attachmentId: string, fileName: string): Promise<string> {
  if (_isDemo()) return `/DemoDownloads/${fileName}`;
  return withSessionGuard(() => invoke<string>("mail_download_attachment", { messageId, attachmentId, fileName }));
}

// ============ Google Calendar ============

interface GcalStatus {
  authenticated: boolean;
  calendar_exists: boolean;
  synced_events: number;
  calendar_id: string;
}

interface GcalSyncEntry {
  day: string;
  period: number;
  course_name: string;
  room: string;
  is_cancelled: boolean;
}

export async function gcalCheckSession(): Promise<GcalStatus> {
  if (_isDemo()) return { authenticated: true, calendar_exists: true, synced_events: 24, calendar_id: "demo-selah-calendar" };
  return invoke<GcalStatus>("gcal_check_session");
}

export async function gcalSyncTimetable(entries: GcalSyncEntry[], weekLabel: string): Promise<string> {
  if (_isDemo()) return `デモモード: ${entries.length}件を ${weekLabel || "今週"} として同期した体験を表示しました`;
  return invoke<string>("gcal_sync_timetable", { entries, weekLabel });
}

export async function gcalOpenLogin(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("gcal_open_login");
}

export async function gcalDisconnect(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("gcal_disconnect");
}

export async function gcalGetConfig(): Promise<{ client_id: string; client_secret: string }> {
  if (_isDemo()) return readDemoJson(DEMO_GCAL_CONFIG_KEY, { client_id: "", client_secret: "" });
  return invoke("gcal_get_config");
}

export async function gcalSaveConfig(clientId: string, clientSecret: string): Promise<void> {
  if (_isDemo()) {
    writeDemoJson(DEMO_GCAL_CONFIG_KEY, { client_id: clientId, client_secret: clientSecret });
    return;
  }
  return invoke("gcal_save_config", { config: { client_id: clientId, client_secret: clientSecret } });
}

export async function gcalClearCalendar(): Promise<void> {
  if (_isDemo()) return;
  return invoke("gcal_clear_calendar", { deleteCalendar: false });
}

export async function getDataCache(key: string): Promise<string | null> {
  if (_isDemo()) {
    const DEMO_DB_MAP: Record<string, () => any> = {
      exam_timetable: () => import("./demo").then(m => m.demoExams()),
      syllabus_favorites: () => import("./demo").then(m => m.demoSyllabusFavorites()),
    };
    const gen = DEMO_DB_MAP[key];
    if (gen) return JSON.stringify(await gen());
    return null;
  }
  return invoke<string | null>("get_data_cache", { key });
}

export async function getDataCacheUpdatedAt(key: string): Promise<number | null> {
  if (_isDemo()) return null;
  return invoke<number | null>("get_data_cache_updated_at", { key });
}

export async function saveDataCache(key: string, json: string): Promise<void> {
  if (_isDemo()) return;
  return invoke("save_data_cache", { key, json });
}

export async function getBackendAiRefreshStatus(): Promise<BackendAiRefreshStatus> {
  if (_isDemo()) {
    return { running: false, last_run: null, last_ok: null, interval_minutes: 0, items: [] };
  }
  return invoke<BackendAiRefreshStatus>("get_backend_ai_refresh_status");
}

export async function backendAiRefreshNow(force: boolean = true, keys?: string[]): Promise<BackendAiRefreshStatus> {
  if (_isDemo()) {
    return { running: false, last_run: Math.floor(Date.now() / 1000), last_ok: true, interval_minutes: 0, items: [] };
  }
  const status = await invoke<BackendAiRefreshStatus>("backend_ai_refresh_now", { force, keys: keys ?? null });
  if (!keys?.length) applyBackendAiRefreshStatus(status);
  return status;
}

export async function refreshBackendAiTaskStatus(): Promise<void> {
  const status = await getBackendAiRefreshStatus();
  applyBackendAiRefreshStatus(status);
}

const BACKEND_CACHE_DB_KEY: Record<string, string> = {
  exams: "exam_timetable",
};

// AI 課題分析の有効期限（秒）。これを過ぎた結果は無効とみなし「再分析」が必要。
const AI_TODO_ANALYSIS_TTL_SECS = 12 * 3600;

async function loadBackendManagedCache(key: string): Promise<any | null> {
  if (_isDemo()) return null;
  if (key === "schedule_data") {
    const base = await getScheduleSnapshot();
    const generated = await getLiveGeneratedTodos();
    return mergeGeneratedTodosIntoSchedule(base, generated) ?? base;
  }
  const dbKey = BACKEND_CACHE_DB_KEY[key] ?? key;
  const json = await getDataCache(dbKey);
  if (key === "luna_todo") {
    const generated = await getLiveGeneratedTodos();
    const detail = await getDetailGeneratedTodos();
    if (!json && generated.length === 0 && detail.length === 0) return null;
    let parsed: LunaTodoItem[] = [];
    if (json) {
      try {
        parsed = JSON.parse(json);
      } catch (e) {
        console.warn(`[Selah] backend cache parse failed for "${key}" from "${dbKey}":`, e);
      }
    }
    const withLive = mergeGeneratedTodosIntoLunaTodos(Array.isArray(parsed) ? parsed : [], generated);
    return mergeDetailTodosIntoLunaTodos(withLive, detail);
  }
  if (!json) return null;
  try {
    return JSON.parse(json);
  } catch (e) {
    console.warn(`[Selah] backend cache parse failed for "${key}" from "${dbKey}":`, e);
    return null;
  }
}


interface CacheBatchRow {
  key: string;
  updated_at: number;
  unchanged: boolean;
  json?: string | null;
}

interface FrontendCacheBatch {
  rows: CacheBatchRow[];
  schedule_updated_at: number;
  live_todo_updated_at: number;
  schedule_unchanged: boolean;
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

function knownUpdatedAtFor(dbKey: string, memoryKey: string): number | null {
  if (!hasMemoryCache(memoryKey) || !hasRawCache(dbKey)) return null;
  return knownRawUpdatedAt(dbKey);
}

function liveTodosFromRow(row: CacheBatchRow | undefined): any[] {
  if (row && row.unchanged && hasRawCache(LIVE_GENERATED_TODO_KEY)) {
    return readRawCache<any[]>(LIVE_GENERATED_TODO_KEY) ?? [];
  }
  if (!row || row.json == null) return readRawCache<any[]>(LIVE_GENERATED_TODO_KEY) ?? [];
  const parsed = parseTodoArray(row.json);
  rememberRawCache(LIVE_GENERATED_TODO_KEY, row.updated_at, parsed);
  return parsed;
}

async function detailTodosFromRow(row: CacheBatchRow | undefined): Promise<any[]> {
  if (row && row.unchanged && hasRawCache(DETAIL_GENERATED_TODO_KEY)) {
    return readRawCache<any[]>(DETAIL_GENERATED_TODO_KEY) ?? [];
  }
  if (!row || row.json == null) return readRawCache<any[]>(DETAIL_GENERATED_TODO_KEY) ?? [];
  const parsed = parseTodoArray(row.json);
  const repaired = await repairDetailGeneratedTodoSourceUrls(parsed, getDataCache, saveDataCache);
  rememberRawCache(DETAIL_GENERATED_TODO_KEY, row.updated_at, repaired);
  return repaired;
}

async function syncBackendManagedKeys(keys: string[], onlyIfStale = false): Promise<void> {
  const uniqueKeys = [...new Set(keys.filter(Boolean))];
  if (!uniqueKeys.length || _isDemo()) return;
  const pending = uniqueKeys.filter((key) => !(onlyIfStale && isCacheFresh(key, 5 * 60 * 1000)));
  if (!pending.length) return;

  const includeSchedule = pending.includes("schedule_data");
  const needsLiveTodos = includeSchedule || pending.includes("luna_todo");
  const needsDetailTodos = pending.includes("luna_todo");
  const queries: Array<{ key: string; knownUpdatedAt: number | null }> = [];
  const seenQuery = new Set<string>();
  const pushQuery = (dbKey: string, knownUpdatedAt: number | null) => {
    if (seenQuery.has(dbKey)) return;
    seenQuery.add(dbKey);
    queries.push({ key: dbKey, knownUpdatedAt });
  };

  for (const key of pending) {
    if (key === "schedule_data") continue;
    pushQuery(cacheDbKey(key), knownUpdatedAtFor(cacheDbKey(key), key));
  }
  if (needsLiveTodos) {
    pushQuery(
      LIVE_GENERATED_TODO_KEY,
      hasRawCache(LIVE_GENERATED_TODO_KEY) ? knownRawUpdatedAt(LIVE_GENERATED_TODO_KEY) : null,
    );
  }
  if (needsDetailTodos) {
    pushQuery(
      DETAIL_GENERATED_TODO_KEY,
      hasRawCache(DETAIL_GENERATED_TODO_KEY) ? knownRawUpdatedAt(DETAIL_GENERATED_TODO_KEY) : null,
    );
  }

  const batch = await invoke<FrontendCacheBatch>("get_frontend_cache_batch", {
    queries,
    includeSchedule,
    knownScheduleStamp: includeSchedule ? knownScheduleStamp() : null,
  });
  const rows = batch.rows ?? [];

  if (includeSchedule) {
    const stamp = String(batch.schedule_updated_at) + ":" + String(batch.live_todo_updated_at);
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

  if (pending.includes("luna_todo")) {
    const lunaRow = rowByKey(rows, "luna_todo");
    const liveRow = rowByKey(rows, LIVE_GENERATED_TODO_KEY);
    const detailRow = rowByKey(rows, DETAIL_GENERATED_TODO_KEY);
    const liveChanged = !!liveRow && !liveRow.unchanged;
    const detailChanged = !!detailRow && !detailRow.unchanged;
    if (lunaRow?.unchanged && !liveChanged && !detailChanged && hasMemoryCache("luna_todo")) {
      touchCacheTimestamp("luna_todo");
    } else {
      const generated = liveTodosFromRow(liveRow);
      const detail = await detailTodosFromRow(detailRow);
      let base: unknown = [];
      if (lunaRow?.unchanged && hasMemoryCache("luna_todo")) {
        base = getCached("luna_todo") ?? [];
      } else if (lunaRow?.json) {
        const parsed = parseCacheJson<unknown>(lunaRow.json, "luna_todo");
        base = Array.isArray(parsed) ? parsed : [];
        if (parsed != null) rememberRawCache("luna_todo", lunaRow.updated_at, parsed);
      }
      const nothingStored = !lunaRow?.json && !lunaRow?.unchanged && generated.length === 0 && detail.length === 0;
      if (!nothingStored) {
        const merged = mergeDetailTodosIntoLunaTodos(
          mergeGeneratedTodosIntoLunaTodos(base, generated),
          detail,
        );
        const stamp = String(lunaRow?.updated_at ?? 0) + ":" + String(liveRow?.updated_at ?? 0) + ":" + String(detailRow?.updated_at ?? 0);
        replaceCacheEntry("luna_todo", merged, Date.now(), stamp);
      }
    }
  }

  for (const key of pending) {
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
    if (!row.json) continue;
    const data = parseCacheJson<any>(row.json, key);
    if (data == null) continue;
    if (
      key === "notifications"
      && isEmptyNotificationsPayload(data)
      && hasMemoryCache(key)
      && !isEmptyNotificationsPayload(getCached(key))
    ) {
      continue;
    }
    rememberRawCache(dbKey, row.updated_at, data);
    if (key === "ai_notif_analysis") {
      aiNotifStore.set({
        result: data.result ?? data,
        sources: Array.isArray(data.sources) ? data.sources : [],
        timestamp: typeof data.generated_at === "number" ? data.generated_at * 1000 : Date.now(),
      });
      replaceCacheEntry(key, data, Date.now(), row.updated_at);
      continue;
    }
    if (key === "ai_todo_analysis") {
      const ageSecs = row.updated_at ? Date.now() / 1000 - row.updated_at : Infinity;
      replaceCacheEntry(key, data, Date.now(), row.updated_at);
      if (ageSecs > AI_TODO_ANALYSIS_TTL_SECS) {
        aiTodoStore.set(null);
        continue;
      }
      const result = { ...(data as Record<string, unknown>) };
      delete result._cache_fingerprint;
      aiTodoStore.set({ result, timestamp: row.updated_at ? row.updated_at * 1000 : Date.now() });
      continue;
    }
    replaceCacheEntry(key, data, Date.now(), row.updated_at);
  }

  cacheStatus.update((s) => ({ ...s, lastUpdated: Date.now() }));
}

function refreshVisibleBackendCaches() {
  void syncBackendManagedKeys([
    "schedule_data",
    "notifications",
    "luna_updates",
    "luna_todo",
    "kwic_home",
    "mail_inbox",
    "weather",
    "student_profile",
    "exams",
    "ai_notif_analysis",
    "ai_todo_analysis",
  ], true);
}

async function syncBackendSessionStatusNow(): Promise<void> {
  if (_isDemo()) return;
  const status = await invoke<BackendSessionStatus>("backend_sync_session_status_now");
  applyBackendSessionStatus(status);
}

let lastForegroundSessionSyncAt = 0;
const FOREGROUND_SESSION_SYNC_COOLDOWN_MS = 60_000;

function syncForegroundSessionStatus() {
  const now = Date.now();
  if (now - lastForegroundSessionSyncAt < FOREGROUND_SESSION_SYNC_COOLDOWN_MS) return;
  lastForegroundSessionSyncAt = now;
  syncBackendSessionStatusNow().catch((err) => {
    console.warn("[Selah] foreground session status sync failed:", err);
  });
}

// ---------- Public API ----------

export async function openLoginWindow(): Promise<void> {
  if (_isDemo()) return;
  await invoke("open_login_window");
}

export async function enterDemoMode(): Promise<void> {
  stopBackgroundPolling();
  stopTrayStatus();
  sessionExpired.set(false);
  const { activateDemo } = await import("./demo");
  activateDemo();
  startBackgroundPolling();
  startTrayStatus();
}

export async function logout(): Promise<void> {
  // Demo mode: just clear demo state, no real invoke
  const { deactivateDemo, isDemoMode } = await import("./demo");
  if (isDemoMode()) {
    deactivateDemo();
    stopBackgroundPolling();
    stopTrayStatus();
    sessionExpired.set(false);
    for (const svc of Object.values(serviceRegistry)) svc.onReset();
    invalidateCache();
    try {
      localStorage.removeItem(EVER_AUTH_KEY);
      localStorage.removeItem(EVER_AUTH_SOURCE_KEY);
    } catch {}
    return;
  }

  stopBackgroundPolling();
  await invoke("logout");
  stopTrayStatus();
  sessionExpired.set(false);
  for (const svc of Object.values(serviceRegistry)) svc.onReset();
  invalidateCache();
  // Clear the persistent "ever logged in" flag so Login page shows
  try {
    localStorage.removeItem(EVER_AUTH_KEY);
    localStorage.removeItem(EVER_AUTH_SOURCE_KEY);
  } catch {}
}

async function getKgcSessionSnapshot(): Promise<SessionStatus> {
  return await invoke<SessionStatus>("get_kgc_session_snapshot");
}

async function checkSession(): Promise<SessionStatus> {
  return await invoke<SessionStatus>("check_session");
}

export async function validateSession(): Promise<SessionStatus> {
  if (_isDemo()) {
    return {
      valid: true,
      username: "demo_user",
      display_name: "関学 太郎",
      student_id: "12345678",
      faculty: "理工学部",
      department: "情報科学科",
    };
  }
  return await checkSession();
}

// ── AI-driven schedule (DB-backed, KGC+Luna raw + AI analysis) ──

export async function getScheduleSnapshot(): Promise<ScheduleResponse> {
  if (_isDemo()) {
    const { demoScheduleData } = await import("./demo");
    return demoScheduleData();
  }
  return invoke<ScheduleResponse>("get_schedule_snapshot");
}

export async function syncScheduleData(): Promise<ScheduleResponse> {
  if (_isDemo()) return getScheduleSnapshot();
  return withSessionGuard(() => invoke<ScheduleResponse>("sync_schedule_data"));
}

export async function enrichSchedule(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("enrich_schedule");
}

export async function refreshLunaCounts(): Promise<number> {
  if (_isDemo()) return 0;
  return invoke<number>("refresh_luna_counts");
}

export async function aiGenerateSchedule(
  currentWeekLabel: string,
  nextWeekLabel: string,
  force: boolean = false,
): Promise<AiScheduleResult> {
  if (_isDemo()) {
    await new Promise(r => setTimeout(r, 1200));
    const { demoAiScheduleResult } = await import("./demo");
    return demoAiScheduleResult();
  }
  return invoke<AiScheduleResult>("ai_generate_schedule", {
    currentWeekLabel,
    nextWeekLabel,
    force,
  });
}

// force=false はキャッシュのみ（AI を呼ばない）。直近の「再分析」結果が無ければ
// 失敗する。force=true で実際に AI 分析を実行する（AI 補助モードの「再分析」専用）。
export async function aiAnalyzeTodo(force: boolean = false): Promise<AiTodoAnalysis> {
  if (_isDemo()) {
    await new Promise(r => setTimeout(r, 1500));
    const { demoAiTodoAnalysis } = await import("./demo");
    return demoAiTodoAnalysis();
  }
  return invoke<AiTodoAnalysis>("ai_analyze_todo", { force });
}

export interface DetailTodoSuggestion {
  title: string;
  course_name: string;
  content_type: string;
  deadline: string;
  source_url: string;
  source_excerpt: string;
  note: string;
}

export async function aiExtractDetailTodos(force: boolean = false): Promise<DetailTodoSuggestion[]> {
  if (_isDemo()) return [];
  return invoke<DetailTodoSuggestion[]>("ai_extract_detail_todos", { force });
}

export async function fetchGrades(): Promise<GradesData> {
  if (_isDemo()) {
    const { demoGrades } = await import("./demo");
    return demoGrades();
  }
  return withSessionGuard(() => invoke<GradesData>("fetch_grades"));
}

export async function fetchCancellations(): Promise<CancellationsData> {
  if (_isDemo()) {
    const { demoCancellations } = await import("./demo");
    return demoCancellations();
  }
  return withSessionGuard(() => invoke<CancellationsData>("fetch_cancellations"));
}

export async function fetchMakeupClasses(): Promise<MakeupData> {
  if (_isDemo()) {
    const { demoMakeup } = await import("./demo");
    return demoMakeup();
  }
  return withSessionGuard(() => invoke<MakeupData>("fetch_makeup_classes"));
}

export async function fetchRoomChanges(): Promise<RoomChangesData> {
  if (_isDemo()) {
    const { demoRoomChanges } = await import("./demo");
    return demoRoomChanges();
  }
  return withSessionGuard(() => invoke<RoomChangesData>("fetch_room_changes"));
}

export async function fetchRegistration(): Promise<RegistrationData> {
  if (_isDemo()) {
    const { demoRegistration } = await import("./demo");
    return demoRegistration();
  }
  return withSessionGuard(() => invoke<RegistrationData>("fetch_registration"));
}

export async function fetchExamTimetable(): Promise<ExamTimetableData> {
  if (_isDemo()) {
    const { demoExams } = await import("./demo");
    return demoExams();
  }
  return withSessionGuard(() => invoke<ExamTimetableData>("fetch_exam_timetable"));
}

export async function fetchNotifications(): Promise<NotificationsData> {
  if (_isDemo()) {
    const { demoNotifications } = await import("./demo");
    return demoNotifications();
  }
  return withSessionGuard(() => invoke<NotificationsData>("fetch_notifications"));
}

export async function fetchPage(path: string): Promise<string> {
  if (_isDemo()) {
    const { demoFetchPage } = await import("./demo");
    return demoFetchPage(path);
  }
  return withSessionGuard(() => invoke<string>("fetch_page", { path }));
}

export async function fetchStudentProfile(): Promise<StudentInfo> {
  if (_isDemo()) {
    const { demoStudentProfile } = await import("./demo");
    return demoStudentProfile();
  }
  return withSessionGuard(() => invoke<StudentInfo>("fetch_student_profile"));
}

export async function searchSyllabus(params: SyllabusSearchParams): Promise<SyllabusSearchResult> {
  if (_isDemo()) {
    const { demoSearchSyllabus } = await import("./demo");
    return demoSearchSyllabus(params);
  }
  return withSessionGuard(() => invoke<SyllabusSearchResult>("search_syllabus", { params }));
}

export async function fetchSyllabusFavorites(): Promise<SyllabusSearchResult> {
  if (_isDemo()) {
    const { demoSyllabusFavorites } = await import("./demo");
    return demoSyllabusFavorites();
  }
  return withSessionGuard(() => invoke<SyllabusSearchResult>("fetch_syllabus_favorites"));
}

export async function toggleSyllabusBookmark(classCode: string): Promise<boolean> {
  if (_isDemo()) {
    const demo = await import("./demo");
    const next = demo.demoToggleSyllabusBookmark(classCode);
    const now = Date.now();
    const favorites = demo.demoSyllabusFavorites();
    try {
      localStorage.setItem("selah_cache_favorites", JSON.stringify({ v: 1, data: favorites, ts: now }));
      localStorage.setItem("selah_cache_syllabus_favorites", JSON.stringify({ v: 1, data: favorites, ts: now }));
    } catch {}
    return next;
  }
  return withSessionGuard(() => invoke<boolean>("toggle_syllabus_bookmark", { classCode }));
}

export async function openSyllabusDetail(classCode: string, courseName: string): Promise<void> {
  if (_isDemo()) return;
  return withSessionGuard(() => invoke<void>("open_syllabus_detail", { classCode, courseName }));
}

// ---------- AI API ----------

export async function getAiConfig(): Promise<AiConfig> {
  if (_isDemo()) {
    return readDemoJson(DEMO_AI_CONFIG_KEY, {
      ai_enabled: true,
      api_key: "demo",
      model: "",
      provider: "local",
      local_model: "apple-intelligence",
      base_url: "",
      max_tokens: 0,
      temperature: 0.7,
      reply_language: "ja",
      ai_refresh_interval: 0,
      live_summary_interval_minutes: 5,
    } satisfies AiConfig);
  }
  return invoke<AiConfig>("get_ai_config");
}

/**
 * Check if AI is actually usable for auto-trigger purposes.
 * For local provider: check if the selected model is downloaded.
 * For API providers: trust the user's configuration.
 */
let _aiReadyCache: boolean | null = null;
let _aiReadyPromise: Promise<boolean> | null = null;
export async function isAiReady(): Promise<boolean> {
  if (_isDemo()) {
    const cfg = await getAiConfig();
    return cfg.ai_enabled !== false;
  }
  if (_aiReadyCache !== null) return _aiReadyCache;
  if (_aiReadyPromise) return _aiReadyPromise;
  _aiReadyPromise = (async () => {
    try {
      const cfg = await getAiConfig();
      if (!cfg || cfg.ai_enabled === false) {
        _aiReadyCache = false;
        return false;
      }
      if (cfg.provider === "local") {
        const support = await invoke<{ supported: boolean }>("get_local_ai_support");
        _aiReadyCache = support.supported === true;
        return _aiReadyCache;
      }
      // API provider — needs api_key
      _aiReadyCache = !!(cfg.api_key?.trim());
      return _aiReadyCache;
    } catch {
      _aiReadyCache = false;
      return false;
    } finally {
      _aiReadyPromise = null;
    }
  })();
  return _aiReadyPromise;
}
/** Reset the cached AI readiness (e.g. after settings change). */
export function resetAiReady() { _aiReadyCache = null; }

/**
 * Recompute AI readiness and push into the reactive stores
 * (`aiReady` for general AI features, `agentReady` for agent entry).
 * Call this on app init and whenever AI settings change.
 */
export async function updateAiReadiness(): Promise<void> {
  resetAiReady();
  if (_isDemo()) {
    const cfg = await getAiConfig();
    aiReady.set(cfg.ai_enabled !== false);
    agentReady.set(false);
    return;
  }
  try {
    const cfg = await getAiConfig();
    if (!cfg || cfg.ai_enabled === false) {
      aiReady.set(false);
      agentReady.set(false);
      return;
    }
    if (cfg.provider === "local") {
      const support = await invoke<{ supported: boolean }>("get_local_ai_support");
      aiReady.set(support.supported === true);
      agentReady.set(support.supported === true);
    } else {
      const hasKey = !!(cfg.api_key?.trim());
      aiReady.set(hasKey);
      agentReady.set(hasKey);
    }
  } catch {
    aiReady.set(false);
    agentReady.set(false);
  }
}

export async function aiChat(messages: AiChatMessage[]): Promise<string> {
  if (_isDemo()) {
    await new Promise(r => setTimeout(r, 1000));
    const { demoAiNotifResult } = await import("./demo");
    return JSON.stringify(demoAiNotifResult());
  }
  return invoke<string>("ai_chat", { messages });
}

export type {
  LiveCourseInfo,
  LiveTranscriptLine,
  LiveTermExplanation,
  LiveWhiteboardNode,
  LiveWhiteboardEdge,
  LiveWhiteboard,
  LiveSummaryChunk,
  LiveSessionSnapshot,
  LiveSaveResult,
  LiveTodoSuggestionsEvent,
  LiveTodoSuggestion,
  LiveGeneratedTodo,
} from "./liveSessionApi";
export {
  liveGetSession,
  livePeekDayCache,
  liveStartSession,
  liveAppendTranscript,
  liveFlushSummary,
  liveGenerateOverallSummary,
  liveCancelSession,
  liveClearDayCache,
  liveFinishSession,
} from "./liveSessionApi";

async function readGeneratedTodos<T>(cacheKey: string): Promise<T[]> {
  if (_isDemo()) return [];
  const json = await getDataCache(cacheKey);
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

async function writeGeneratedTodos<T>(
  cacheKey: string,
  next: T[],
  refreshCaches: (next: T[]) => Promise<void>,
): Promise<void> {
  await saveDataCache(cacheKey, JSON.stringify(next));
  await refreshCaches(next);
}

async function refreshLiveGeneratedTodoCaches(next: LiveGeneratedTodo[]) {
  const cachedTodos = getCached<LunaTodoItem[]>("luna_todo");
  if (cachedTodos) {
    replaceCacheEntry("luna_todo", mergeGeneratedTodosIntoLunaTodos(cachedTodos, next));
  } else {
    // No memory cache: do a full load+merge so we don't blow away Luna/detail TODOs sitting on disk.
    const loaded = await loadBackendManagedCache("luna_todo");
    if (loaded) replaceCacheEntry("luna_todo", loaded);
  }
  try {
    const freshSchedule = await getScheduleSnapshot();
    const mergedSchedule = mergeGeneratedTodosIntoSchedule(freshSchedule, next);
    if (mergedSchedule) replaceCacheEntry("schedule_data", mergedSchedule);
  } catch {
    // Do not republish a disk timetable; it may belong to the previous semester.
  }
}

export async function getLiveGeneratedTodos(): Promise<LiveGeneratedTodo[]> {
  return readGeneratedTodos<LiveGeneratedTodo>(LIVE_GENERATED_TODO_KEY);
}

export async function saveLiveGeneratedTodos(
  suggestions: LiveTodoSuggestion[],
  sourcePath: string,
): Promise<LiveGeneratedTodo[]> {
  if (_isDemo() || suggestions.length === 0) return [];
  const existing = await getLiveGeneratedTodos();
  const seen = new Set(existing.map(generatedTodoIdentityKey));
  const createdAt = new Date().toISOString();
  const additions: LiveGeneratedTodo[] = [];
  for (const item of suggestions) {
    const title = (item.title || "").trim();
    if (!title) continue;
    const normalized: LiveGeneratedTodo = {
      id: `live-${createdAt}-${additions.length}`,
      title,
      course_name: (item.course_name || "").trim(),
      content_type: (item.content_type || "課題").trim(),
      deadline: (item.deadline || "").trim(),
      note: (item.note || "").trim(),
      source_excerpt: (item.source_excerpt || "").trim(),
      day: Number(item.day) || 0,
      period: Number(item.period) || 0,
      created_at: createdAt,
      source_path: sourcePath || "",
    };
    const key = generatedTodoIdentityKey(normalized);
    if (seen.has(key)) continue;
    seen.add(key);
    additions.push(normalized);
  }
  const next = [...existing, ...additions];
  await writeGeneratedTodos(LIVE_GENERATED_TODO_KEY, next, refreshLiveGeneratedTodoCaches);
  return additions;
}

export async function completeLiveGeneratedTodo(id: string): Promise<void> {
  if (_isDemo()) return;
  const target = id.trim();
  if (!target) return;
  const todos = await getLiveGeneratedTodos();
  const completedAt = new Date().toISOString();
  const next = todos.map((item) => (
    item.id === target
      ? { ...item, completed_at: item.completed_at || completedAt, archived_at: undefined }
      : item
  ));
  await writeGeneratedTodos(LIVE_GENERATED_TODO_KEY, next, refreshLiveGeneratedTodoCaches);
}

export async function deleteLiveGeneratedTodo(id: string): Promise<void> {
  if (_isDemo()) return;
  const target = id.trim();
  if (!target) return;
  const next = (await getLiveGeneratedTodos()).filter((item) => item.id !== target);
  await writeGeneratedTodos(LIVE_GENERATED_TODO_KEY, next, refreshLiveGeneratedTodoCaches);
}

// ── 詳細TODO (AI extracted from Luna 消息/課題/通知) ────────────────────────

export interface DetailGeneratedTodo extends DetailTodoSuggestion {
  id: string;
  created_at: string;
  completed_at?: string;
  archived_at?: string;
}

async function refreshDetailGeneratedTodoCaches(next: DetailGeneratedTodo[]) {
  const cachedTodos = getCached<LunaTodoItem[]>("luna_todo");
  if (cachedTodos) {
    replaceCacheEntry("luna_todo", mergeDetailTodosIntoLunaTodos(cachedTodos, next));
  } else {
    const loaded = await loadBackendManagedCache("luna_todo");
    if (loaded) replaceCacheEntry("luna_todo", loaded);
  }
}

export async function getDetailGeneratedTodos(): Promise<DetailGeneratedTodo[]> {
  const items = await readGeneratedTodos<DetailGeneratedTodo>(DETAIL_GENERATED_TODO_KEY);
  return repairDetailGeneratedTodoSourceUrls(items, getDataCache, saveDataCache);
}

export async function saveDetailGeneratedTodos(
  suggestions: DetailTodoSuggestion[],
): Promise<DetailGeneratedTodo[]> {
  if (_isDemo() || suggestions.length === 0) return [];
  const existing = await getDetailGeneratedTodos();
  const seen = new Set(existing.map(generatedTodoIdentityKey));
  const createdAt = new Date().toISOString();
  const additions: DetailGeneratedTodo[] = [];
  for (const item of suggestions) {
    const title = (item.title || "").trim();
    if (!title) continue;
    const normalized: DetailGeneratedTodo = {
      id: `detail-${createdAt}-${additions.length}`,
      title,
      course_name: (item.course_name || "").trim(),
      content_type: (item.content_type || "課題").trim(),
      deadline: (item.deadline || "").trim(),
      source_url: (item.source_url || "").trim(),
      source_excerpt: (item.source_excerpt || "").trim(),
      note: (item.note || "").trim(),
      created_at: createdAt,
    };
    const key = generatedTodoIdentityKey(normalized);
    if (seen.has(key)) continue;
    seen.add(key);
    additions.push(normalized);
  }
  const next = [...existing, ...additions];
  await writeGeneratedTodos(DETAIL_GENERATED_TODO_KEY, next, refreshDetailGeneratedTodoCaches);
  return additions;
}

export async function completeDetailGeneratedTodo(id: string): Promise<void> {
  if (_isDemo()) return;
  const target = id.trim();
  if (!target) return;
  const todos = await getDetailGeneratedTodos();
  const completedAt = new Date().toISOString();
  const next = todos.map((item) => (
    item.id === target
      ? { ...item, completed_at: item.completed_at || completedAt, archived_at: undefined }
      : item
  ));
  await writeGeneratedTodos(DETAIL_GENERATED_TODO_KEY, next, refreshDetailGeneratedTodoCaches);
}

export async function deleteDetailGeneratedTodo(id: string): Promise<void> {
  if (_isDemo()) return;
  const target = id.trim();
  if (!target) return;
  const next = (await getDetailGeneratedTodos()).filter((item) => item.id !== target);
  await writeGeneratedTodos(DETAIL_GENERATED_TODO_KEY, next, refreshDetailGeneratedTodoCaches);
}

export async function openSettingsWindow(panel?: string): Promise<void> {
  if (panel) activeSettingsPanel.set(panel as any);
  activeTab.set("settings");
}

export async function openFilesTab(focusCourse?: string): Promise<void> {
  if (_isDemo()) {
    activeSettingsPanel.set("download");
    activeTab.set("settings");
    return;
  }
  return invoke<void>("open_files_tab", { focusCourse: focusCourse ?? null });
}

/** Open the new-tab (home) page in the document-tabs window. */
export async function openNewTabPage(): Promise<void> {
  await invoke("document_tabs_new_tab");
}

export async function openProfileEditWindow(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("open_profile_edit_window");
}

export async function openSubtitleOverlay(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("open_subtitle_overlay");
}

export async function closeSubtitleOverlay(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("close_subtitle_overlay");
}

export async function subtitleOverlayIsOpen(): Promise<boolean> {
  if (_isDemo()) return false;
  return invoke<boolean>("subtitle_overlay_is_open");
}

export async function showMainAgentWindow(): Promise<void> {
  if (_isDemo()) return;
  return invoke<void>("show_main_agent_window");
}

// ============ Backend-Owned Refresh ============
// Routine cache refresh, notification polling, and session pre-renewal now live
// in Rust. The frontend keeps only:
//   1. cache hydration from backend-emitted updates
//   2. a foreground catch-up when the WebView becomes visible
//   3. AI status display and manual AI refresh commands

const TASK_LABELS: Record<string, string> = {
  schedule_data: "時間割同期",
  notifications: "KGC お知らせ取得",
  luna_todo: "Luna 課題一覧",
  luna_updates: "Luna 更新情報",
  kwic_home: "KWIC ホーム取得",
  weather: "天気予報取得",
  mail_inbox: "メール受信箱",
  grades: "成績データ",
  exams: "試験時間割",
  registration: "履修登録",
  cancellations: "休講情報",
  makeup: "補講情報",
  rooms: "教室変更",
  student_profile: "学生プロフィール",
  preemptive_renewal: "セッション更新チェック",
  ai_scheduler: "AI 定期更新",
};

const BACKEND_TASKS: Array<{ key: string; tier: "volatile" | "stable" | "system"; intervalMs: number }> = [
  { key: "notifications", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "luna_todo", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "luna_updates", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "mail_inbox", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "cancellations", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "makeup", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "rooms", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "weather", tier: "stable", intervalMs: 60 * 60 * 1000 },
  { key: "schedule_data", tier: "stable", intervalMs: 6 * 60 * 60 * 1000 },
  { key: "student_profile", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "grades", tier: "stable", intervalMs: 72 * 60 * 60 * 1000 },
  { key: "exams", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "registration", tier: "stable", intervalMs: 72 * 60 * 60 * 1000 },
  { key: "kwic_home", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "preemptive_renewal", tier: "system", intervalMs: 5 * 60 * 1000 },
];

function registerBackendRefreshTasks() {
  for (const task of BACKEND_TASKS) {
    registerTask(task.key, TASK_LABELS[task.key] ?? task.key, task.tier, task.intervalMs);
  }
}

async function readBackendTaskSyncedAt(key: string): Promise<number | null> {
  try {
    if (key === "schedule_data") {
      const snapshot = await getScheduleSnapshot();
      return snapshot.snapshot_updated_at > 0 ? snapshot.snapshot_updated_at * 1000 : null;
    }
    if (key === "preemptive_renewal") return null;
    const dbKey = BACKEND_CACHE_DB_KEY[key] ?? key;
    const updatedAt = await getDataCacheUpdatedAt(dbKey);
    return updatedAt && updatedAt > 0 ? updatedAt * 1000 : null;
  } catch {
    return null;
  }
}

export async function refreshBackendTaskStatuses() {
  await Promise.all(BACKEND_TASKS.map(async (task) => {
    if (task.key === "preemptive_renewal") return;
    const syncedAt = await readBackendTaskSyncedAt(task.key);
    updateTask(task.key, {
      running: false,
      lastRunTs: syncedAt,
      lastOk: syncedAt != null ? true : null,
    });
  }));
}

function markBackendTasksUpdated(keys: string[], ok: boolean) {
  const ts = Date.now();
  for (const key of [...new Set(keys.filter(Boolean))]) {
    updateTask(key, { running: false, lastRunTs: ts, lastOk: ok });
  }
}

export function startBackgroundPolling() {
  // Demo mode: no real polling
  if (typeof localStorage !== "undefined" && localStorage.getItem("selah-demo-mode") === "1") return;

  stopBackgroundPolling();
  document.addEventListener("visibilitychange", handlePollVisibility);
  // Routine cache/session/AI refresh is backend-owned now. Frontend only keeps
  // a foreground catch-up in case cache-update events were missed.
  registerBackendRefreshTasks();
  refreshBackendTaskStatuses().catch((err) => {
    console.warn("[Selah] backend task status hydration failed:", err);
  });
  registerTask("ai_scheduler", TASK_LABELS["ai_scheduler"], "stable", 0);
  refreshBackendAiTaskStatus().catch((err) => {
    console.warn("[Selah] backend AI task status hydration failed:", err);
  });
  refreshVisibleBackendCaches();
}

export function stopBackgroundPolling() {
  document.removeEventListener("visibilitychange", handlePollVisibility);
}

function handlePollVisibility() {
  if (document.visibilityState === "visible") {
    refreshVisibleBackendCaches();
    syncForegroundSessionStatus();
    refreshBackendAiTaskStatus().catch((err) => {
      console.warn("[Selah] backend AI status visibility sync failed:", err);
    });
  }
}

// ============ Backend AI Refresh ============
// Periodic non-Live AI analysis is timed and triggered by Rust. This wrapper is
// kept for existing manual callers; it does not start any frontend scheduler.
export async function runAiRefresh(force: boolean = false): Promise<void> {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;
  await backendAiRefreshNow(force);
}

/** One-click full refresh: invalidate all caches and re-fetch everything */
interface RefreshStep {
  key: string;
  label: string;
  platform: string;
  guard?: () => boolean;
}

/** Ordered refresh sequence: persistent data first, real-time data later. Serial within each platform. */
function getRefreshSequence(): RefreshStep[] {
  return [
    // -- KGC stable (persistent) --
    { key: "student_profile", label: TASK_LABELS.student_profile, platform: "KGC" },
    { key: "grades", label: TASK_LABELS.grades, platform: "KGC" },
    { key: "exams", label: TASK_LABELS.exams, platform: "KGC" },
    { key: "registration", label: TASK_LABELS.registration, platform: "KGC" },
    { key: "cancellations", label: TASK_LABELS.cancellations, platform: "KGC" },
    { key: "makeup", label: TASK_LABELS.makeup, platform: "KGC" },
    { key: "rooms", label: TASK_LABELS.rooms, platform: "KGC" },
    // -- KGC volatile (real-time) --
    { key: "notifications", label: TASK_LABELS.notifications, platform: "KGC" },
    { key: "kwic_home", label: TASK_LABELS.kwic_home, platform: "KGC", guard: () => get(kwicAuthState).authenticated },
    // -- Luna --
    { key: "luna_todo", label: TASK_LABELS.luna_todo, platform: "Luna", guard: () => get(lunaAuthState).authenticated },
    { key: "luna_updates", label: TASK_LABELS.luna_updates, platform: "Luna", guard: () => get(lunaAuthState).authenticated },
    // -- Mail --
    { key: "mail_inbox", label: TASK_LABELS.mail_inbox, platform: "Mail", guard: () => get(mailAuthState).authenticated },
    // -- Other --
    { key: "weather", label: TASK_LABELS.weather, platform: "Other" },
  ];
}

export async function refreshAllData(): Promise<void> {
  if (_isDemo() || !get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;

  const sequence = getRefreshSequence();
  // Filter out guarded items that aren't available
  const steps = sequence.filter(s => !s.guard || s.guard());
  // Build initial item status list
  const initialItems: RefreshItemStatus[] = steps.map(s => ({
    key: s.key, label: s.label, platform: s.platform, status: "pending",
  }));
  // Add schedule sync as the last item
  initialItems.push({ key: "schedule_sync", label: "時間割同期", platform: "KGC", status: "pending" });
  // Add AI refresh items (only if AI has been validated to work)
  const aiReady = await isAiReady();
  if (aiReady) {
    initialItems.push({ key: "ai_notif", label: "AI 通知分析", platform: "AI", status: "pending" });
    // AI 課題分析は一括更新では実行しない。AI 補助モードの「再分析」専用。
    initialItems.push({ key: "ai_schedule", label: "AI 時間割分析", platform: "AI", status: "pending" });
  }

  cacheStatus.update(s => ({ ...s, fullRefreshing: true, refreshingCount: initialItems.length, items: initialItems }));
  invalidateCache();

  function setItemStatus(key: string, status: RefreshItemStatus["status"]) {
    cacheStatus.update(s => ({
      ...s,
      items: s.items.map(it => it.key === key ? { ...it, status } : it),
      refreshingCount: status === "done" || status === "error" ? Math.max(0, s.refreshingCount - 1) : s.refreshingCount,
    }));
  }

  try {
    // Serial execution: one item at a time
    for (const step of steps) {
      setItemStatus(step.key, "running");
      updateTask(step.key, { running: true });
      try {
        const data = await refreshBackendManagedCache(step.key);
        updateTask(step.key, { running: false, lastRunTs: Date.now(), lastOk: data !== undefined });
        setItemStatus(step.key, "done");
      } catch {
        updateTask(step.key, { running: false, lastRunTs: Date.now(), lastOk: false });
        setItemStatus(step.key, "error");
      }
    }
    // Schedule sync
    setItemStatus("schedule_sync", "running");
    updateTask("schedule_data", { running: true });
    try {
      await refreshBackendManagedCache("schedule_data");
      updateTask("schedule_data", { running: false, lastRunTs: Date.now(), lastOk: true });
      setItemStatus("schedule_sync", "done");
    } catch {
      updateTask("schedule_data", { running: false, lastRunTs: Date.now(), lastOk: false });
      setItemStatus("schedule_sync", "error");
    }
    // AI refresh (after all data is fresh)
    if (aiReady) {
      setItemStatus("ai_notif", "running");
      try {
        const status = await backendAiRefreshNow(true);
        const itemStatus = new Map((status.items ?? []).map(item => [item.key, item.status]));
        setItemStatus("ai_notif", itemStatus.get("ai_notif") === "error" ? "error" : "done");
        setItemStatus("ai_schedule", itemStatus.get("ai_schedule") === "error" ? "error" : "done");
      } catch {
        setItemStatus("ai_notif", "error");
        setItemStatus("ai_schedule", "error");
      }
    }
    cacheStatus.update(s => ({ ...s, lastUpdated: Date.now() }));
  } finally {
    cacheStatus.update(s => ({ ...s, fullRefreshing: false, refreshingCount: 0 }));
  }
}

export type {
  AgentConversationSummary,
  AgentImagePart,
  AgentMessage,
  AgentStreamEvent,
} from "./agentApi";
export {
  agentListConversations,
  agentCreateConversation,
  agentLoadMessages,
  agentSend,
  agentCancel,
  agentDeleteConversation,
  agentRenameConversation,
} from "./agentApi";

// ============ Image Share ============

/** Save PNG image data to a file using the native save dialog. */
function uint8ToBase64(data: Uint8Array): Promise<string> {
  const copy = new Uint8Array(data.byteLength);
  copy.set(data);
  const blob = new Blob([copy.buffer]);
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.onerror = () => reject(reader.error ?? new Error("画像の変換に失敗しました"));
    reader.readAsDataURL(blob);
  });
}

export async function saveImageFile(data: Uint8Array, defaultName: string): Promise<string> {
  return invoke<string>("save_image_file", {
    dataBase64: await uint8ToBase64(data),
    defaultName,
  });
}

/** Copy PNG image data to the system clipboard using native APIs. */
export async function copyImageToClipboard(data: Uint8Array): Promise<void> {
  return invoke("copy_image_to_clipboard", {
    dataBase64: await uint8ToBase64(data),
  });
}

/** Share PNG image data via the native OS share sheet. */
export async function shareImageNative(data: Uint8Array, fileName: string): Promise<void> {
  return invoke("share_image_native", {
    dataBase64: await uint8ToBase64(data),
    fileName,
  });
}
