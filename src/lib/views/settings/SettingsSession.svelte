  <script lang="ts">
  import { onMount } from "svelte";
  import { validateSession, lunaCheckSession, kwicCheckSession, syncSession, initiateRelogin, isDemoActive } from "../../api";

  type SvcState = { state: "loading" | "ok" | "ng"; label: string };

  let kg = $state<SvcState>({ state: "loading", label: "確認中..." });
  let luna = $state<SvcState>({ state: "loading", label: "確認中..." });
  let kwic = $state<SvcState>({ state: "loading", label: "確認中..." });

  let recheckBusy = $state(false);
  let reloginBusy = $state(false);
  let statusMsg = $state("");
  let statusColor = $state("");

  async function checkKg(): Promise<boolean> {
    kg = { state: "loading", label: "確認中..." };
    try {
      const s = await validateSession();
      const label = s.valid ? "有効" + (s.student_id ? ` (${s.student_id})` : "") : "無効・期限切れ";
      kg = { state: s.valid ? "ok" : "ng", label };
      return s.valid;
    } catch {
      kg = { state: "ng", label: "エラー" };
      return false;
    }
  }
  async function checkLuna(): Promise<boolean> {
    luna = { state: "loading", label: "確認中..." };
    try {
      const ok = await lunaCheckSession();
      luna = { state: ok ? "ok" : "ng", label: ok ? "有効" : "無効・未接続" };
      return ok;
    } catch {
      luna = { state: "ng", label: "エラー" };
      return false;
    }
  }
  async function checkKwic(): Promise<boolean> {
    kwic = { state: "loading", label: "確認中..." };
    try {
      const ok = await kwicCheckSession();
      kwic = { state: ok ? "ok" : "ng", label: ok ? "有効" : "無効・未接続" };
      return ok;
    } catch {
      kwic = { state: "ng", label: "エラー" };
      return false;
    }
  }

  async function recheckAll() {
    recheckBusy = true;
    try {
      const [rkg, rluna, rkwic] = await Promise.all([checkKg(), checkLuna(), checkKwic()]);
      const dead: string[] = [];
      if (!rkg) dead.push("kgc");
      if (!rluna) dead.push("luna");
      if (!rkwic) dead.push("kwic");
      if (dead.length > 0 && dead.length < 3) {
        statusColor = "var(--text-secondary)";
        statusMsg = "自動復旧中...";
        if (dead.includes("kgc")) kg = { state: "loading", label: "復旧中..." };
        if (dead.includes("luna")) luna = { state: "loading", label: "復旧中..." };
        if (dead.includes("kwic")) kwic = { state: "loading", label: "復旧中..." };
        let anyRecovered = false;
        await Promise.all(dead.map(async (svc) => {
          try {
            const ok = await syncSession(svc);
            if (ok) anyRecovered = true;
          } catch { /* ignore */ }
        }));
        await Promise.all([checkKg(), checkLuna(), checkKwic()]);
        if (anyRecovered) {
          statusColor = "var(--green)";
          statusMsg = "自動復旧完了";
        } else {
          statusColor = "var(--orange, #ff9500)";
          statusMsg = "復旧失敗 - 再ログインが必要です";
        }
        setTimeout(() => { statusMsg = ""; }, 5000);
      }
    } finally {
      recheckBusy = false;
    }
  }

  async function relogin() {
    reloginBusy = true;
    statusColor = "var(--text-secondary)";
    statusMsg = "再ログイン中...";
    try {
      if (isDemoActive()) {
        await initiateRelogin();
        statusColor = "var(--green)";
        statusMsg = "演示モードの再認証状態を更新しました";
      } else {
        const ok = await syncSession("all");
        if (ok) {
          statusColor = "var(--green)";
          statusMsg = "再ログイン成功";
        } else {
          statusColor = "var(--red)";
          statusMsg = "Okta SSO 期限切れ - アプリ内で再ログインしてください";
        }
      }
      await Promise.all([checkKg(), checkLuna(), checkKwic()]);
    } catch (e) {
      statusColor = "var(--red)";
      statusMsg = "失敗: " + String(e);
    } finally {
      reloginBusy = false;
      setTimeout(() => { statusMsg = ""; }, 5000);
    }
  }

  onMount(() => {
    void Promise.all([checkKg(), checkLuna(), checkKwic()]);
  });
</script>

<div class="hero-card">
  <div class="hero-icon" style="background:linear-gradient(135deg,rgba(52,199,89,0.15),rgba(0,122,255,0.15));">
    <svg viewBox="0 0 20 20" fill="none" stroke="#2d8a4e" stroke-width="1.3">
      <rect x="3" y="5" width="14" height="10" rx="2"/>
      <path d="M7 5V3.5a3 3 0 016 0V5" stroke-linecap="round"/>
      <circle cx="10" cy="10.5" r="1.5"/>
      <path d="M10 12v1.5" stroke-linecap="round"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">セッション</h2>
    <p class="panel-desc">各サービスへの認証セッション状態を確認できます。セッションが切れている場合は、再ログインで復旧します。</p>
  </div>
</div>

<div class="card-label">セッション状態</div>
<div class="card">
  <div class="row">
    <span class="row-label">KG Course</span>
    <div class="row-input">
      <div class="session-indicator">
        {#if kg.state === "loading"}<span class="spinner-sm"></span>
        {:else if kg.state === "ok"}<span class="session-dot ok"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {kg.label}
      </div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna LMS</span>
    <div class="row-input">
      <div class="session-indicator">
        {#if luna.state === "loading"}<span class="spinner-sm"></span>
        {:else if luna.state === "ok"}<span class="session-dot ok"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {luna.label}
      </div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">KWIC Portal</span>
    <div class="row-input">
      <div class="session-indicator">
        {#if kwic.state === "loading"}<span class="spinner-sm"></span>
        {:else if kwic.state === "ok"}<span class="session-dot ok"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {kwic.label}
      </div>
    </div>
  </div>
</div>

<div class="action-bar">
  <button class="btn-test" disabled={recheckBusy} onclick={recheckAll}>再検証</button>
  <button class="btn-test warn" disabled={reloginBusy} onclick={relogin}>再ログイン</button>
  {#if statusMsg}
    <span class="hint" style="color:{statusColor};margin-left:4px;">{statusMsg}</span>
  {/if}
</div>

<style>
  .action-bar {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 4px;
  }
  :global(.settings-main .btn-test.warn) {
    background: rgba(255, 150, 0, 0.12);
    color: var(--orange, #ff9500);
    border-color: rgba(255, 150, 0, 0.3);
  }
  :global(.settings-main .btn-test.warn:hover:not(:disabled)) {
    background: var(--orange, #ff9500);
    color: #fff;
    border-color: var(--orange, #ff9500);
  }
</style>
