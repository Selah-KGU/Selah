<script lang="ts">
  import "./styles.css";
  import Login from "./lib/Login.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import { demoMode } from "./lib/demoStore";
  import { universityLoginPersistencePending, authState, sessionExpired, invalidateCache, activeTab } from "./lib/stores";
  import { openSettingsWindow, restoreAllSessions, startBackgroundPolling, stopBackgroundPolling, serviceRegistry, liveHasActiveSession } from "./lib/api";
  import { ResourceScope } from "./lib/resourceScope";
  import { startTrayStatus, stopTrayStatus } from "./lib/trayStatus";
  import { startSilentUpdateCheck } from "./lib/updater";
  import { listen } from "@tauri-apps/api/event";
  import { get } from "svelte/store";
  import { onMount, onDestroy } from "svelte";
  // Persistent latch: once the user has EVER logged in, always show Dashboard
  // (with cached data + re-auth badge). Only cleared by explicit logout.
  // Demo sessions do not participate in this latch.
  function readDemoBootFlag(): boolean {
    try {
      return localStorage.getItem("selah-demo-mode") === "1";
    } catch {
      return false;
    }
  }
  function readEverLoggedIn(): boolean {
    try {
      if (localStorage.getItem("selah-ever-auth") !== "1") return false;
      const source = localStorage.getItem("selah-ever-auth-source");
      if (source === "real") return true;
      // Backward compatibility: older builds only stored the boolean flag.
      // Keep treating it as a real login latch unless demo mode itself is active.
      if (!source && localStorage.getItem("selah-demo-mode") !== "1") return true;
      return false;
    } catch {
      return false;
    }
  }

  function debugLog(...args: unknown[]): void {
    try {
      if (localStorage.getItem("selah-debug-logs") === "1") console.log(...args);
    } catch {
      // Ignore storage access failures during early app startup.
    }
  }

  let demoBootFlag = $state(readDemoBootFlag());
  let everLoggedIn = $state(readEverLoggedIn());
  let currentView = $derived(($demoMode || demoBootFlag || $authState.authenticated || $sessionExpired || everLoggedIn) ? "dashboard" : "login");
  let restoring = $state(true);
  const resources = new ResourceScope();
  let startupVersion = 0;

  async function restoreDemoState(current: () => boolean): Promise<boolean> {
    const { restoreDemo } = await import("./lib/demo");
    return current() && restoreDemo();
  }

  async function handleLogout() {
    // Invalidate boot reads immediately, before loading the demo module.
    const version = ++startupVersion;
    stopBackgroundPolling();
    stopTrayStatus();
    sessionExpired.set(false);
    for (const svc of Object.values(serviceRegistry)) svc.onReset();
    invalidateCache();
    try {
      localStorage.removeItem("selah-ever-auth");
      localStorage.removeItem("selah-ever-auth-source");
    } catch {}
    demoBootFlag = false;
    everLoggedIn = false;
    restoring = false;
    const { deactivateDemo, isDemoMode: checkDemo } = await import("./lib/demo");
    if (resources.active && version === startupVersion && checkDemo()) deactivateDemo();
  }

  async function initializeApp() {
    const version = startupVersion;
    const current = () => resources.active && version === startupVersion;
    // Subscribe before starting reads. A late registration remains owned even
    // if the root boundary was destroyed while Tauri was registering it.
    try {
      await resources.acquire(() => listen("logout", resources.guard(() => {
        void handleLogout().catch(error => {
          if (resources.active) console.warn("[Selah] logout handling failed:", error);
        });
      })));
    } catch (error) {
      if (current()) console.warn("[Selah] logout subscription failed:", error);
    }
    if (!current()) return;
    // The native STT/session survives a WebView reload. Reopen its UI even if
    // restoring the university login takes longer or is currently unavailable.
    void liveHasActiveSession().then((active) => {
      if (current() && active) {
        everLoggedIn = true;
        activeTab.set("live");
      }
    }).catch((err) => {
      if (current()) console.warn("[Selah] LIVE recovery read failed:", err);
    });

    try {
      // Demo mode: restore from previous session, skip real network calls.
      const demoRestored = await restoreDemoState(current);
      if (!current()) return;
      if (demoRestored) {
        demoBootFlag = true;
        startTrayStatus();
        return;
      }
      // Restore all service sessions (KGC + Luna + future)
      const session = await restoreAllSessions(current);
      if (!current()) return;
      debugLog("[Selah] App.onMount: restoreAllSessions returned", session ? "non-null" : "null",
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
      if (!current()) return;
      console.warn("Session restore failed:", e);
      if (everLoggedIn) {
        sessionExpired.set(true);
        startBackgroundPolling();
      }
    } finally {
      if (current()) {
        restoring = false;
        void startSilentUpdateCheck();
      }
    }
  }

  onMount(() => {
    void initializeApp().catch(error => {
      if (resources.active) console.warn("[Selah] initialization failed:", error);
    });
  });

  onDestroy(() => {
    resources.dispose();
    startupVersion += 1;
    stopTrayStatus();
    stopBackgroundPolling();
  });
</script>

{#if currentView === "login" && !restoring}
  <main class="app-main">
    <div class="page-transition">
      <Login />
    </div>
  </main>
{:else}
  {#if $universityLoginPersistencePending && !$demoMode}
    <div class="login-persistence-notice" role="status">
      ログイン済みですが、認証情報の保存は未完了です。
      <button onclick={() => openSettingsWindow("session")}>設定で保存を再試行</button>
    </div>
  {/if}
  <div class="dashboard-host"><Dashboard /></div>
{/if}

<style>
  .login-persistence-notice {
    padding: 8px 16px 8px 84px;
    flex-shrink: 0;
    color: var(--text-primary);
    background: var(--bg-secondary);
    font-size: 12px;
  }
  .dashboard-host { flex: 1; min-height: 0; }
  .dashboard-host :global(.dashboard) { height: 100%; min-height: 0; }
  .login-persistence-notice button { margin-left: 8px; color: var(--blue); }

  .app-main {
    flex: 1;
    overflow: hidden;
  }
  .page-transition {
    height: 100%;
    animation: fade-in-scale 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }
</style>
