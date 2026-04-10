<script lang="ts">
  import { openLoginWindow, setAuthFromSession, startBackgroundPolling } from "./api";
  import { authState } from "./stores";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";
  import kgLogoRaw from "../assets/kg-logo.svg?raw";

  let unlisten1: (() => void) | null = null;
  let unlisten2: (() => void) | null = null;
  let unlisten3: (() => void) | null = null;

  onMount(async () => {
    unlisten1 = await listen<{ username: string; display_name: string; student_id: string; faculty: string; department: string }>(
      "login-success",
      (event) => {
        setAuthFromSession(event.payload);
        // Luna auth state is set by the "luna-login-success" event listener in api.ts
        // after Phase 2 (Luna SAML) actually completes.
        startBackgroundPolling();
      }
    );

    unlisten2 = await listen<string>("login-error", (event) => {
      authState.update((s) => ({
        ...s,
        loading: false,
        error: event.payload || "ログインに失敗しました",
      }));
    });

    unlisten3 = await listen<string>("login-cancelled", () => {
      authState.update((s) => ({ ...s, loading: false }));
    });
  });

  onDestroy(() => {
    unlisten1?.();
    unlisten2?.();
    unlisten3?.();
  });

  async function handleLogin() {
    authState.update((s) => ({ ...s, loading: true, error: "" }));
    try {
      await openLoginWindow();
    } catch (e: any) {
      authState.update((s) => ({
        ...s,
        loading: false,
        error: e?.message || e?.toString() || "接続エラー",
      }));
    }
  }
</script>

<div class="login-container">
  <div class="login-card">
    <div class="login-header">
      <span class="login-logo" aria-label="関西学院大学">{@html kgLogoRaw}</span>
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

<style>
  .login-container {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 20px;
    background: var(--bg-secondary);
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
    height: 28px;
    margin-bottom: 8px;
    display: inline-flex;
    align-items: center;
    color: #231f20;
  }
  .login-logo :global(svg) {
    height: 28px;
    width: auto;
  }
  :global([data-theme="dark"]) .login-logo {
    color: var(--text-primary);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .login-logo {
      color: var(--text-primary);
    }
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
</style>
