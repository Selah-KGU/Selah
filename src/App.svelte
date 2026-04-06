<script lang="ts">
  import "./styles.css";
  import Login from "./lib/Login.svelte";
  import Dashboard from "./lib/Dashboard.svelte";
  import DebugPanel from "./lib/DebugPanel.svelte";
  import { authState, reloginInProgress, debugVisible } from "./lib/stores";
  import { restoreAllSessions, validateSession, triggerRelogin, startBackgroundPolling, stopBackgroundPolling } from "./lib/api";
  import { listen } from "@tauri-apps/api/event";
  import { get } from "svelte/store";
  import { onMount, onDestroy } from "svelte";

  let currentView = $derived($authState.authenticated ? "dashboard" : "login");
  let restoring = $state(true);
  let validating = false;
  let intervalId: ReturnType<typeof setInterval> | null = null;
  let unlistenDebugToggle: (() => void) | null = null;

  onMount(async () => {
    unlistenDebugToggle = await listen("toggle-debug", () => {
      debugVisible.update(v => !v);
    });

    try {
      // Restore all service sessions (KGC + Luna + future)
      const session = await restoreAllSessions();
      if (session) startBackgroundPolling();
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

    validating = true;
    try {
      const status = await validateSession();
      if (!status.valid) {
        console.log("[Selah] Session expired, triggering recovery...");
        await triggerRelogin();
      }
    } catch (e) {
      console.warn("[Selah] Session validation/recovery error:", e);
    } finally {
      validating = false;
    }
  }
</script>

{#if restoring}
  <main class="app-main">
    <div class="restoring">
      <div class="restoring-spinner"></div>
    </div>
  </main>
{:else if currentView === "login"}
  <main class="app-main">
    <div class="page-transition">
      <Login />
    </div>
  </main>
{:else}
  <Dashboard />
{/if}
{#if $reloginInProgress}
  <div class="relogin-overlay">
    <div class="relogin-card">
      <div class="relogin-spinner"></div>
      <p>セッションが期限切れです</p>
      <p class="relogin-sub">再ログイン中…</p>
    </div>
  </div>
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
  .restoring {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
  }
  .restoring-spinner {
    width: 24px;
    height: 24px;
    border: 2.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  .relogin-overlay {
    position: fixed;
    inset: 0;
    z-index: 9000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    animation: fade-in 0.25s ease both;
  }
  .relogin-card {
    text-align: center;
    padding: 28px 36px;
    border-radius: 14px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
  }
  .relogin-card p {
    margin: 8px 0 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
  }
  .relogin-sub {
    font-size: 12px !important;
    font-weight: 400 !important;
    color: var(--text-secondary) !important;
    opacity: 0.8;
  }
  .relogin-spinner {
    width: 28px;
    height: 28px;
    margin: 0 auto 4px;
    border: 2.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
