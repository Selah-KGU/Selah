<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  type Status = "loading" | "ok" | "ng" | "none";

  let clientId = $state("");
  let sessionState = $state<Status>("none");
  let sessionLabel = $state("未確認");
  let accountInfo = $state("");
  let statusMsg = $state("");
  let statusColor = $state("");
  let saveBusy = $state(false);

  async function loadConfig() {
    try {
      const cfg = await invoke<{ client_id?: string }>("mail_get_config");
      clientId = cfg.client_id || "";
    } catch (e) {
      console.error("Failed to load mail config:", e);
    }
    void checkSession();
  }

  async function checkSession() {
    sessionState = "loading";
    sessionLabel = "確認中...";
    accountInfo = "";
    try {
      const s = await invoke<{ authenticated: boolean; display_name?: string; email?: string }>("mail_check_session");
      if (s.authenticated) {
        sessionState = "ok";
        sessionLabel = "認証済み";
        const parts: string[] = [];
        if (s.display_name) parts.push(s.display_name);
        if (s.email) parts.push(s.email);
        if (parts.length) {
          accountInfo = parts.join(" — ");
        } else {
          try {
            const p = await invoke<{ displayName?: string; mail?: string; userPrincipalName?: string }>("mail_fetch_profile");
            const parts2: string[] = [];
            if (p.displayName) parts2.push(p.displayName);
            if (p.mail || p.userPrincipalName) parts2.push(p.mail || p.userPrincipalName || "");
            if (parts2.length) accountInfo = parts2.join(" — ");
          } catch (e) {
            accountInfo = String(e);
          }
        }
      } else {
        sessionState = "ng";
        sessionLabel = "未接続";
      }
    } catch {
      sessionState = "ng";
      sessionLabel = "エラー";
    }
  }

  async function logout() {
    try {
      await invoke("mail_logout");
      statusColor = "var(--green)";
      statusMsg = "ログアウトしました";
      void checkSession();
    } catch (e) {
      statusColor = "var(--red)";
      statusMsg = "ログアウト失敗: " + String(e);
    }
    setTimeout(() => { statusMsg = ""; }, 4000);
  }

  export async function save() {
    saveBusy = true;
    try {
      await invoke("mail_save_config", { config: { client_id: clientId.trim() } });
      void checkSession();
    } catch (e) {
      throw e;
    } finally {
      saveBusy = false;
    }
  }

  function openAzure() {
    invoke("open_external_url", { url: "https://portal.azure.com" }).catch(console.error);
  }

  onMount(() => {
    void loadConfig();
  });
</script>

<div class="hero-card">
  <div class="hero-icon" style="background:linear-gradient(135deg,rgba(0,122,255,0.15),rgba(88,86,214,0.15));">
    <svg viewBox="0 0 20 20" fill="none" stroke="#0055b3" stroke-width="1.3">
      <rect x="2" y="4" width="16" height="12" rx="2"/>
      <polyline points="18 4 10 11 2 4"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">メール設定</h2>
    <p class="panel-desc">Microsoft 365 メール連携の設定を管理します。</p>
  </div>
</div>

<div class="card-label">Microsoft Azure AD</div>
<div class="card">
  <div class="row">
    <span class="row-label">アカウント</span>
    <div class="row-input">
      <div class="session-indicator">
        {#if sessionState === "loading"}<span class="spinner-sm"></span>
        {:else if sessionState === "ok"}<span class="session-dot ok"></span>
        {:else if sessionState === "ng"}<span class="session-dot ng"></span>{/if}
        {sessionLabel}
      </div>
      {#if accountInfo}
        <div class="hint" style="margin-top:2px;">{accountInfo}</div>
      {/if}
    </div>
  </div>
</div>

<div class="action-bar">
  <button class="btn-test danger" onclick={logout}>ログアウト</button>
  {#if statusMsg}
    <span class="hint" style="color:{statusColor};margin-left:4px;">{statusMsg}</span>
  {/if}
</div>

<details style="margin-top:12px;">
  <summary>独自クライアント ID を使用する（上級者向け）</summary>
  <div class="card-label" style="margin-top:8px;">Azure AD クライアント ID</div>
  <div class="card">
    <div class="row">
      <span class="row-label">クライアント ID</span>
      <div class="row-input">
        <input
          type="text"
          bind:value={clientId}
          placeholder="空欄 = デフォルト"
          spellcheck="false"
          style="font-family:monospace;font-size:11px;"
        />
        <div class="hint">Azure Portal で登録したアプリケーション (クライアント) ID を入力してください。空欄の場合はデフォルト値が使用されます。</div>
      </div>
    </div>
  </div>
  <div class="card-label" style="margin-top:12px;">独自クライアント ID の取得方法</div>
  <div class="card" style="padding:12px 14px;font-size:11px;color:var(--text-secondary);line-height:1.6;">
    <ol style="margin:0;padding-left:18px;">
      <li>
        <a href="https://portal.azure.com" style="color:var(--blue);" onclick={(e) => { e.preventDefault(); openAzure(); }}>portal.azure.com</a>
        でアプリを登録
      </li>
      <li>サポートされるアカウントの種類: 「任意の組織ディレクトリ内のアカウント」</li>
      <li>リダイレクト URI: 「パブリッククライアント」→ <code>http://localhost</code></li>
      <li>API のアクセス許可: Microsoft Graph → <code>Mail.Read</code></li>
      <li>認証 → 「パブリッククライアントフローを許可する」を「はい」に</li>
    </ol>
  </div>
</details>

<style>
  .action-bar {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 4px;
  }
  :global(.settings-main .btn-test.danger) {
    background: rgba(255, 59, 48, 0.08);
    color: var(--red);
    border-color: rgba(255, 59, 48, 0.2);
  }
  :global(.settings-main .btn-test.danger:hover:not(:disabled)) {
    background: var(--red);
    color: #fff;
    border-color: var(--red);
  }
  code {
    font-size: 10px;
    background: var(--bg-hover);
    padding: 1px 4px;
    border-radius: 3px;
  }
  details summary {
    cursor: pointer;
    font-size: 11px;
    color: var(--text-secondary);
    user-select: none;
  }
</style>
