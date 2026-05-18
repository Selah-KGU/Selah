import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openExternalUrl } from "./system";
import { startTrayStatus, stopTrayStatus } from "./trayStatus";
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
import { authState, lunaAuthState, kwicAuthState, mailAuthState, gcalAuthState, invalidateCache, reloginInProgress, sessionExpired, refreshCache, refreshBackendManagedCache, registerTask, updateTask, updateTaskInterval, cacheStatus, aiNotifStore, aiTodoStore, aiRefreshing, aiReady, agentReady, activeTab, activeSettingsPanel, replaceCacheEntry, getCached, requestedMailMessageId } from "./stores";
import type { RefreshItemStatus } from "./stores";
import { get } from "svelte/store";

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
if (!__selahGlobal[__SELAH_LISTENERS_KEY]) {
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
    gcalAuthState.update(s => ({ ...s, authenticated: false }));
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
  });

  listen<{ keys?: string[] }>("backend-cache-updated", (event) => {
    const keys = event.payload?.keys ?? [];
    if (!keys.length) return;
    markBackendTasksUpdated(keys, true);
    syncBackendManagedKeys(keys).catch((err) => {
      console.warn("[Selah] backend cache sync failed:", err);
    });
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
  mailAuthState.set({
    authenticated: status.mail_authenticated,
    email: status.mail_email,
    displayName: status.mail_display_name,
  });

  if (status.kgc_valid) {
    setAuthFromSession({
      username: status.username,
      display_name: status.display_name,
      student_id: status.student_id,
      faculty: status.faculty,
      department: status.department,
    });
    if (get(sessionExpired)) sessionExpired.set(false);
    return;
  }

  if (status.session_expired) {
    sessionExpired.set(true);
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
  kgc_valid: boolean;
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
      // Don't wipe authState entirely — if sessionExpired is set, the user
      // should see the Dashboard with cached data, not the Login page.
      // Only wipe auth on explicit logout (which calls logout() directly).
      if (get(sessionExpired)) {
        debugLog("[Selah] kgc.onReset: sessionExpired=true, keeping authState for cached view");
        return;
      }
      authState.set({
        authenticated: false, username: "", displayName: "",
        studentId: "", faculty: "", department: "",
        loading: false, error: "",
      });
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

/** Re-fetch KG-Course user info and update authState store */
async function refreshKgcAuthState(): Promise<boolean> {
  const status = await checkSession();
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

async function getSessionStates(): Promise<SessionStates> {
  return invoke<SessionStates>("get_session_states");
}

/** Dedup map: only one syncSession per service at a time.
 * Prevents concurrent headless SAML windows from closing each other.
 * "all" waits for per-service syncs to finish; per-service waits for "all". */
const _syncInFlight = new Map<string, Promise<boolean>>();

export async function syncSession(service: string): Promise<boolean> {
  if (_isDemo()) return true;
  const existing = _syncInFlight.get(service);
  if (existing) return existing;

  // "all" internally refreshes all three — wait for per-service syncs
  if (service === "all") {
    const waits = [_syncInFlight.get("luna"), _syncInFlight.get("kwic"), _syncInFlight.get("kgc")].filter(Boolean);
    if (waits.length) await Promise.allSettled(waits);
  } else {
    // Per-service sync — wait for "all" if it's in flight (it will refresh this service too)
    const allSync = _syncInFlight.get("all");
    if (allSync) {
      return allSync;
    }
  }

  const promise = invoke<boolean>("sync_session", { service })
    .finally(() => _syncInFlight.delete(service));
  _syncInFlight.set(service, promise);

  const ok = await promise;

  // Cross-renewal: if a single service sync succeeded, Okta is alive.
  // Opportunistically refresh other expired services in the background.
  if (ok && service !== "all") {
    crossRenewOtherServices(service);
  }

  return ok;
}

const SAML_SERVICES = ["kgc", "luna", "kwic"] as const;

/**
 * When one SAML service's headless refresh succeeds, Okta SSO is proven alive.
 * Fire-and-forget refresh for other services that are currently dead.
 */
function crossRenewOtherServices(succeededService: string) {
  for (const svc of SAML_SERVICES) {
    if (svc === succeededService) continue;
    // Skip if already in-flight
    if (_syncInFlight.has(svc) || _syncInFlight.has("all")) continue;

    const isAlive = svc === "kgc"
      ? get(authState).authenticated
      : svc === "luna"
        ? get(lunaAuthState).authenticated
        : get(kwicAuthState).authenticated;

    if (!isAlive) {
      debugLog(`[Selah] Cross-renewal: ${succeededService} alive -> trying ${svc}`);
      syncSession(svc).then(ok => {
        if (ok) {
          serviceRegistry[svc].onRecovered();
          debugLog(`[Selah] Cross-renewal: ${svc} recovered`);
        }
      }).catch(() => {});
    }
  }
}

// --- Recovery flow (shared by all services) ---

let recoveryPromise: Promise<void> | null = null;
let lastRecoveryTime = 0;
const RECOVERY_COOLDOWN = 5_000; // 5 seconds cooldown after recovery completes

/**
 * Unified session recovery: headless refresh all services -> visible login.
 * Multiple concurrent callers share the same promise (only one recovery at a time).
 */
export function triggerRelogin(): Promise<void> {
  if (recoveryPromise) return recoveryPromise;
  // Skip if recovery just completed recently
  if (Date.now() - lastRecoveryTime < RECOVERY_COOLDOWN) return Promise.resolve();

  recoveryPromise = (async () => {
    // Phase 1: Headless refresh (Okta SSO may still be alive)
    debugLog("[Selah] Session expired, trying headless refresh...");
    try {
      const ok = await syncSession("all");
      if (ok) {
        debugLog("[Selah] Headless refresh: at least one service recovered");
        // Verify each service individually
        const [kgcOk, lunaOk, kwicOk] = await Promise.all([
          checkSession().then(s => s.valid).catch(() => false),
          lunaCheckSession().catch(() => false),
          kwicCheckSession().catch(() => false),
        ]);
        if (kgcOk) { serviceRegistry.kgc.onRecovered(); sessionExpired.set(false); }
        if (lunaOk) serviceRegistry.luna.onRecovered();
        else serviceRegistry.luna.onReset();
        if (kwicOk) serviceRegistry.kwic.onRecovered();
        else serviceRegistry.kwic.onReset();
        // If KGC recovered, we're good — app is usable
        if (kgcOk) return;
        // KGC failed but others may be alive — still need user to re-login for KGC
        debugLog("[Selah] KGC failed but secondary services may be alive");
      }
    } catch (e) {
      console.warn("[Selah] Headless refresh error:", e);
    }

    // Phase 2: Okta SSO expired — mark session as expired and let user initiate re-login
    debugLog("[Selah] Okta expired, marking session as expired (user can re-verify from titlebar)");
    sessionExpired.set(true);
  })().finally(() => { recoveryPromise = null; lastRecoveryTime = Date.now(); });

  return recoveryPromise;
}

/**
 * User-initiated re-login from the titlebar badge.
 * Opens a visible login window and on success clears sessionExpired + refreshes all data.
 */
export async function initiateRelogin(): Promise<void> {
  if (_isDemo()) {
    sessionExpired.set(false);
    return;
  }
  try {
    await openVisibleLogin();
    sessionExpired.set(false);
    // Refresh all data after successful re-login
    startBackgroundPolling();
  } catch (e: any) {
    if (e?.message !== "__login_cancelled__") {
      console.warn("[Selah] User-initiated relogin failed:", e);
    }
  }
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
          // Don't mark secondary services here — backend Phase 2/3 runs
          // asynchronously after this event. Global listeners for
          // luna-login-success / kwic-login-success will set their states.
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

    // KGC session expired → full recovery (headless all → visible login)
    if (expiredService === "kgc") {
      try {
        await triggerRelogin();
      } catch (recoveryErr: any) {
        debugLog("[Selah] Recovery failed:", recoveryErr);
        throw recoveryErr;
      }
      if (get(sessionExpired)) throw err;
      return await fn();
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
    // (Luna and KWIC share Okta SSO — triggerRelogin would kill the other service too)
    svc.onReset();
    throw err;
  }
}

/**
 * Restore all sessions on app startup.
 * Returns KGC session status, or null if KGC session is invalid.
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
    checkSession(),
    getSessionStates().catch(() => ({ kgc: false, luna: false, kwic: false })),
  ]);
  let status = initialStatus;
  debugLog("[Selah] restoreAllSessions: initial check_session =", JSON.stringify(status));
  debugLog("[Selah] restoreAllSessions: session states =", JSON.stringify(states));

  // If any service has expired disk cookies, refresh all in parallel.
  // This avoids sequential 20s timeouts when Okta is expired.
  const needsKgcSync = !status.valid;
  const secondaryTasks = [
    { key: "luna" as const, hasSession: states.luna, validate: () => lunaCheckSession(), config: serviceRegistry.luna },
    { key: "kwic" as const, hasSession: states.kwic, validate: () => kwicCheckSession(), config: serviceRegistry.kwic },
  ];

  // Validate secondary services that have disk cookies (fast, no WebView)
  const secondaryValid: Record<string, boolean> = {};
  await Promise.allSettled(secondaryTasks.map(async ({ key, hasSession, validate }) => {
    if (hasSession) {
      secondaryValid[key] = await validate().catch(() => false);
    }
  }));

  // Collect services that need headless sync
  const syncNeeded: string[] = [];
  if (needsKgcSync) syncNeeded.push("kgc");
  for (const { key, hasSession } of secondaryTasks) {
    if (hasSession && !secondaryValid[key]) syncNeeded.push(key);
  }

  if (syncNeeded.length > 0) {
    debugLog(`[Selah] Disk sessions expired, syncing in parallel: ${syncNeeded.join(", ")}`);
    // Run all headless syncs in parallel — shares Okta SSO, so if one fails they all will
    const results = await Promise.allSettled(syncNeeded.map(svc => syncSession(svc)));
    for (let i = 0; i < syncNeeded.length; i++) {
      const svc = syncNeeded[i];
      const res = results[i];
      const ok = res.status === "fulfilled" && res.value;
      if (svc === "kgc") {
        if (ok) {
          const fresh = await checkSession().catch(() => null);
          if (fresh?.valid) status = fresh;
          debugLog("[Selah] Headless KGC refresh succeeded");
        }
      } else {
        const config = serviceRegistry[svc];
        if (ok) config.onRecovered();
        else config.onReset();
      }
    }
  } else {
    // All disk cookies were valid — mark secondary services
    for (const { key, config } of secondaryTasks) {
      if (secondaryValid[key]) config.onRecovered();
    }
  }

  // If KGC was not valid initially, cross-renewal from Luna/KWIC may have saved it.
  if (!status.valid) {
    debugLog("[Selah] Re-checking KGC after parallel sync...");
    const recheck = await checkSession().catch(() => null);
    if (recheck?.valid) {
      status = recheck;
      debugLog("[Selah] KGC recovered via cross-renewal");
    }
    // Keep original status (with disk user info) if re-check also failed
  }

  if (!status.valid) {
    debugLog("[Selah] restoreAllSessions: KGC invalid after all recovery attempts. status =", JSON.stringify(status), "states.kgc =", states.kgc);
    // KGC session is dead, but if we have disk-saved user info, show cached
    // data with a re-auth prompt instead of dumping user to the login page.
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
      sessionExpired.set(true);
      debugLog("[Selah] restoreAllSessions: showing cached Dashboard with re-auth badge");
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

const BACKEND_CACHE_DB_KEY: Record<string, string> = {
  exams: "exam_timetable",
};

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

async function syncBackendManagedKeys(keys: string[]): Promise<void> {
  const uniqueKeys = [...new Set(keys.filter(Boolean))];
  if (!uniqueKeys.length || _isDemo()) return;

  await Promise.all(uniqueKeys.map(async (key) => {
    const data = await loadBackendManagedCache(key);
    if (data == null) return;
    replaceCacheEntry(key, data);
  }));

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
    "mail_profile",
    "exams",
  ]);
}

async function syncBackendSessionStatusNow(): Promise<void> {
  if (_isDemo()) return;
  const status = await invoke<BackendSessionStatus>("backend_sync_session_status_now");
  applyBackendSessionStatus(status);
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
  // Clear sessionExpired FIRST so kgc.onReset actually wipes authState
  sessionExpired.set(false);
  for (const svc of Object.values(serviceRegistry)) svc.onReset();
  invalidateCache();
  // Clear the persistent "ever logged in" flag so Login page shows
  try {
    localStorage.removeItem(EVER_AUTH_KEY);
    localStorage.removeItem(EVER_AUTH_SOURCE_KEY);
  } catch {}
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
  return await invoke<SessionStatus>("validate_session");
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
      local_model: "qwen3.5-8b",
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
        // Check if the selected model is downloaded
        const models = await invoke<any[]>("list_local_models");
        const selected = models.find((m: any) => m.id === cfg.local_model);
        _aiReadyCache = selected?.downloaded === true;
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
      const models = await invoke<any[]>("list_local_models");
      const selected = models.find((m: any) => m.id === cfg.local_model);
      const downloaded = selected?.downloaded === true;
      aiReady.set(downloaded);
      agentReady.set(downloaded);
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

/** Returns true when local provider is using the standard 2B model. */
export async function isLocalStandard2b(): Promise<boolean> {
  if (_isDemo()) {
    const cfg = await getAiConfig();
    return cfg.provider === "local" && cfg.local_model === "qwen3.5-2b";
  }
  try {
    const cfg = await getAiConfig();
    return cfg.provider === "local" && cfg.local_model === "qwen3.5-2b";
  } catch {
    return false;
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

export interface LiveCourseInfo {
  course_name: string;
  course_code: string;
  room: string;
  teacher: string;
  day: number;
  period: number;
  time_label: string;
  is_free_note: boolean;
}

export interface LiveTranscriptLine {
  text: string;
  at: string;
}

export interface LiveTermExplanation {
  term: string;
  explanation: string;
  source_excerpt?: string;
  external_source?: string;
}

export interface LiveSummaryChunk {
  title: string;
  range_label: string;
  body: string;
  line_count: number;
  terms?: LiveTermExplanation[];
}

export interface LiveSessionSnapshot {
  active: boolean;
  course: LiveCourseInfo | null;
  started_at: string | null;
  transcript_lines: LiveTranscriptLine[];
  pending_lines: LiveTranscriptLine[];
  summaries: LiveSummaryChunk[];
}

export interface LiveSaveResult {
  saved: boolean;
  path: string;
  markdown: string;
  snapshot: LiveSessionSnapshot;
  suggested_todos?: LiveTodoSuggestion[];
}

export interface LiveTodoSuggestion {
  title: string;
  course_name: string;
  content_type: string;
  deadline: string;
  note: string;
  source_excerpt: string;
  day: number;
  period: number;
}

export interface LiveGeneratedTodo extends LiveTodoSuggestion {
  id: string;
  created_at: string;
  source_path: string;
  completed_at?: string;
  archived_at?: string;
}

const DEMO_LIVE_KEY = "selah-demo-live-session";
const LIVE_GENERATED_TODO_KEY = "live_generated_todo";

function emptyDemoLiveSession(): LiveSessionSnapshot {
  return {
    active: false,
    course: null,
    started_at: null,
    transcript_lines: [],
    pending_lines: [],
    summaries: [],
  };
}

function loadDemoLiveSession(): LiveSessionSnapshot {
  if (!_isDemo()) return emptyDemoLiveSession();
  try {
    const raw = localStorage.getItem(DEMO_LIVE_KEY);
    if (!raw) return emptyDemoLiveSession();
    const parsed = JSON.parse(raw) as Partial<LiveSessionSnapshot>;
    return {
      active: parsed.active === true,
      course: parsed.course ?? null,
      started_at: parsed.started_at ?? null,
      transcript_lines: Array.isArray(parsed.transcript_lines) ? parsed.transcript_lines : [],
      pending_lines: Array.isArray(parsed.pending_lines) ? parsed.pending_lines : [],
      summaries: Array.isArray(parsed.summaries) ? parsed.summaries : [],
    };
  } catch {
    return emptyDemoLiveSession();
  }
}

function saveDemoLiveSession(snapshot: LiveSessionSnapshot): LiveSessionSnapshot {
  if (_isDemo()) {
    try { localStorage.setItem(DEMO_LIVE_KEY, JSON.stringify(snapshot)); } catch {}
  }
  return snapshot;
}

function demoLiveCourseMatches(a: LiveCourseInfo | null, b: LiveCourseInfo | null): boolean {
  if (!a || !b) return false;
  return a.course_name === b.course_name && a.day === b.day && a.period === b.period;
}

function buildDemoLiveSummaries(lines: LiveTranscriptLine[]): LiveSummaryChunk[] {
  if (lines.length === 0) return [];
  const recent = lines.slice(-3).map((line) => line.text).join(" / ");
  return [{
    title: "デモ用要約",
    range_label: "最近",
    body: `### 全体要約\n${recent || "このセッションでは授業内容の要点がまとめられます。"}\n\n### 次に見るポイント\n- キーワードを 2〜3 個に絞って見返す\n- 宿題や小テストに関係する箇所を先に確認する`,
    line_count: lines.length,
    terms: [
      {
        term: "メタ認知",
        explanation: "自分の理解度や学習方法を客観的に確認する考え方。復習時は、何が分かっていて何が曖昧かを分けて見る観点になる。",
        source_excerpt: "キーワードを短くメモし、あとで見返しやすい形に整理",
        external_source: "Flavell, J. H. (1979), Metacognition and cognitive monitoring, American Psychologist",
      },
      {
        term: "想起練習",
        explanation: "資料を眺めるだけでなく、覚えている内容を自分で思い出す復習方法。小テスト対策では、要点を閉じた状態で説明できるかを確認する。",
        source_excerpt: "課題や小テストにつながるポイント",
        external_source: "Roediger, H. L. & Karpicke, J. D. (2006), Test-enhanced learning, Psychological Science",
      },
    ],
  }];
}

function buildDemoLiveTranscript(course: LiveCourseInfo): LiveTranscriptLine[] {
  const now = new Date();
  const at = (offsetMin: number) =>
    new Date(now.getTime() + offsetMin * 60_000).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
    });
  const name = course.course_name || "自由ノート";
  return [
    { at: at(0), text: `${name} のデモセッションを開始しました。今日のテーマと到達目標を確認します。` },
    { at: at(2), text: "授業で強調されたキーワードを短くメモし、あとで見返しやすい形に整理します。" },
    { at: at(4), text: "課題や小テストにつながるポイントを先に押さえておくと復習が楽になります。" },
  ];
}

export async function liveGetSession(): Promise<LiveSessionSnapshot> {
  if (_isDemo()) return loadDemoLiveSession();
  return invoke<LiveSessionSnapshot>("live_get_session");
}

export async function livePeekDayCache(course: LiveCourseInfo): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    return demoLiveCourseMatches(snapshot.course, course) ? snapshot : emptyDemoLiveSession();
  }
  return invoke<LiveSessionSnapshot>("live_peek_day_cache", { course });
}

export async function liveStartSession(course: LiveCourseInfo): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const transcript_lines = buildDemoLiveTranscript(course);
    return saveDemoLiveSession({
      active: true,
      course,
      started_at: new Date().toISOString(),
      transcript_lines,
      pending_lines: [],
      summaries: buildDemoLiveSummaries(transcript_lines),
    });
  }
  return invoke<LiveSessionSnapshot>("live_start_session", { course });
}

export async function liveAppendTranscript(text: string): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    if (!snapshot.active || !text.trim()) return snapshot;
    const next: LiveSessionSnapshot = {
      ...snapshot,
      transcript_lines: [
        ...snapshot.transcript_lines,
        {
          text: text.trim(),
          at: new Date().toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" }),
        },
      ],
      pending_lines: [],
    };
    return saveDemoLiveSession(next);
  }
  return invoke<LiveSessionSnapshot>("live_append_transcript", { text });
}

export async function liveFlushSummary(force: boolean = false): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    if (!force && snapshot.transcript_lines.length === 0) return snapshot;
    const next = {
      ...snapshot,
      summaries: buildDemoLiveSummaries(snapshot.transcript_lines),
    };
    return saveDemoLiveSession(next);
  }
  return invoke<LiveSessionSnapshot>("live_flush_summary", { force });
}

export async function liveCancelSession(): Promise<void> {
  if (_isDemo()) {
    saveDemoLiveSession(emptyDemoLiveSession());
    return;
  }
  return invoke<void>("live_cancel_session");
}

export async function liveClearDayCache(course: LiveCourseInfo): Promise<void> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    if (demoLiveCourseMatches(snapshot.course, course)) {
      saveDemoLiveSession(emptyDemoLiveSession());
    }
    return;
  }
  return invoke<void>("live_clear_day_cache", { course });
}

export async function liveFinishSession(): Promise<LiveSaveResult> {
  if (_isDemo()) {
    const snapshot = await liveFlushSummary(true);
    const saved = snapshot.transcript_lines.length > 0;
    const markdown = saved
      ? `# ${snapshot.course?.course_name ?? "LIVE Demo"}\n\n${snapshot.summaries.map((chunk) => chunk.body).join("\n\n")}\n\n## Transcript\n${snapshot.transcript_lines.map((line) => `- ${line.at} ${line.text}`).join("\n")}`
      : "";
    const result: LiveSaveResult = {
      saved,
      path: saved ? `/DemoNotes/${(snapshot.course?.course_name ?? "live-demo").replace(/[^\w\u3040-\u30ff\u4e00-\u9faf-]+/g, "_")}.md` : "",
      markdown,
      snapshot: emptyDemoLiveSession(),
    };
    saveDemoLiveSession(emptyDemoLiveSession());
    return result;
  }
  return invoke<LiveSaveResult>("live_finish_session");
}

type GeneratedTodoIdentity = {
  course_name?: string;
  title?: string;
  deadline?: string;
  completed_at?: string;
  archived_at?: string;
};

function normalizeTodoIdentityKey(item: GeneratedTodoIdentity): string {
  return [item.course_name, item.title, item.deadline]
    .map((part) => (part || "").trim().toLowerCase().replace(/\s+/g, " "))
    .join("|");
}

function normalizeGeneratedTodoKey(item: GeneratedTodoIdentity): string {
  return normalizeTodoIdentityKey(item);
}

function isActiveGeneratedTodo(item: LiveGeneratedTodo): boolean {
  return isActiveGeneratedTodoItem(item);
}

function isActiveGeneratedTodoItem(item: GeneratedTodoIdentity): boolean {
  return !item.completed_at && !item.archived_at;
}

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

function isGeneratedLunaTodoItem(item: LunaTodoItem): boolean {
  return (
    item.source === "live" ||
    item.url?.startsWith("live-generated://") ||
    item.feedback?.startsWith("Liveから追加")
  );
}

function liveGeneratedTodoToLunaItem(item: LiveGeneratedTodo): LunaTodoItem {
  return {
    course_name: item.course_name,
    content_type: item.content_type || "課題",
    content_name: item.title,
    url: `live-generated://${encodeURIComponent(item.id)}`,
    deadline: item.deadline || "",
    status: "未提出",
    feedback: item.note ? `Liveから追加: ${item.note}` : "Liveから追加",
    source: "live",
    local_id: item.id,
    source_path: item.source_path,
    source_excerpt: item.source_excerpt,
  };
}

function mergeGeneratedTodosIntoLunaTodos(base: LunaTodoItem[], generated: LiveGeneratedTodo[]): LunaTodoItem[] {
  const baseWithoutGenerated = base.filter((item) => !isGeneratedLunaTodoItem(item));
  const seen = new Set(baseWithoutGenerated.map((item) => normalizeGeneratedTodoKey({
    course_name: item.course_name,
    title: item.content_name,
    deadline: item.deadline,
  })));
  const merged = [...baseWithoutGenerated];
  for (const item of generated.filter(isActiveGeneratedTodo)) {
    const key = normalizeGeneratedTodoKey({
      course_name: item.course_name,
      title: item.title,
      deadline: item.deadline,
    });
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(liveGeneratedTodoToLunaItem(item));
  }
  return merged;
}

function assignmentLabelFromGeneratedTodo(item: LiveGeneratedTodo): string {
  const type = item.content_type || "課題";
  const deadline = item.deadline ? ` (締切: ${item.deadline})` : "";
  return `Live追加 ${type}: ${item.title}${deadline}`;
}

function mergeGeneratedTodosIntoSchedule(base: ScheduleResponse | null, generated: LiveGeneratedTodo[]): ScheduleResponse | null {
  if (!base?.ai_result) return base;
  const cloned: ScheduleResponse = JSON.parse(JSON.stringify(base));
  const activeGenerated = generated.filter(isActiveGeneratedTodo);
  const mergeWeek = (items: any[]) => {
    for (const cell of items) {
      if (Array.isArray(cell.assignments)) {
        cell.assignments = cell.assignments.filter((label: unknown) => !String(label).startsWith("Live追加 "));
      }
      for (const todo of activeGenerated) {
        const matchesCourse = todo.course_name && cell.course_name === todo.course_name;
        const matchesSlot = todo.day > 0 && todo.period > 0 && cell.day === todo.day && cell.period === todo.period;
        if (!matchesCourse && !matchesSlot) continue;
        const label = assignmentLabelFromGeneratedTodo(todo);
        if (!Array.isArray(cell.assignments)) cell.assignments = [];
        if (!cell.assignments.includes(label)) cell.assignments.push(label);
      }
    }
  };
  const aiResult = cloned.ai_result;
  if (!aiResult) return cloned;
  mergeWeek(aiResult.current_week);
  mergeWeek(aiResult.next_week);
  return cloned;
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
  const cachedSchedule = getCached<ScheduleResponse>("schedule_data");
  if (cachedSchedule) {
    const mergedSchedule = mergeGeneratedTodosIntoSchedule(cachedSchedule, next);
    if (mergedSchedule) replaceCacheEntry("schedule_data", mergedSchedule);
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
  const seen = new Set(existing.map(normalizeGeneratedTodoKey));
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
    const key = normalizeGeneratedTodoKey(normalized);
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

const DETAIL_GENERATED_TODO_KEY = "detail_generated_todo";

export interface DetailGeneratedTodo extends DetailTodoSuggestion {
  id: string;
  created_at: string;
  completed_at?: string;
  archived_at?: string;
}

function normalizeDetailTodoKey(item: GeneratedTodoIdentity): string {
  return normalizeTodoIdentityKey(item);
}

function isActiveDetailTodo(item: DetailGeneratedTodo): boolean {
  return isActiveGeneratedTodoItem(item);
}

function isDetailLunaTodoItem(item: LunaTodoItem): boolean {
  return item.source === "detail" || item.url?.startsWith("detail-generated://");
}

function detailGeneratedTodoToLunaItem(item: DetailGeneratedTodo): LunaTodoItem {
  return {
    course_name: item.course_name,
    content_type: item.content_type || "課題",
    content_name: item.title,
    url: `detail-generated://${encodeURIComponent(item.id)}`,
    deadline: item.deadline || "",
    status: "未提出",
    feedback: item.note ? `マグネット: ${item.note}` : "マグネットで追加",
    source: "detail",
    local_id: item.id,
    source_path: item.source_url,
    source_excerpt: item.source_excerpt,
  };
}

function mergeDetailTodosIntoLunaTodos(base: LunaTodoItem[], generated: DetailGeneratedTodo[]): LunaTodoItem[] {
  const baseWithoutDetail = base.filter((item) => !isDetailLunaTodoItem(item));
  const seen = new Set(baseWithoutDetail.map((item) => normalizeDetailTodoKey({
    course_name: item.course_name,
    title: item.content_name,
    deadline: item.deadline,
  })));
  const merged = [...baseWithoutDetail];
  for (const item of generated.filter(isActiveDetailTodo)) {
    const key = normalizeDetailTodoKey({
      course_name: item.course_name,
      title: item.title,
      deadline: item.deadline,
    });
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(detailGeneratedTodoToLunaItem(item));
  }
  return merged;
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
  return readGeneratedTodos<DetailGeneratedTodo>(DETAIL_GENERATED_TODO_KEY);
}

export async function saveDetailGeneratedTodos(
  suggestions: DetailTodoSuggestion[],
): Promise<DetailGeneratedTodo[]> {
  if (_isDemo() || suggestions.length === 0) return [];
  const existing = await getDetailGeneratedTodos();
  const seen = new Set(existing.map(normalizeDetailTodoKey));
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
    const key = normalizeDetailTodoKey(normalized);
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

export async function openDownloadsWindow(focusCourse?: string): Promise<void> {
  if (_isDemo()) {
    activeSettingsPanel.set("download");
    activeTab.set("settings");
    return;
  }
  return invoke<void>("open_downloads_window", { focusCourse: focusCourse ?? null });
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
//   3. AI-only scheduling that still depends on frontend stores/views

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
  mail_profile: "メールプロフィール",
  preemptive_renewal: "セッション更新チェック",
  ai_scheduler: "AI 定期更新",
};

const BACKEND_TASKS: Array<{ key: string; tier: "volatile" | "stable" | "system"; intervalMs: number }> = [
  { key: "notifications", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "luna_todo", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "luna_updates", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "mail_inbox", tier: "volatile", intervalMs: 5 * 60 * 1000 },
  { key: "cancellations", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "makeup", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "rooms", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "weather", tier: "stable", intervalMs: 60 * 60 * 1000 },
  { key: "schedule_data", tier: "stable", intervalMs: 6 * 60 * 60 * 1000 },
  { key: "student_profile", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "grades", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "exams", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "registration", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "mail_profile", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
  { key: "kwic_home", tier: "stable", intervalMs: 12 * 60 * 60 * 1000 },
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
      lastOk: syncedAt != null,
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
  // Routine cache/session refresh is backend-owned now. Frontend only keeps
  // AI scheduling plus a foreground catch-up in case cache-update events were missed.
  registerBackendRefreshTasks();
  refreshBackendTaskStatuses().catch((err) => {
    console.warn("[Selah] backend task status hydration failed:", err);
  });
  registerTask("ai_scheduler", TASK_LABELS["ai_scheduler"], "stable", 0);
  refreshVisibleBackendCaches();
  syncBackendSessionStatusNow().catch((err) => {
    console.warn("[Selah] backend session status sync failed:", err);
  });
  startAiScheduler();
}

export function stopBackgroundPolling() {
  document.removeEventListener("visibilitychange", handlePollVisibility);
  stopAiScheduler();
}

function handlePollVisibility() {
  if (document.visibilityState === "visible") {
    refreshVisibleBackendCaches();
    syncBackendSessionStatusNow().catch((err) => {
      console.warn("[Selah] backend session status visibility sync failed:", err);
    });
  }
}

// ============ Unified AI Refresh Scheduler ============
// Periodically triggers AI notification analysis and AI todo analysis
// based on user-configured interval (ai_refresh_interval in AiConfig).

let aiRefreshTimer: ReturnType<typeof setInterval> | null = null;
let aiRefreshInitTimeout: ReturnType<typeof setTimeout> | null = null;
const AI_LAST_RUN_KEY = "ai-scheduler-last-run";

function getAiLastRun(): number {
  try { return parseInt(localStorage.getItem(AI_LAST_RUN_KEY) || "0") || 0; } catch { return 0; }
}

function setAiLastRun(ts: number) {
  try { localStorage.setItem(AI_LAST_RUN_KEY, String(ts)); } catch { /* ignore */ }
}

/** Run both AI analyses, updating shared stores. force=true bypasses backend cache. */
export async function runAiRefresh(force: boolean = false): Promise<void> {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;

  if (!await isAiReady()) return;

  // AI todo analysis (runs if Luna is authenticated)
  if (get(lunaAuthState).authenticated) {
    aiRefreshing.update(s => ({ ...s, todo: true }));
    try {
      const result = await aiAnalyzeTodo(force);
      aiTodoStore.set({ result, timestamp: Date.now() });
    } catch (e) {
      console.warn("[AI Scheduler] todo analysis failed:", e);
    } finally {
      aiRefreshing.update(s => ({ ...s, todo: false }));
    }
  }

  // Signal HomePage to refresh AI notifs via the existing store mechanism
  aiNotifStore.update(s => {
    // Set timestamp to 0 to signal that a refresh is needed
    // HomePage will pick this up and run its own fetchAiNotifs with full context
    return s ? { ...s, timestamp: 0 } : s;
  });

  setAiLastRun(Date.now());
}

async function aiSchedulerTick() {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;
  try {
    if (!await isAiReady()) return;
    const cfg = await getAiConfig();
    if (!cfg.ai_refresh_interval) return;
    const intervalMs = cfg.ai_refresh_interval * 60 * 1000;
    const lastRun = getAiLastRun();
    if (Date.now() - lastRun < intervalMs) return;
    debugLog("[AI Scheduler] interval reached, running AI refresh");
    updateTask("ai_scheduler", { running: true });
    await runAiRefresh(true);
    updateTask("ai_scheduler", { running: false, lastRunTs: Date.now(), lastOk: true });
  } catch (e) {
    console.warn("[AI Scheduler] tick error:", e);
    updateTask("ai_scheduler", { running: false, lastRunTs: Date.now(), lastOk: false });
  }
}

export function startAiScheduler() {
  stopAiScheduler();
  // Check after 30s initial delay (let data load first)
  aiRefreshInitTimeout = setTimeout(async () => {
    // Update interval display from config
    try {
      const cfg = await getAiConfig();
      if (cfg.ai_refresh_interval) {
        updateTaskInterval("ai_scheduler", cfg.ai_refresh_interval * 60 * 1000);
      }
    } catch { /* ignore */ }
    aiSchedulerTick();
    // Then check every 5 minutes if interval has been reached
    aiRefreshTimer = setInterval(aiSchedulerTick, 5 * 60 * 1000);
  }, 30_000);
}

export function stopAiScheduler() {
  if (aiRefreshInitTimeout) { clearTimeout(aiRefreshInitTimeout); aiRefreshInitTimeout = null; }
  if (aiRefreshTimer) { clearInterval(aiRefreshTimer); aiRefreshTimer = null; }
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
    // -- Mail stable then volatile --
    { key: "mail_profile", label: TASK_LABELS.mail_profile, platform: "Mail", guard: () => get(mailAuthState).authenticated },
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
  const aiBlocked2b = aiReady && await isLocalStandard2b();
  if (aiReady && !aiBlocked2b) {
    initialItems.push({ key: "ai_notif", label: "AI 通知分析", platform: "AI", status: "pending" });
    if (get(lunaAuthState).authenticated) {
      initialItems.push({ key: "ai_todo", label: "AI 課題分析", platform: "AI", status: "pending" });
    }
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
    if (aiReady && !aiBlocked2b) {
      setItemStatus("ai_notif", "running");
      aiNotifStore.set(null); // clear so HomePage knows to generate fresh
      // Brief wait for views to pick up fresh data
      await new Promise(r => setTimeout(r, 500));
      setItemStatus("ai_notif", "done");

      // AI todo analysis
      if (get(lunaAuthState).authenticated) {
        setItemStatus("ai_todo", "running");
        aiRefreshing.update(s => ({ ...s, todo: true }));
        try {
          const result = await aiAnalyzeTodo(true);
          aiTodoStore.set({ result, timestamp: Date.now() });
          setItemStatus("ai_todo", "done");
        } catch {
          setItemStatus("ai_todo", "error");
        } finally {
          aiRefreshing.update(s => ({ ...s, todo: false }));
        }
      }
      setAiLastRun(Date.now());
    }
    cacheStatus.update(s => ({ ...s, lastUpdated: Date.now() }));
  } finally {
    cacheStatus.update(s => ({ ...s, fullRefreshing: false, refreshingCount: 0 }));
  }
}

// ── Agent (Selah) ──

export interface AgentConversationSummary {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
}

export interface AgentImagePart {
  mime: string;
  data_base64: string;
}

export interface AgentMessage {
  id: number;
  conv_id: string;
  role: "user" | "assistant" | "tool";
  content: string;
  images?: AgentImagePart[] | null;
  tool_name?: string | null;
  tool_result?: unknown;
  created_at: number;
}

export type AgentStreamEvent =
  | { type: "phase"; stage: "planning" | "answering" }
  | { type: "tool_call"; name: string }
  | { type: "tool_result"; name: string; preview: string; ok: boolean }
  | { type: "think"; text: string }
  | { type: "token"; text: string }
  | { type: "done" }
  | { type: "error"; message: string };

export async function agentListConversations(): Promise<AgentConversationSummary[]> {
  if (_isDemo()) return [];
  return invoke<AgentConversationSummary[]>("agent_list_conversations");
}

export async function agentCreateConversation(title?: string): Promise<string> {
  if (_isDemo()) throw new Error("デモモードでは Agent は利用できません");
  return invoke<string>("agent_create_conversation", { title: title ?? null });
}

export async function agentLoadMessages(convId: string): Promise<AgentMessage[]> {
  if (_isDemo()) return [];
  return invoke<AgentMessage[]>("agent_load_messages", { convId });
}

export async function agentSend(
  convId: string,
  content: string,
  images: AgentImagePart[] = [],
): Promise<void> {
  if (_isDemo()) throw new Error("デモモードでは Agent は利用できません");
  return invoke("agent_send", { convId, content, images });
}

export async function agentCancel(convId: string): Promise<void> {
  if (_isDemo()) return;
  return invoke("agent_cancel", { convId });
}

export async function agentDeleteConversation(convId: string): Promise<void> {
  if (_isDemo()) return;
  return invoke("agent_delete_conversation", { convId });
}

export async function agentRenameConversation(convId: string, title: string): Promise<void> {
  if (_isDemo()) return;
  return invoke("agent_rename_conversation", { convId, title });
}

// ============ Image Share ============

/** Save PNG image data to a file using the native save dialog. */
export async function saveImageFile(data: Uint8Array, defaultName: string): Promise<string> {
  return invoke<string>("save_image_file", {
    data: Array.from(data),
    defaultName,
  });
}

/** Copy PNG image data to the system clipboard using native APIs. */
export async function copyImageToClipboard(data: Uint8Array): Promise<void> {
  return invoke("copy_image_to_clipboard", {
    data: Array.from(data),
  });
}

/** Share PNG image data via the native OS share sheet. */
export async function shareImageNative(data: Uint8Array, fileName: string): Promise<void> {
  return invoke("share_image_native", {
    data: Array.from(data),
    fileName,
  });
}
