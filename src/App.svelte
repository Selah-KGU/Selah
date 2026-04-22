<script lang="ts">
  import "./styles.css";
  import Login from "./lib/Login.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import { authState, lunaAuthState, kwicAuthState, mailAuthState, reloginInProgress, sessionExpired, registerTask, updateTask, invalidateCache } from "./lib/stores";
  import { restoreAllSessions, validateSession, triggerRelogin, startBackgroundPolling, stopBackgroundPolling, syncSession, lunaCheckSession, kwicCheckSession, mailCheckSession, setAuthFromSession, serviceRegistry } from "./lib/api";
  import { startTrayStatus, stopTrayStatus } from "./lib/trayStatus";
  import { listen } from "@tauri-apps/api/event";
  import { get } from "svelte/store";
  import { onMount, onDestroy } from "svelte";
  // Persistent latch: once the user has EVER logged in, always show Dashboard
  // (with cached data + re-auth badge). Only cleared by explicit logout.
  // The flag is SET in setAuthFromSession() (api.ts) and CLEARED in logout().
  let everLoggedIn = $state(!!localStorage.getItem("selah-ever-auth"));
  let currentView = $derived(($authState.authenticated || $sessionExpired || everLoggedIn) ? "dashboard" : "login");
  let restoring = $state(true);
  let validating = false;
  let lastValidateTime = 0;
  const VALIDATE_COOLDOWN = 60_000; // 60 seconds minimum between validations
  let intervalId: ReturnType<typeof setInterval> | null = null;
  let unlistenLogout: (() => void) | null = null;

  // Backoff: track consecutive sync failures per service to avoid spamming headless WebViews
  const syncFailures: Record<string, { count: number; backoffUntil: number }> = {
    luna: { count: 0, backoffUntil: 0 },
    kwic: { count: 0, backoffUntil: 0 },
  };
  function shouldSkipSync(service: string): boolean {
    const f = syncFailures[service];
    if (!f || f.count === 0) return false;
    return Date.now() < f.backoffUntil;
  }
  function recordSyncResult(service: string, ok: boolean) {
    const f = syncFailures[service];
    if (!f) return;
    if (ok) { f.count = 0; f.backoffUntil = 0; return; }
    f.count++;
    // Exponential backoff: 5min, 10min, 20min, capped at 30min
    const delay = Math.min(5 * 60_000 * Math.pow(2, f.count - 1), 30 * 60_000);
    f.backoffUntil = Date.now() + delay;
  }

  async function restoreDemoState(): Promise<boolean> {
    const { restoreDemo } = await import("./lib/demo");
    return restoreDemo();
  }

  async function isDemoModeEnabled(): Promise<boolean> {
    const { isDemoMode } = await import("./lib/demo");
    return isDemoMode();
  }

  onMount(async () => {
    // Handle logout triggered from settings window (or other windows)
    unlistenLogout = await listen("logout", async () => {
      const { deactivateDemo, isDemoMode: checkDemo } = await import("./lib/demo");
      if (checkDemo()) deactivateDemo();
      stopBackgroundPolling();
      sessionExpired.set(false);
      for (const svc of Object.values(serviceRegistry)) svc.onReset();
      invalidateCache();
      try { localStorage.removeItem("selah-ever-auth"); } catch {}
      everLoggedIn = false;
    });

    // Demo mode: restore from previous session, skip real network calls
    if (await restoreDemoState()) {
      restoring = false;
      return;
    }

    try {
      // Restore all service sessions (KGC + Luna + future)
      const session = await restoreAllSessions();
      console.log("[Selah] App.onMount: restoreAllSessions returned", session ? "non-null" : "null",
        "authState.authenticated =", get(authState).authenticated,
        "sessionExpired =", get(sessionExpired),
        "everLoggedIn =", everLoggedIn);
      if (session) {
        startBackgroundPolling();
      } else if (everLoggedIn) {
        // Had a previous session (from a past app run) but recovery failed.
        // Set sessionExpired so the re-auth badge shows, and start polling
        // so cached/SWR data is served.
        sessionExpired.set(true);
        startBackgroundPolling();
      }
      startTrayStatus();
    } catch (e) {
      console.warn("Session restore failed:", e);
      if (everLoggedIn) {
        sessionExpired.set(true);
        startBackgroundPolling();
      }
    } finally {
      restoring = false;
    }

    // Periodic session validation (every 3 minutes)
    registerTask("session_validate", "セッション検証 (KGC/Luna/KWIC)", "system", 3 * 60 * 1000);
    intervalId = setInterval(() => {
      if (document.visibilityState === "visible") {
        doValidate();
      }
    }, 3 * 60 * 1000);

    // Validate on visibility change (tab/window comes back to foreground)
    document.addEventListener("visibilitychange", handleVisibility);
  });

  onDestroy(() => {
    unlistenLogout?.();
    stopTrayStatus();
    stopBackgroundPolling();
    if (intervalId) clearInterval(intervalId);
    document.removeEventListener("visibilitychange", handleVisibility);
  });

  function handleVisibility() {
    if (document.visibilityState === "visible") {
      doValidate();
    }
  }

  async function doValidate() {
    if (await isDemoModeEnabled()) return;
    // Allow validation when session is expired (need to attempt recovery)
    if ((!get(authState).authenticated && !get(sessionExpired)) || validating || get(reloginInProgress)) return;
    const now = Date.now();
    if (now - lastValidateTime < VALIDATE_COOLDOWN) return;

    validating = true;
    lastValidateTime = now;
    updateTask("session_validate", { running: true });
    try {
      // Validate all SAML services in parallel
      const [kgcStatus, lunaValid, kwicValid] = await Promise.all([
        validateSession().catch(() => ({ valid: false } as { valid: boolean })),
        get(lunaAuthState).authenticated
          ? lunaCheckSession().catch(() => false)
          : Promise.resolve(false),
        get(kwicAuthState).authenticated
          ? kwicCheckSession().catch(() => false)
          : Promise.resolve(false),
      ]);

      // Track which services need refresh (only if they WERE authenticated)
      const needsRefresh: string[] = [];
      if (!kgcStatus.valid) needsRefresh.push("kgc");
      if (!lunaValid && get(lunaAuthState).authenticated && !shouldSkipSync("luna")) needsRefresh.push("luna");
      if (!kwicValid && get(kwicAuthState).authenticated && !shouldSkipSync("kwic")) needsRefresh.push("kwic");

      if (needsRefresh.length === 0) {
        // All good
        if (get(sessionExpired)) sessionExpired.set(false);
        if (lunaValid) recordSyncResult("luna", true);
        if (kwicValid) recordSyncResult("kwic", true);
      } else if (needsRefresh.length === 3) {
        // All three expired — Okta is likely dead, need full recovery
        console.log("[Selah] All services expired, triggering full recovery");
        await triggerRelogin();
      } else {
        // Some services expired — targeted refresh with cross-renewal
        // Don't clear sessionExpired yet — only clear on confirmed KGC recovery
        const refreshTasks = needsRefresh.map(svc => {
          if (shouldSkipSync(svc)) return Promise.resolve();
          return syncSession(svc).then(ok => {
            recordSyncResult(svc, ok);
            if (ok) serviceRegistry[svc].onRecovered();
            // Don't reset KGC here — let triggerRelogin handle it
            // (premature reset drops user to Login screen before badge can show)
            else if (svc !== "kgc") serviceRegistry[svc].onReset();
          }).catch(() => { recordSyncResult(svc, false); });
        });
        await Promise.allSettled(refreshTasks);
        if (needsRefresh.includes("kgc")) {
          // Re-validate KGC before deciding to escalate
          const kgcNow = await validateSession().catch(() => ({ valid: false } as { valid: boolean }));
          if (!kgcNow.valid) {
            console.log("[Selah] KGC targeted refresh failed, escalating to full recovery");
            await triggerRelogin();
          } else {
            setAuthFromSession(kgcNow as any);
            sessionExpired.set(false);
          }
        } else {
          // KGC is fine — safe to clear expired state
          if (get(sessionExpired)) sessionExpired.set(false);
        }
      }

      // Mail: validate OAuth token if authenticated
      if (get(mailAuthState).authenticated) {
        try {
          const mailStatus = await mailCheckSession();
          if (!mailStatus.authenticated) {
            mailAuthState.set({ authenticated: false, email: "", displayName: "" });
          }
        } catch {
          mailAuthState.set({ authenticated: false, email: "", displayName: "" });
        }
      }
      updateTask("session_validate", { running: false, lastRunTs: Date.now(), lastOk: true });
    } catch (e) {
      console.warn("[Selah] Session validation/recovery error:", e);
      updateTask("session_validate", { running: false, lastRunTs: Date.now(), lastOk: false });
    } finally {
      validating = false;
    }
  }
</script>

{#if currentView === "login" && !restoring}
  <main class="app-main">
    <div class="page-transition">
      <Login />
    </div>
  </main>
{:else}
  <Dashboard />
{/if}

<style>
  .app-main {
    flex: 1;
    overflow: auto;
  }
  .page-transition {
    height: 100%;
    animation: fade-in-scale 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }
</style>
