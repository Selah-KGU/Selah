<script lang="ts">
  import { openLoginWindow, setAuthFromSession, startBackgroundPolling, enterDemoMode } from "./api";
  import { authState } from "./stores";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, onDestroy } from "svelte";
  import { ResourceScope, acquireResourceGroup } from "./resourceScope";
  import selahLogoUrl from "../assets/logo.png";

  const resources = new ResourceScope();
  onDestroy(() => resources.dispose());
  let listenersBound = false;
  let listenerRegistration: Promise<boolean> | null = null;
  let openingLogin = false;
  let startingDemo = false;

  // Demo mode: 7 taps on logo within 3 seconds
  let logoTapCount = 0;
  let cancelLogoTapTimer: (() => void) | null = null;
  const DEMO_TAPS = 7;
  const DEMO_TAP_WINDOW = 3000;
  let showDemoConfirm = $state(false);
  const isWindows = navigator.userAgent.includes("Windows");
  const appWindow = getCurrentWindow();

  function minimizeWindow() { if (resources.active) void appWindow.minimize(); }
  function toggleMaximize() { if (resources.active) void appWindow.toggleMaximize(); }
  function closeWindow() { if (resources.active) void appWindow.close(); }

  function handleLogoClick() {
    if (!resources.active) return;
    logoTapCount++;
    cancelLogoTapTimer?.();
    cancelLogoTapTimer = resources.schedule(() => {
      logoTapCount = 0;
      cancelLogoTapTimer = null;
    }, DEMO_TAP_WINDOW);

    if (logoTapCount >= DEMO_TAPS) {
      logoTapCount = 0;
      cancelLogoTapTimer();
      cancelLogoTapTimer = null;
      showDemoConfirm = true;
    }
  }

  async function startDemoMode() {
    if (!resources.active || startingDemo) return;
    startingDemo = true;
    showDemoConfirm = false;
    try {
      await enterDemoMode(() => resources.active);
    } catch (e: any) {
      if (!resources.active) return;
      authState.update((s) => ({
        ...s,
        loading: false,
        error: e?.message || e?.toString() || "デモモードの起動に失敗しました",
      }));
    } finally {
      startingDemo = false;
    }
  }

  function cancelDemoMode() {
    if (!resources.active) return;
    showDemoConfirm = false;
  }

  function ensureLoginListeners(): Promise<boolean> {
    if (!resources.active) return Promise.resolve(false);
    if (listenersBound) return Promise.resolve(true);
    if (listenerRegistration) return listenerRegistration;
    const registration = (async () => {
      await acquireResourceGroup(resources, [
        group => listen<{ username: string; display_name: string; student_id: string; faculty: string; department: string }>(
          "login-success", group.guard((event) => {
            setAuthFromSession(event.payload);
            // Luna's API listener applies auth after its SAML phase finishes.
            startBackgroundPolling(true);
          })),

        group => listen<string>("login-error", group.guard((event) => {
          authState.update((s) => ({
            ...s,
            loading: false,
            error: event.payload || "ログインに失敗しました",
          }));
        })),

        group => listen<string>("login-cancelled", group.guard(() => {
          authState.update((s) => ({ ...s, loading: false }));
        })),
      ]);
      if (!resources.active) return false;
      listenersBound = true;
      return true;
    })().finally(() => {
      if (listenerRegistration === registration) listenerRegistration = null;
    });
    listenerRegistration = registration;
    return registration;
  }

  function showLoginError(error: unknown) {
    if (!resources.active) return;
    const detail = error as { message?: string; toString?: () => string } | null | undefined;
    authState.update(s => ({
      ...s,
      loading: false,
      error: detail?.message || detail?.toString?.() || "接続エラー",
    }));
  }

  onMount(() => {
    void ensureLoginListeners().catch(showLoginError);
  });

  async function handleLogin() {
    if (!resources.active || openingLogin) return;
    openingLogin = true;
    authState.update((s) => ({ ...s, loading: true, error: "" }));
    try {
      if (!(await ensureLoginListeners()) || !resources.active) return;
      await openLoginWindow();
    } catch (e) {
      showLoginError(e);
    } finally {
      openingLogin = false;
    }
  }
</script>

<div class="login-container">
  {#if isWindows}
    <div class="login-drag-region" data-tauri-drag-region aria-hidden="true"></div>
    <div class="login-window-controls" aria-label="ウィンドウ操作">
      <button class="login-win-ctrl" type="button" onclick={minimizeWindow} title="最小化" aria-label="最小化">
        <svg width="10" height="1" viewBox="0 0 10 1"><line x1="0" y1="0.5" x2="10" y2="0.5" stroke="currentColor" stroke-width="1" /></svg>
      </button>
      <button class="login-win-ctrl" type="button" onclick={toggleMaximize} title="最大化" aria-label="最大化">
        <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" stroke-width="1" fill="none" /></svg>
      </button>
      <button class="login-win-ctrl login-win-close" type="button" onclick={closeWindow} title="閉じる" aria-label="閉じる">
        <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" stroke-width="1.2" /><line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
    </div>
  {/if}
  <div class="login-card">
    <div class="login-header">
      <button type="button" class="login-logo" aria-label="Selah" onclick={handleLogoClick}><img src={selahLogoUrl} alt="Selah" /></button>
    </div>

    <div class="login-body">
      {#if $authState.error}
        <div class="error-banner">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
          </svg>
          <span>{$authState.error}</span>
        </div>
      {/if}

      <p class="login-desc">
        学生システムにサインインするには、<br />
        関西学院のアカウントが必要です。
      </p>

      <button
        class="btn-login"
        onclick={handleLogin}
        disabled={$authState.loading}
      >
        {#if $authState.loading}
          <span class="loading-spinner"></span>
          サインイン中...
        {:else}
          サインイン
        {/if}
      </button>
    </div>

    <div class="login-footer">
      <p>Okta SSO 経由で安全に接続します</p>
    </div>
  </div>
</div>

{#if showDemoConfirm}
  <div class="demo-confirm-shell" role="presentation">
    <div class="demo-confirm-card" role="dialog" aria-modal="true" aria-labelledby="demo-confirm-title">
      <div id="demo-confirm-title" class="demo-confirm-title">デモモード</div>
      <div class="demo-confirm-body">テストデータでデモモードに入ります。実際のログインは行われません。</div>
      <div class="demo-confirm-actions">
        <button type="button" class="demo-confirm-btn demo-confirm-cancel" onclick={cancelDemoMode}>キャンセル</button>
        <button type="button" class="demo-confirm-btn demo-confirm-enter" onclick={startDemoMode}>デモモードに入る</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .login-container {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 20px;
    background: var(--bg-secondary);
  }

  .login-window-controls {
    position: absolute;
    top: 10px;
    right: 12px;
    z-index: 10;
    display: flex;
    align-items: center;
  }

  .login-drag-region {
    position: absolute;
    top: 0;
    left: 0;
    right: 132px;
    height: 46px;
    z-index: 9;
  }

  .login-win-ctrl {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 30px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }

  .login-win-ctrl:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .login-win-close:hover {
    background: #e81123;
    color: #fff;
  }

  .login-card {
    width: 380px;
    background: var(--bg-card);
    border-radius: 16px;
    border: 0.5px solid var(--border-strong);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    animation: fade-in 0.4s ease;
  }

  .login-header {
    text-align: center;
    padding: 40px 24px 20px;
  }

  .login-logo {
    height: 60px;
    display: inline-flex;
    align-items: center;
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
  }
  .login-logo img {
    height: 60px;
    width: auto;
  }

  .login-body {
    padding: 0 32px 28px;
  }

  .login-desc {
    font-size: 13px;
    color: var(--text-secondary);
    text-align: center;
    margin-bottom: 24px;
    line-height: 1.7;
  }

  .error-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-radius: 10px;
    background: rgba(255, 59, 48, 0.08);
    color: var(--red);
    font-size: 13px;
    margin-bottom: 20px;
    border: 0.5px solid rgba(255, 59, 48, 0.2);
  }

  .btn-login {
    width: 100%;
    padding: 12px;
    background: var(--accent);
    color: var(--text-on-accent);
    font-size: 15px;
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: none;
    border-radius: 10px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-login:hover {
    background: var(--accent-hover);
  }

  .btn-login:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .login-footer {
    padding: 12px 24px 16px;
    text-align: center;
    border-top: 0.5px solid var(--border);
  }

  .login-footer p {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .demo-confirm-shell {
    position: fixed;
    inset: 0;
    z-index: 1100;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(15, 23, 42, 0.36);
  }

  .demo-confirm-card {
    width: min(340px, calc(100vw - 32px));
    background: var(--bg-primary);
    border: 0.5px solid var(--border-strong);
    border-radius: 16px;
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }

  .demo-confirm-title {
    padding: 18px 20px 6px;
    text-align: center;
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .demo-confirm-body {
    padding: 6px 22px 18px;
    text-align: center;
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .demo-confirm-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-top: 0.5px solid var(--border);
  }

  .demo-confirm-btn {
    min-height: 46px;
    border-radius: 0;
    background: transparent;
    font-size: 14px;
    font-weight: 600;
  }

  .demo-confirm-cancel {
    color: var(--text-secondary);
    border-right: 0.5px solid var(--border);
  }

  .demo-confirm-enter {
    color: var(--accent);
  }
</style>
