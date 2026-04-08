<script lang="ts">
  import "./styles.css";
  import Login from "./lib/Login.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import DebugPanel from "./lib/DebugPanel.svelte";
  import { authState, lunaAuthState, kwicAuthState, mailAuthState, reloginInProgress, sessionExpired, debugVisible } from "./lib/stores";
  import { restoreAllSessions, validateSession, triggerRelogin, startBackgroundPolling, stopBackgroundPolling, syncSession, lunaCheckSession, kwicCheckSession, mailCheckSession, setAuthFromSession, serviceRegistry } from "./lib/api";
  import { startTrayStatus, stopTrayStatus } from "./lib/trayStatus";
  import { listen } from "@tauri-apps/api/event";
  import { get } from "svelte/store";
  import { onMount, onDestroy } from "svelte";

  let currentView = $derived($authState.authenticated ? "dashboard" : "login");
  let restoring = $state(true);
  let validating = false;
  let lastValidateTime = 0;
  const VALIDATE_COOLDOWN = 60_000; // 60 seconds minimum between validations
  let intervalId: ReturnType<typeof setInterval> | null = null;
  let unlistenDebugToggle: (() => void) | null = null;

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

  onMount(async () => {
    unlistenDebugToggle = await listen("toggle-debug", () => {
      debugVisible.update(v => !v);
    });

    try {
      // Restore all service sessions (KGC + Luna + future)
      const session = await restoreAllSessions();
      if (session) {
        startBackgroundPolling();
      }
      startTrayStatus();
    } catch (e) {
      console.warn("Session restore failed:", e);
    } finally {
      restoring = false;
    }

    // Periodic session validation (every 3 minutes)
    intervalId = setInterval(() => {
      if (document.visibilityState === "visible") {
        doValidate();
      }
    }, 3 * 60 * 1000);

    // Validate on visibility change (tab/window comes back to foreground)
    document.addEventListener("visibilitychange", handleVisibility);
  });

  onDestroy(() => {
    unlistenDebugToggle?.();
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
    if (!get(authState).authenticated || validating || get(reloginInProgress)) return;
    const now = Date.now();
    if (now - lastValidateTime < VALIDATE_COOLDOWN) return;

    validating = true;
    lastValidateTime = now;
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

      // Track which services need refresh
      const needsRefresh: string[] = [];
      if (!kgcStatus.valid) needsRefresh.push("kgc");
      if (!lunaValid && (get(lunaAuthState).authenticated || !shouldSkipSync("luna"))) needsRefresh.push("luna");
      if (!kwicValid && (get(kwicAuthState).authenticated || !shouldSkipSync("kwic"))) needsRefresh.push("kwic");

      if (needsRefresh.length === 0) {
        // All good
        if (get(sessionExpired)) sessionExpired.set(false);
        recordSyncResult("luna", true);
        recordSyncResult("kwic", true);
      } else if (needsRefresh.length === 3) {
        // All three expired — Okta is likely dead, need full recovery
        console.log("[Selah] All services expired, triggering full recovery");
        await triggerRelogin();
      } else {
        // Some services expired — targeted refresh with cross-renewal
        // Even KGC can be refreshed alone; cross-renewal will help
        if (get(sessionExpired)) sessionExpired.set(false);
        const refreshTasks = needsRefresh.map(svc => {
          if (shouldSkipSync(svc)) return Promise.resolve();
          return syncSession(svc).then(ok => {
            recordSyncResult(svc, ok);
            if (ok) serviceRegistry[svc].onRecovered();
            else serviceRegistry[svc].onReset();
          }).catch(() => { recordSyncResult(svc, false); });
        });
        await Promise.allSettled(refreshTasks);
        // If KGC was in the list, cross-renewal + onRecovered is async.
        // Re-validate KGC before deciding to escalate.
        if (needsRefresh.includes("kgc")) {
          const kgcNow = await validateSession().catch(() => ({ valid: false } as { valid: boolean }));
          if (!kgcNow.valid) {
            console.log("[Selah] KGC targeted refresh failed, escalating to full recovery");
            await triggerRelogin();
          } else {
            setAuthFromSession(kgcNow as any);
            sessionExpired.set(false);
          }
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
    } catch (e) {
      console.warn("[Selah] Session validation/recovery error:", e);
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
<DebugPanel />

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
