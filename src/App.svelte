<script lang="ts">
  import "./styles.css";
  import Login from "./lib/Login.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import { demoMode } from "./lib/demoStore";
  import { authState, sessionExpired, invalidateCache } from "./lib/stores";
  import { restoreAllSessions, startBackgroundPolling, stopBackgroundPolling, serviceRegistry } from "./lib/api";
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
  let demoBootFlag = $state(readDemoBootFlag());
  let everLoggedIn = $state(readEverLoggedIn());
  let currentView = $derived(($demoMode || demoBootFlag || $authState.authenticated || $sessionExpired || everLoggedIn) ? "dashboard" : "login");
  let restoring = $state(true);
  let unlistenLogout: (() => void) | null = null;

  async function restoreDemoState(): Promise<boolean> {
    const { restoreDemo } = await import("./lib/demo");
    return restoreDemo();
  }

  onMount(async () => {
    // Handle logout triggered from settings window (or other windows)
    unlistenLogout = await listen("logout", async () => {
      const { deactivateDemo, isDemoMode: checkDemo } = await import("./lib/demo");
      if (checkDemo()) deactivateDemo();
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
    });

    // Demo mode: restore from previous session, skip real network calls
    if (await restoreDemoState()) {
      demoBootFlag = true;
      startTrayStatus();
      void startSilentUpdateCheck();
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
      void startSilentUpdateCheck();
    }
  });

  onDestroy(() => {
    unlistenLogout?.();
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
  <Dashboard />
{/if}

<style>
  .app-main {
    flex: 1;
    overflow: hidden;
  }
  .page-transition {
    height: 100%;
    animation: fade-in-scale 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }
</style>
