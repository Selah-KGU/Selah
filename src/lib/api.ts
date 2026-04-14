import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
import type { ScheduleResponse, AiScheduleResult, AiTodoAnalysis } from "./types";
import { authState, lunaAuthState, kwicAuthState, mailAuthState, gcalAuthState, invalidateCache, reloginInProgress, sessionExpired, refreshCache, registerTask, updateTask, cacheStatus } from "./stores";
import type { RefreshItemStatus } from "./stores";
import { get } from "svelte/store";

// Global listeners — app-lifetime, no cleanup needed
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

interface SessionStatus {
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
        console.log("[Selah] kgc.onReset: sessionExpired=true, keeping authState for cached view");
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
      console.log("[Selah] Session expired detected:", msg);
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
  try { localStorage.setItem(EVER_AUTH_KEY, "1"); } catch {}
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
      console.log(`[Selah] Cross-renewal: ${succeededService} alive -> trying ${svc}`);
      syncSession(svc).then(ok => {
        if (ok) {
          serviceRegistry[svc].onRecovered();
          console.log(`[Selah] Cross-renewal: ${svc} recovered`);
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
    console.log("[Selah] Session expired, trying headless refresh...");
    try {
      const ok = await syncSession("all");
      if (ok) {
        console.log("[Selah] Headless refresh: at least one service recovered");
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
        console.log("[Selah] KGC failed but secondary services may be alive");
      }
    } catch (e) {
      console.warn("[Selah] Headless refresh error:", e);
    }

    // Phase 2: Okta SSO expired — mark session as expired and let user initiate re-login
    console.log("[Selah] Okta expired, marking session as expired (user can re-verify from titlebar)");
    sessionExpired.set(true);
  })().finally(() => { recoveryPromise = null; lastRecoveryTime = Date.now(); });

  return recoveryPromise;
}

/**
 * User-initiated re-login from the titlebar badge.
 * Opens a visible login window and on success clears sessionExpired + refreshes all data.
 */
export async function initiateRelogin(): Promise<void> {
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
      console.log("[Selah] Transient error, retrying once...");
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
        console.log("[Selah] Recovery failed:", recoveryErr);
        throw recoveryErr;
      }
      if (get(sessionExpired)) throw err;
      return await fn();
    }

    // Mail expired → OAuth token revoked, no headless recovery possible
    if (expiredService === "mail") {
      console.log("[Selah] Mail auth expired, resetting mail state");
      serviceRegistry.mail.onReset();
      throw err;
    }

    // Secondary service (Luna/KWIC) expired → try headless sync for just that service
    const svc = serviceRegistry[expiredService];
    console.log(`[Selah] ${expiredService} session expired, trying targeted refresh...`);
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
  const [initialStatus, states] = await Promise.all([
    checkSession(),
    getSessionStates().catch(() => ({ kgc: false, luna: false, kwic: false })),
  ]);
  let status = initialStatus;
  console.log("[Selah] restoreAllSessions: initial check_session =", JSON.stringify(status));
  console.log("[Selah] restoreAllSessions: session states =", JSON.stringify(states));

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
    console.log(`[Selah] Disk sessions expired, syncing in parallel: ${syncNeeded.join(", ")}`);
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
          console.log("[Selah] Headless KGC refresh succeeded");
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
    console.log("[Selah] Re-checking KGC after parallel sync...");
    const recheck = await checkSession().catch(() => null);
    if (recheck?.valid) {
      status = recheck;
      console.log("[Selah] KGC recovered via cross-renewal");
    }
    // Keep original status (with disk user info) if re-check also failed
  }

  if (!status.valid) {
    console.log("[Selah] restoreAllSessions: KGC invalid after all recovery attempts. status =", JSON.stringify(status), "states.kgc =", states.kgc);
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
      console.log("[Selah] restoreAllSessions: showing cached Dashboard with re-auth badge");
      return status; // non-null: App.svelte will show Dashboard
    }
    console.log("[Selah] restoreAllSessions: no disk session, returning null -> Login page");
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
  return invoke<boolean>("luna_check_session");
}

export async function kwicCheckSession(): Promise<boolean> {
  return invoke<boolean>("kwic_check_session");
}

export async function kwicFetchHome(): Promise<KwicPortalHome> {
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
  return invoke<WeatherData>("fetch_weather");
}

export async function kwicFetchDetail(n: KwicPortalNotification): Promise<KwicNotificationDetail> {
  return kwicInvoke<KwicNotificationDetail>("kwic_fetch_detail", {
    informationId: n.id,
    informationType: n.information_type,
    personCategoryCd: n.person_category_cd,
    categoryCd: n.category_cd,
  });
}

export async function kwicFetchSubportal(tagCd: string): Promise<KwicSubportalData> {
  return kwicInvoke<KwicSubportalData>("kwic_fetch_subportal", { tagCd });
}

export async function kwicOpenLink(url: string, title: string): Promise<void> {
  return invoke<void>("kwic_open_link", { url, title });
}

export async function kwicOpenDetail(item: { id: string; title: string; information_type: string; person_category_cd: string; category_cd: string }): Promise<void> {
  return invoke<void>("kwic_open_detail_window", {
    title: item.title,
    informationId: item.id,
    informationType: item.information_type,
    personCategoryCd: item.person_category_cd,
    categoryCd: item.category_cd,
  });
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
  return invoke<MailSessionStatus>("mail_check_session");
}

export async function mailOpenLogin(): Promise<void> {
  return invoke<void>("mail_open_login");
}

export async function mailFetchProfile(): Promise<MailProfile> {
  return invoke<MailProfile>("mail_fetch_profile");
}

export async function mailFetchInbox(top?: number, skip?: number): Promise<MailMessage[]> {
  return withSessionGuard(() => invoke<MailMessage[]>("mail_fetch_inbox", { top: top ?? 20, skip: skip ?? 0 }));
}

export async function mailFetchMessage(messageId: string): Promise<MailDetail> {
  return withSessionGuard(() => invoke<MailDetail>("mail_fetch_message", { messageId }));
}

export async function mailFetchAttachments(messageId: string): Promise<MailAttachment[]> {
  return withSessionGuard(() => invoke<MailAttachment[]>("mail_fetch_attachments", { messageId }));
}

export async function mailDownloadAttachment(messageId: string, attachmentId: string, fileName: string): Promise<string> {
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
  return invoke<GcalStatus>("gcal_check_session");
}

export async function gcalSyncTimetable(entries: GcalSyncEntry[], weekLabel: string): Promise<string> {
  return invoke<string>("gcal_sync_timetable", { entries, weekLabel });
}

export async function gcalOpenLogin(): Promise<void> {
  return invoke<void>("gcal_open_login");
}

export async function gcalDisconnect(): Promise<void> {
  return invoke<void>("gcal_disconnect");
}

export async function gcalGetConfig(): Promise<{ client_id: string; client_secret: string }> {
  return invoke("gcal_get_config");
}

export async function gcalSaveConfig(clientId: string, clientSecret: string): Promise<void> {
  return invoke("gcal_save_config", { clientId, clientSecret });
}

export async function gcalClearCalendar(): Promise<void> {
  return invoke("gcal_clear_calendar");
}

export async function syncCalendar(entries: GcalSyncEntry[], weekLabel: string): Promise<string> {
  return invoke<string>("sync_calendar", { entries, weekLabel });
}

export async function getDataCache(key: string): Promise<string | null> {
  return invoke<string | null>("get_data_cache", { key });
}

export async function saveDataCache(key: string, json: string): Promise<void> {
  return invoke("save_data_cache", { key, json });
}

// ---------- Public API ----------

export async function openLoginWindow(): Promise<void> {
  await invoke("open_login_window");
}

export async function logout(): Promise<void> {
  stopBackgroundPolling();
  await invoke("logout");
  // Clear sessionExpired FIRST so kgc.onReset actually wipes authState
  sessionExpired.set(false);
  for (const svc of Object.values(serviceRegistry)) svc.onReset();
  invalidateCache();
  // Clear the persistent "ever logged in" flag so Login page shows
  try { localStorage.removeItem(EVER_AUTH_KEY); } catch {}
}

async function checkSession(): Promise<SessionStatus> {
  return await invoke<SessionStatus>("check_session");
}

export async function validateSession(): Promise<SessionStatus> {
  return await invoke<SessionStatus>("validate_session");
}

// ── AI-driven schedule (DB-backed, KGC+Luna raw + AI analysis) ──

export async function getScheduleSnapshot(): Promise<ScheduleResponse> {
  return invoke<ScheduleResponse>("get_schedule_snapshot");
}

export async function syncScheduleData(): Promise<ScheduleResponse> {
  return withSessionGuard(() => invoke<ScheduleResponse>("sync_schedule_data"));
}

export async function enrichSchedule(): Promise<void> {
  return invoke<void>("enrich_schedule");
}

export async function refreshLunaCounts(): Promise<number> {
  return invoke<number>("refresh_luna_counts");
}

export async function aiGenerateSchedule(
  currentWeekLabel: string,
  nextWeekLabel: string,
  force: boolean = false,
): Promise<AiScheduleResult> {
  return invoke<AiScheduleResult>("ai_generate_schedule", {
    currentWeekLabel,
    nextWeekLabel,
    force,
  });
}

export async function aiAnalyzeTodo(force: boolean = false): Promise<AiTodoAnalysis> {
  return invoke<AiTodoAnalysis>("ai_analyze_todo", { force });
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

export async function aiChat(messages: AiChatMessage[]): Promise<string> {
  return invoke<string>("ai_chat", { messages });
}

export async function openSettingsWindow(): Promise<void> {
  return invoke<void>("open_settings_window");
}

export async function openProfileEditWindow(): Promise<void> {
  return invoke<void>("open_profile_edit_window");
}

// ============ Background Polling ============
// Two tiers:
//   - Volatile (5 min): notifications, todo, change info, weather
//   - Stable (12 hours): schedule_data, grades, exams, registration

const POLL_INTERVAL = 5 * 60 * 1000; // 5 minutes
const STABLE_POLL_INTERVAL = 12 * 60 * 60 * 1000; // 12 hours
let pollTimer: ReturnType<typeof setInterval> | null = null;
let stablePollTimer: ReturnType<typeof setInterval> | null = null;
let initialPollTimeout: ReturnType<typeof setTimeout> | null = null;
let stablePollInitTimeout: ReturnType<typeof setTimeout> | null = null;
let preemptiveRenewalTimer: ReturnType<typeof setInterval> | null = null;

interface PollTarget {
  key: string;
  fetcher: () => Promise<any>;
  /** Only poll when this returns true */
  guard?: () => boolean;
}

function getVolatileTargets(): PollTarget[] {
  return [
    { key: "notifications", fetcher: fetchNotifications },
    { key: "luna_todo", fetcher: () => lunaInvoke<any>("luna_fetch_todo"), guard: () => get(lunaAuthState).authenticated },
    { key: "luna_updates", fetcher: () => lunaInvoke<any>("luna_fetch_updates"), guard: () => get(lunaAuthState).authenticated },
    { key: "kwic_home", fetcher: kwicFetchHome, guard: () => get(kwicAuthState).authenticated },
    { key: "weather", fetcher: fetchWeather },
    { key: "mail_inbox", fetcher: () => mailFetchInbox(20, 0), guard: () => get(mailAuthState).authenticated },
  ];
}

function getStableTargets(): PollTarget[] {
  return [
    { key: "grades", fetcher: fetchGrades },
    { key: "exams", fetcher: fetchExamTimetable },
    { key: "registration", fetcher: fetchRegistration },
    { key: "cancellations", fetcher: fetchCancellations },
    { key: "makeup", fetcher: fetchMakeupClasses },
    { key: "rooms", fetcher: fetchRoomChanges },
    { key: "student_profile", fetcher: fetchStudentProfile },
    { key: "mail_profile", fetcher: mailFetchProfile, guard: () => get(mailAuthState).authenticated },
  ];
}

function doPoll() {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;
  const promises: Promise<any>[] = [];
  for (const t of getVolatileTargets()) {
    if (t.guard && !t.guard()) continue;
    updateTask(t.key, { running: true });
    cacheStatus.update(s => ({ ...s, refreshingCount: s.refreshingCount + 1 }));
    const p = refreshCache(t.key, t.fetcher);
    if (p) {
      const tracked = p.then((data) => {
        updateTask(t.key, { running: false, lastRunTs: Date.now(), lastOk: data !== undefined });
      }).finally(() => {
        cacheStatus.update(s => ({ ...s, refreshingCount: Math.max(0, s.refreshingCount - 1) }));
      });
      promises.push(tracked);
    } else {
      updateTask(t.key, { running: false });
      cacheStatus.update(s => ({ ...s, refreshingCount: Math.max(0, s.refreshingCount - 1) }));
    }
  }
  if (promises.length) {
    Promise.all(promises).then(() => {
      cacheStatus.update(s => ({ ...s, lastUpdated: Date.now() }));
    });
  }
}

function doStablePoll() {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;
  const promises: Promise<any>[] = [];
  for (const t of getStableTargets()) {
    if (t.guard && !t.guard()) continue;
    updateTask(t.key, { running: true });
    cacheStatus.update(s => ({ ...s, refreshingCount: s.refreshingCount + 1 }));
    const p = refreshCache(t.key, t.fetcher);
    if (p) {
      const tracked = p.then((data) => {
        updateTask(t.key, { running: false, lastRunTs: Date.now(), lastOk: data !== undefined });
      }).finally(() => {
        cacheStatus.update(s => ({ ...s, refreshingCount: Math.max(0, s.refreshingCount - 1) }));
      });
      promises.push(tracked);
    } else {
      updateTask(t.key, { running: false });
      cacheStatus.update(s => ({ ...s, refreshingCount: Math.max(0, s.refreshingCount - 1) }));
    }
  }
  if (promises.length) {
    Promise.all(promises).then(() => {
      cacheStatus.update(s => ({ ...s, lastUpdated: Date.now() }));
    });
  }
}

const PREEMPTIVE_RENEWAL_THRESHOLD = 300; // 5 minutes in seconds

async function getSessionExpiry(): Promise<number | null> {
  return invoke<number | null>("get_session_expiry");
}

async function checkPreemptiveRenewal() {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;
  updateTask("preemptive_renewal", { running: true });
  try {
    const secs = await getSessionExpiry();
    if (secs !== null && secs <= PREEMPTIVE_RENEWAL_THRESHOLD) {
      console.log(`[preemptive-renewal] Cookie expiry in ${secs}s, triggering sync`);
      await syncSession("all");
    }
    updateTask("preemptive_renewal", { running: false, lastRunTs: Date.now(), lastOk: true });
  } catch (e) {
    console.warn("[preemptive-renewal] check failed:", e);
    updateTask("preemptive_renewal", { running: false, lastRunTs: Date.now(), lastOk: false });
  }
}

const TASK_LABELS: Record<string, string> = {
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
};

function registerAllTasks() {
  for (const t of getVolatileTargets()) registerTask(t.key, TASK_LABELS[t.key] ?? t.key, "volatile", POLL_INTERVAL);
  for (const t of getStableTargets()) registerTask(t.key, TASK_LABELS[t.key] ?? t.key, "stable", STABLE_POLL_INTERVAL);
  registerTask("preemptive_renewal", TASK_LABELS["preemptive_renewal"], "system", 3 * 60 * 1000);
}

export function startBackgroundPolling() {
  stopBackgroundPolling();
  registerAllTasks();
  // Initial volatile poll after a short delay (let views mount first)
  initialPollTimeout = setTimeout(doPoll, 10_000);
  pollTimer = setInterval(() => {
    if (document.visibilityState === "visible") doPoll();
  }, POLL_INTERVAL);
  // Stable data: initial fetch after views mount, then refresh every 12 hours
  stablePollInitTimeout = setTimeout(doStablePoll, 15_000);
  stablePollTimer = setInterval(() => {
    doStablePoll();
  }, STABLE_POLL_INTERVAL);
  // Preemptive session renewal: check cookie expiry every 3 min
  preemptiveRenewalTimer = setInterval(checkPreemptiveRenewal, 3 * 60 * 1000);
  // Also poll when window becomes visible after being hidden
  document.addEventListener("visibilitychange", handlePollVisibility);
}

export function stopBackgroundPolling() {
  if (initialPollTimeout) { clearTimeout(initialPollTimeout); initialPollTimeout = null; }
  if (stablePollInitTimeout) { clearTimeout(stablePollInitTimeout); stablePollInitTimeout = null; }
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
  if (stablePollTimer) { clearInterval(stablePollTimer); stablePollTimer = null; }
  if (preemptiveRenewalTimer) { clearInterval(preemptiveRenewalTimer); preemptiveRenewalTimer = null; }
  document.removeEventListener("visibilitychange", handlePollVisibility);
}

function handlePollVisibility() {
  if (document.visibilityState === "visible") doPoll();
}

/** One-click full refresh: invalidate all caches and re-fetch everything */
interface RefreshStep {
  key: string;
  label: string;
  platform: string;
  fetcher: () => Promise<any>;
  guard?: () => boolean;
}

/** Ordered refresh sequence: persistent data first, real-time data later. Serial within each platform. */
function getRefreshSequence(): RefreshStep[] {
  return [
    // -- KGC stable (persistent) --
    { key: "student_profile", label: TASK_LABELS.student_profile, platform: "KGC", fetcher: fetchStudentProfile },
    { key: "grades", label: TASK_LABELS.grades, platform: "KGC", fetcher: fetchGrades },
    { key: "exams", label: TASK_LABELS.exams, platform: "KGC", fetcher: fetchExamTimetable },
    { key: "registration", label: TASK_LABELS.registration, platform: "KGC", fetcher: fetchRegistration },
    { key: "cancellations", label: TASK_LABELS.cancellations, platform: "KGC", fetcher: fetchCancellations },
    { key: "makeup", label: TASK_LABELS.makeup, platform: "KGC", fetcher: fetchMakeupClasses },
    { key: "rooms", label: TASK_LABELS.rooms, platform: "KGC", fetcher: fetchRoomChanges },
    // -- KGC volatile (real-time) --
    { key: "notifications", label: TASK_LABELS.notifications, platform: "KGC", fetcher: fetchNotifications },
    { key: "kwic_home", label: TASK_LABELS.kwic_home, platform: "KGC", fetcher: kwicFetchHome, guard: () => get(kwicAuthState).authenticated },
    // -- Luna --
    { key: "luna_todo", label: TASK_LABELS.luna_todo, platform: "Luna", fetcher: () => lunaInvoke<any>("luna_fetch_todo"), guard: () => get(lunaAuthState).authenticated },
    { key: "luna_updates", label: TASK_LABELS.luna_updates, platform: "Luna", fetcher: () => lunaInvoke<any>("luna_fetch_updates"), guard: () => get(lunaAuthState).authenticated },
    // -- Mail stable then volatile --
    { key: "mail_profile", label: TASK_LABELS.mail_profile, platform: "Mail", fetcher: mailFetchProfile, guard: () => get(mailAuthState).authenticated },
    { key: "mail_inbox", label: TASK_LABELS.mail_inbox, platform: "Mail", fetcher: () => mailFetchInbox(20, 0), guard: () => get(mailAuthState).authenticated },
    // -- Other --
    { key: "weather", label: TASK_LABELS.weather, platform: "Other", fetcher: fetchWeather },
  ];
}

export async function refreshAllData(): Promise<void> {
  if (!get(authState).authenticated || get(reloginInProgress) || get(sessionExpired)) return;

  const sequence = getRefreshSequence();
  // Filter out guarded items that aren't available
  const steps = sequence.filter(s => !s.guard || s.guard());
  // Build initial item status list
  const initialItems: RefreshItemStatus[] = steps.map(s => ({
    key: s.key, label: s.label, platform: s.platform, status: "pending",
  }));
  // Add schedule sync as the last item
  initialItems.push({ key: "schedule_sync", label: "時間割同期", platform: "KGC", status: "pending" });

  cacheStatus.update(s => ({ ...s, fullRefreshing: true, refreshingCount: steps.length + 1, items: initialItems }));
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
        const data = await refreshCache(step.key, step.fetcher);
        updateTask(step.key, { running: false, lastRunTs: Date.now(), lastOk: data !== undefined });
        setItemStatus(step.key, "done");
      } catch {
        updateTask(step.key, { running: false, lastRunTs: Date.now(), lastOk: false });
        setItemStatus(step.key, "error");
      }
    }
    // Schedule sync
    setItemStatus("schedule_sync", "running");
    try {
      await syncScheduleData();
      setItemStatus("schedule_sync", "done");
    } catch {
      setItemStatus("schedule_sync", "error");
    }
    cacheStatus.update(s => ({ ...s, lastUpdated: Date.now() }));
  } finally {
    cacheStatus.update(s => ({ ...s, fullRefreshing: false, refreshingCount: 0 }));
  }
}
