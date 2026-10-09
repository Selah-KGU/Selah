<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { gcalCheckSession, gcalClearCalendar, gcalDisconnect, gcalGetConfig, gcalOpenLogin, gcalSaveConfig, isDemoActive } from "../../api";
  import { openExternalUrl } from "../../system";

  const DEMO_CALENDAR_CONFIG_KEY = "selah-demo-calendar-config";

  function readDemoCalendarConfig() {
    try {
      const raw = localStorage.getItem(DEMO_CALENDAR_CONFIG_KEY);
      if (!raw) return null;
      return JSON.parse(raw);
    } catch {
      return null;
    }
  }

  function writeDemoCalendarConfig(config: any) {
    try { localStorage.setItem(DEMO_CALENDAR_CONFIG_KEY, JSON.stringify(config)); } catch { /* ignore */ }
  }

  type Status = "loading" | "ok" | "ng" | "none";

  let springStart = $state("2026-04-03");
  let fallStart = $state("2026-09-21");
  let gcalAutoSync = $state("false");
  let calSyncInterval = $state("12");
  let gcalClientId = $state("");
  let gcalClientSecret = $state("");
  let gcalConfigLoaded = $state(false);
  let gcalConfigEdited = $state(false);

  let gcalState = $state<Status>("none");
  let gcalLabel = $state("未確認");

  let gcalStatusMsg = $state("");
  let gcalStatusColor = $state("");
  let saveBusy = $state(false);
  let bindingAccount = $state<{ username: string | null; generation: number; connectionId: string | null; calendarId: string } | null>(null);
  let accountVersion = 0;
  let disposed = false;
  let unlistenAccount: (() => void) | null = null;
  let unlistenLogin: (() => void) | null = null;

  async function loadCalendar() {
    const version = accountVersion;
    if (isDemoActive()) {
      const c = readDemoCalendarConfig() || {};
      springStart = c.spring_start || "2026-04-03";
      fallStart = c.fall_start || "2026-09-21";
      calSyncInterval = String(c.cal_sync_interval || 12);
      gcalAutoSync = c.gcal_auto_sync ? "true" : "false";
      localStorage.setItem("selah-gcal-auto-sync", gcalAutoSync);
      localStorage.setItem("selah-cal-sync-interval", calSyncInterval);
      return;
    }
    try {
      const c = await invoke<any>("get_calendar_config");
      if (disposed || version !== accountVersion) return;
      bindingAccount = { username: c.account_username ?? null, generation: c.account_generation, connectionId: c.connection_id ?? null, calendarId: c.calendar_id ?? "" };
      springStart = c.spring_start || "2026-04-03";
      fallStart = c.fall_start || "2026-09-21";
      calSyncInterval = String(c.cal_sync_interval || 12);
      gcalAutoSync = c.gcal_auto_sync ? "true" : "false";
      localStorage.setItem("selah-gcal-auto-sync", gcalAutoSync);
      localStorage.setItem("selah-cal-sync-interval", calSyncInterval);
    } catch (e) {
      console.error("Failed to load calendar config:", e);
    }
  }

  async function loadGcal() {
    try {
      const cfg = await gcalGetConfig();
      gcalClientId = cfg.client_id || "";
      gcalClientSecret = cfg.client_secret || "";
      gcalConfigLoaded = true;
      gcalConfigEdited = false;
    } catch (e) {
      gcalConfigLoaded = false;
      gcalStatusColor = "var(--red)";
      gcalStatusMsg = "Google Calendar の設定を読み込めません。保存先を確認してください。";
      console.error("Failed to load gcal config:", e);
    }
    void checkGcalSession();
  }

  async function checkGcalSession() {
    const version = accountVersion;
    gcalState = "loading";
    gcalLabel = "確認中...";
    try {
      const s = await gcalCheckSession();
      if (disposed || version !== accountVersion) return;
      if (s.authenticated) {
        gcalState = "ok";
        gcalLabel = s.calendar_exists ? `認証済み (${s.synced_events ?? 0}件同期済)` : "認証済み (未同期)";
      } else {
        gcalState = "ng";
        gcalLabel = "未接続";
      }
    } catch {
      if (disposed || version !== accountVersion) return;
      gcalState = "ng";
      gcalLabel = "エラー";
    }
  }

  async function gcalLogin() {
    gcalStatusColor = "var(--text-secondary)";
    gcalStatusMsg = isDemoActive() ? "デモモードでは認証処理を行いません" : "ブラウザで認証中...";
    try {
      if (!gcalConfigLoaded && !gcalConfigEdited) throw new Error("Google Calendar の設定を読み込めません");
      await gcalSaveConfig(gcalClientId.trim(), gcalClientSecret.trim());
      await gcalOpenLogin();
      if (!isDemoActive()) {
        gcalStatusColor = "var(--green)";
        gcalStatusMsg = "Google Calendar 認証成功";
        await checkGcalSession();
        await loadCalendar();
      }
    } catch (e) {
      gcalStatusColor = "var(--red)";
      gcalStatusMsg = "認証失敗: " + String(e);
    }
    setTimeout(() => { gcalStatusMsg = ""; }, 4000);
  }

  async function gcalLogout() {
    try {
      await gcalDisconnect();
      gcalStatusColor = "var(--green)";
      gcalStatusMsg = isDemoActive() ? "デモモードでは連携状態を変更しません" : "連携を解除しました";
      refreshBinding();
    } catch (e) {
      gcalStatusColor = "var(--red)";
      gcalStatusMsg = "解除失敗: " + String(e);
    }
    setTimeout(() => { gcalStatusMsg = ""; }, 4000);
  }

  async function gcalClear() {
    try {
      await gcalClearCalendar();
      const r = isDemoActive() ? "デモモードではカレンダーを変更しません" : "Google Calendar のイベントを削除しました";
      gcalStatusColor = "var(--green)";
      gcalStatusMsg = r;
      refreshBinding();
    } catch (e) {
      gcalStatusColor = "var(--red)";
      gcalStatusMsg = "削除失敗: " + String(e);
    }
    setTimeout(() => { gcalStatusMsg = ""; }, 4000);
  }

  async function gcalDeleteCal() {
    try {
      if (!isDemoActive()) {
        await invoke<string>("gcal_clear_calendar", { deleteCalendar: true });
      }
      const r = isDemoActive() ? "デモモードではカレンダーを削除しません" : "Google Calendar を削除しました";
      gcalStatusColor = "var(--green)";
      gcalStatusMsg = r;
      refreshBinding();
    } catch (e) {
      gcalStatusColor = "var(--red)";
      gcalStatusMsg = "削除失敗: " + String(e);
    }
    setTimeout(() => { gcalStatusMsg = ""; }, 4000);
  }

  export async function save() {
    saveBusy = true;
    try {
      const confirmedAccount = bindingAccount ? { ...bindingAccount } : null;
      const cc = {
        spring_start: springStart,
        fall_start: fallStart,
        syscal_enabled: false,
        syscal_auto_sync: false,
        gcal_auto_sync: gcalAutoSync === "true",
        cal_sync_interval: parseInt(calSyncInterval) || 12,
      };
      // A failed read must not turn an unrelated settings save into a reset.
      if (gcalConfigLoaded || gcalConfigEdited) {
        await gcalSaveConfig(gcalClientId.trim(), gcalClientSecret.trim());
      }
      if (isDemoActive()) {
        writeDemoCalendarConfig(cc);
      } else {
        await invoke("save_calendar_config", { config: cc, accountUsername: confirmedAccount?.username ?? null, accountGeneration: confirmedAccount?.generation ?? null, connectionId: confirmedAccount?.connectionId ?? null, calendarId: confirmedAccount?.calendarId ?? null });
      }
      localStorage.setItem("selah-gcal-auto-sync", String(cc.gcal_auto_sync));
      localStorage.setItem("selah-cal-sync-interval", String(cc.cal_sync_interval));
    } catch (e) {
      throw e;
    } finally {
      saveBusy = false;
    }
  }

  function openConsole() {
    openExternalUrl("https://console.cloud.google.com", { allowInDemo: true }).catch(() => {});
  }

  function refreshBinding() {
    accountVersion += 1;
    gcalAutoSync = "false";
    bindingAccount = null;
    void loadCalendar();
    void checkGcalSession();
  }

  onMount(async () => {
    unlistenAccount = await listen("university-auth-generation", refreshBinding);
    if (disposed) { unlistenAccount(); return; }
    unlistenLogin = await listen("gcal-login-success", () => {
      gcalStatusColor = "var(--green)";
      gcalStatusMsg = "Google Calendar 認証成功";
      refreshBinding();
      setTimeout(() => { gcalStatusMsg = ""; }, 4000);
    });
    if (disposed) { unlistenLogin(); return; }
    await loadCalendar();
    if (!disposed) await loadGcal();
  });

  onDestroy(() => {
    disposed = true;
    accountVersion += 1;
    if (unlistenLogin) unlistenLogin();
    if (unlistenAccount) unlistenAccount();
  });
</script>

<div class="hero-card">
  <div class="hero-icon" style="background:linear-gradient(135deg,rgba(52,199,89,0.15),rgba(255,150,0,0.15));">
    <svg viewBox="0 0 20 20" fill="none" stroke="#2d8a4e" stroke-width="1.3">
      <rect x="2.5" y="3.5" width="15" height="13" rx="2"/>
      <path d="M2.5 7.5h15" stroke-linecap="round"/>
      <path d="M6 2v3M14 2v3" stroke-linecap="round"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">カレンダー設定</h2>
    <p class="panel-desc">学期期間の設定と、時間割の Google Calendar への自動同期を管理します。</p>
  </div>
</div>

<div class="card-label">学期設定</div>
<div class="card">
  <div class="row">
    <span class="row-label">春学期開始日</span>
    <div class="row-input">
      <input type="date" bind:value={springStart} />
      <div class="hint">春学期の最初の授業日</div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">秋学期開始日</span>
    <div class="row-input">
      <input type="date" bind:value={fallStart} />
      <div class="hint">秋学期の最初の授業日</div>
    </div>
  </div>
</div>

<div class="card-label" style="margin-top:16px;">同期先</div>
<div class="card">
  <div class="row">
    <span class="row-label">Google Calendar</span>
    <div class="row-input">
      <div class="row-inline">
        <div class="session-indicator" style="white-space:nowrap;">
          {#if gcalState === "loading"}<span class="spinner-sm"></span>
          {:else if gcalState === "ok"}<span class="session-dot ok"></span>
          {:else if gcalState === "ng"}<span class="session-dot ng"></span>{/if}
          {gcalLabel}
        </div>
        <button class="btn-test" onclick={gcalLogin}>認証</button>
        <button class="btn-test danger" onclick={gcalLogout}>解除</button>
      </div>
    </div>
  </div>
</div>

<div class="card-label" style="margin-top:16px;">自動同期</div>
<div class="card">
  <div class="row">
    <span class="row-label">Google Calendar</span>
    <div class="row-input">
      <select bind:value={gcalAutoSync}>
        <option value="false">オフ</option>
        <option value="true">オン</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">同期間隔</span>
    <div class="row-input">
      <select bind:value={calSyncInterval}>
        <option value="6">6時間</option>
        <option value="12">12時間</option>
        <option value="24">24時間</option>
        <option value="48">48時間</option>
        <option value="72">72時間</option>
      </select>
    </div>
  </div>
  <div class="hint" style="padding:6px 14px 8px;">オンにして保存すると、現在の大学アカウント{bindingAccount?.username ? `（${bindingAccount.username}）` : ""}と Google 接続の「Selah 時間割」に自動同期します。大学アカウントや Google 接続を変更した場合は、ここで同期先を確認して再度有効にしてください。</div>
</div>

<div class="card-label" style="margin-top:16px;">データ管理</div>
<div class="card" style="padding:10px 14px;">
  <div class="btn-row">
    <button class="btn-test" onclick={gcalClear}>Google: イベント削除</button>
    <button class="btn-test danger" onclick={gcalDeleteCal}>Google: カレンダー削除</button>
    {#if gcalStatusMsg}
      <span class="hint" style="color:{gcalStatusColor};margin-left:2px;">{gcalStatusMsg}</span>
    {/if}
  </div>
</div>

<details style="margin-top:12px;">
  <summary>独自の Google Calendar API クレデンシャルを使用する（上級者向け）</summary>
  <div class="card-label" style="margin-top:8px;">Google Calendar API 設定</div>
  <div class="card">
    <div class="row">
      <span class="row-label">クライアント ID</span>
      <div class="row-input">
        <input
          type="text"
          bind:value={gcalClientId}
          oninput={() => { gcalConfigEdited = true; }}
          placeholder="xxxxx.apps.googleusercontent.com"
          spellcheck="false"
          style="font-family:monospace;font-size:11px;"
        />
      </div>
    </div>
    <div class="row">
      <span class="row-label">シークレット</span>
      <div class="row-input">
        <input
          type="password"
          bind:value={gcalClientSecret}
          oninput={() => { gcalConfigEdited = true; }}
          placeholder="GOCSPX-xxxxx (Desktop App は空欄可)"
          spellcheck="false"
          style="font-family:monospace;font-size:11px;"
        />
      </div>
    </div>
  </div>
  <div class="card-label" style="margin-top:8px;">Google Cloud Console での設定手順</div>
  <div class="card" style="padding:10px 14px;font-size:11px;color:var(--text-secondary);line-height:1.6;">
    <ol style="margin:0;padding-left:18px;">
      <li>
        <a href="https://console.cloud.google.com" style="color:var(--blue);" onclick={(e) => { e.preventDefault(); openConsole(); }}>console.cloud.google.com</a>
        で新しいプロジェクトを作成
      </li>
      <li>「API とサービス」 → 「ライブラリ」 → <strong>Google Calendar API</strong> を有効化</li>
      <li>「認証情報を作成」 → 「OAuth クライアント ID」 → 「ウェブ アプリケーション」</li>
      <li>リダイレクト URI に <code>http://127.0.0.1</code> を追加</li>
      <li>「OAuth 同意画面」でテストユーザーに自分のメールアドレスを追加</li>
    </ol>
  </div>
</details>

<style>
  .row-inline {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .btn-row {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  details summary {
    cursor: pointer;
    font-size: 11px;
    color: var(--text-secondary);
    user-select: none;
  }
  code {
    font-size: 10px;
    background: var(--bg-hover);
    padding: 1px 4px;
    border-radius: 3px;
  }
</style>
