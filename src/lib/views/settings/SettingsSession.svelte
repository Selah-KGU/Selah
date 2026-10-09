<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    isDemoActive,
    getSavedCookieSummaries,
    getStoredSessionStates,
    lunaCheckSession,
    kwicCheckSession,
    resetUniversityLogin,
    syncSession,
    validateSession,
    type SavedCookieSummary,
  } from "../../api";

  import { serviceVerified, type RecoveryReport, type ServiceStatus } from "../../universitySession";

  type SvcState = { state: "loading" | "saved" | "ok" | "ng" | "error"; label: string };
  type ServiceKey = "kgc" | "luna" | "kwic";

  let kg = $state<SvcState>({ state: "loading", label: "確認中..." });
  let luna = $state<SvcState>({ state: "loading", label: "確認中..." });
  let kwic = $state<SvcState>({ state: "loading", label: "確認中..." });

  let checkBusy = $state(false);
  let repairBusy = $state(false);
  let kgcRefreshBusy = $state(false);
  let resetBusy = $state(false);
  let repairResults = $state<RecoveryReport["results"]>([]);
  let repairUnavailable = $state(false);
  let resetArmed = $state(false);
  let statusMsg = $state("");
  let statusColor = $state("");
  let cookieSummaries = $state<SavedCookieSummary[]>([]);
  let cookieSummaryError = $state(false);
  type SecretStoreStatus = { backend: string; state: string; last_error: string | null };
  let secretStatus = $state<SecretStoreStatus | null>(null);
  let secretRetryBusy = $state(false);
  let loginPersistencePending = $state(false);
  type SecretRecovery = { status: SecretStoreStatus; results: { component: string; outcome: string; error: { kind: string; message: string } | null }[] };
  let secretRecoveryErrors = $state<string[]>([]);
  type ServiceHealth = ServiceStatus;
  let health = $state<ServiceHealth[]>([]);
  let ssoEvidence = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | null = null;

  let disconnectedCore = $derived([
    ...(luna.state === "ng" ? ["luna" as const] : []),
    ...(kwic.state === "ng" ? ["kwic" as const] : []),
  ]);
  let coreReady = $derived(luna.state === "ok" && kwic.state === "ok");
  let coreStored = $derived(
    (luna.state === "ok" || luna.state === "saved")
      && (kwic.state === "ok" || kwic.state === "saved"),
  );
  let checking = $derived(checkBusy || repairBusy || kgcRefreshBusy || resetBusy);

  function clearStatusLater() {
    setTimeout(() => { statusMsg = ""; }, 5000);
  }

  async function checkKg(): Promise<boolean | null> {
    kg = { state: "loading", label: "確認中..." };
    try {
      const s = await validateSession();
      const label = s.valid ? "有効" + (s.student_id ? ` (${s.student_id})` : "") : "無効・期限切れ";
      kg = { state: s.valid ? "ok" : "ng", label };
      return s.valid;
    } catch {
      kg = { state: "error", label: "確認できません" };
      return null;
    }
  }

  async function checkLuna(): Promise<boolean | null> {
    luna = { state: "loading", label: "確認中..." };
    try {
      const result = await lunaCheckSession();
      const ok = result.state === "valid";
      luna = { state: ok ? "ok" : "ng", label: ok ? "有効" : "無効・未接続" };
      return ok;
    } catch {
      luna = { state: "error", label: "確認できません" };
      return null;
    }
  }

  async function checkKwic(): Promise<boolean | null> {
    kwic = { state: "loading", label: "確認中..." };
    try {
      const result = await kwicCheckSession();
      const ok = result.state === "valid";
      kwic = { state: ok ? "ok" : "ng", label: ok ? "有効" : "無効・未接続" };
      return ok;
    } catch {
      kwic = { state: "error", label: "確認できません" };
      return null;
    }
  }

  async function checkAll(showResult = true) {
    checkBusy = true;
    try {
      const results = await Promise.all([checkKg(), checkLuna(), checkKwic()]);
      if (!showResult) return;
      if (results.some(result => result === null)) {
        statusColor = "var(--orange, #ff9500)";
        statusMsg = "一部の状態を確認できませんでした。接続は変更していません。";
      } else if (results[1] && results[2]) {
        statusColor = "var(--green)";
        statusMsg = results[0]
          ? "主要サービスと KG Course は利用可能です"
          : "主要サービスは利用可能です。KG Course のみ未接続です";
      } else {
        statusColor = "var(--red)";
        statusMsg = "主要サービスの再認証が必要です";
      }
      clearStatusLater();
    } finally {
      checkBusy = false;
      await loadCookieSummaries();
    }
  }

  async function loadStoredStates() {
    try {
      const states = await getStoredSessionStates();
      ssoEvidence = states.sso === true;
      kg = states.kgc
        ? { state: "saved", label: "保存済み・未確認" }
        : { state: "ng", label: "未接続" };
      luna = states.luna
        ? { state: "saved", label: "保存済み・未確認" }
        : { state: "ng", label: "未接続" };
      kwic = states.kwic
        ? { state: "saved", label: "保存済み・未確認" }
        : { state: "ng", label: "未接続" };
    } catch {
      kg = { state: "error", label: "状態を読み込めません" };
      luna = { state: "error", label: "状態を読み込めません" };
      kwic = { state: "error", label: "状態を読み込めません" };
    }
  }

  async function loadCookieSummaries() {
    cookieSummaryError = false;
    try {
      cookieSummaries = await getSavedCookieSummaries();
    } catch {
      cookieSummaries = [];
      cookieSummaryError = true;
    }
    await loadSecretStatus();
    await loadSessionHealth();
  }

  async function loadSessionHealth() {
    if (isDemoActive()) return;
    try {
      const result = await invoke<{ services: ServiceHealth[]; login_persistence_pending?: boolean }>("get_session_diagnostics");
      health = result.services;
      loginPersistencePending = result.login_persistence_pending === true;
      ssoEvidence = (await getStoredSessionStates()).sso === true;
      for (const service of health) {
        let next: SvcState | null = null;
        if (service.state === "unverified") next = { state: service.credentials_present ? "saved" : "ng", label: service.credentials_present ? "保存済み・未確認" : "未接続" };
        if (service.state === "valid") next = { state: "ok", label: "有効・確認済み" };
        if (service.state === "refreshing") next = { state: "loading", label: "復旧中..." };
        if (service.state === "unavailable") next = { state: "error", label: "通信状態により確認できません" };
        if (service.state === "needs_login") next = { state: "ng", label: "再認証が必要です" };
        if (service.state === "signed_out") next = { state: "ng", label: "サインアウト済み" };
        if (!next) continue;
        if (service.service === "kgc") kg = next;
        if (service.service === "luna") luna = next;
        if (service.service === "kwic") kwic = next;
      }
    } catch { health = []; }
  }

  async function loadSecretStatus() {
    if (isDemoActive()) return;
    try { secretStatus = await invoke<SecretStoreStatus>("get_secret_store_status"); }
    catch { secretStatus = { backend: "", state: "unavailable", last_error: null }; }
  }

  async function retrySecretStore() {
    secretRetryBusy = true;
    try {
      const report = await invoke<SecretRecovery>("retry_secret_store");
      secretStatus = report.status;
      secretRecoveryErrors = report.results.filter(item => item.error).map(item => `${secretComponentLabel(item.component)}: ${item.error!.message}`);
      await loadStoredStates();
      await loadCookieSummaries();
    } catch (error) {
      secretRecoveryErrors = [String(error)];
      await loadSecretStatus();
    } finally { secretRetryBusy = false; }
  }

  function secretComponentLabel(component: string): string {
    const labels: Record<string, string> = { kgc: "KGC", luna: "Luna", kwic: "KWIC", session_checkpoint: "大学ログイン情報の保存", sso_cookies: "SSO Cookieの復元", mail: "Microsoft 365 メール", gcal: "Google カレンダー", gcal_config: "Google カレンダー設定" };
    return labels[component] ?? component;
  }

  function storageLabel(state: string): string {
    if (state === "ready") return "保存先は利用可能です";
    if (state === "erased") return "認証情報は削除されました。アプリを再起動してください。";
    if (state === "locked") return "資格情報ストアを開けません。ロック解除後に再試行してください。";
    if (state === "write_failed") return "最新の認証情報を保存できていません。アプリを終了する前に再試行してください。";
    if (state === "corrupt") return "保存データを検証できませんでした。元のデータは保持しています。";
    if (state === "cleanup_failed") return "保存先の切り替え後、旧データの削除が完了していません。再試行してください。";
    return "保存先の状態を確認できません。再試行してください。";
  }

  function cookieSummary(service: ServiceKey): SavedCookieSummary | undefined {
    return cookieSummaries.find(summary => summary.service === service);
  }

  function formatDateTime(timestamp: number | null): string {
    if (!timestamp) return "記録なし";
    return new Intl.DateTimeFormat("ja-JP", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(timestamp * 1000));
  }

  function expiryLabel(summary: SavedCookieSummary): string {
    if (summary.active_cookie_count === 0) return "保存内容なし";
    if (summary.earliest_expiry_at) return formatDateTime(summary.earliest_expiry_at);
    return "固定期限なし";
  }

  function setRepairing(service: ServiceKey) {
    if (service === "kgc") kg = { state: "loading", label: "復旧中..." };
    if (service === "luna") luna = { state: "loading", label: "復旧中..." };
    if (service === "kwic") kwic = { state: "loading", label: "復旧中..." };
  }

  async function repair(service: ServiceKey): Promise<boolean> {
    setRepairing(service);
    try {
      const report = await syncSession(service);
      repairResults.push(...report.results);
      await loadSessionHealth();
      const result = report.results.find(result => result.service === service);
      if (result?.outcome === "deferred") {
        statusMsg = "前回の試行後の待機中です。少し待ってから再試行してください。";
      } else if (result?.outcome === "unavailable") {
        statusMsg = "通信または保存先を確認できません。接続情報は保持しています。";
      }
      return serviceVerified(report, service);
    } catch {
      repairUnavailable = true;
      await loadSessionHealth();
      return false;
    }
  }

  function recoveryFailureMessage() {
    if (repairUnavailable || repairResults.some(result => result.outcome === "unavailable")) return "通信または保存先を確認できません。接続情報は保持しています。";
    if (repairResults.some(result => result.outcome === "deferred")) return "前回の試行後の待機中です。少し待ってから再試行してください。";
    return "再認証が必要です。完全再ログインを試してください。";
  }

  async function repairDisconnected() {
    if (disconnectedCore.length === 0) {
      await checkAll();
      return;
    }

    repairBusy = true;
    repairResults = []; repairUnavailable = false;
    statusColor = "var(--text-secondary)";
    statusMsg = "Luna・KWIC の接続を復旧しています...";
    const targets = [...disconnectedCore];
    try {
      for (const service of targets) {
        await repair(service);
      }
      if (luna.state === "ok" && kwic.state === "ok") {
        statusColor = "var(--green)";
        statusMsg = kg.state === "ok"
          ? "すべての接続を復旧しました"
          : "主要サービスを復旧しました。KG Course は引き続き未接続です";
      } else {
        statusColor = "var(--orange, #ff9500)";
        statusMsg = recoveryFailureMessage();
      }
    } finally {
      repairBusy = false;
      await loadCookieSummaries();
      clearStatusLater();
    }
  }

  async function refreshKgcAndCore() {
    kgcRefreshBusy = true;
    repairResults = []; repairUnavailable = false;
    statusColor = "var(--text-secondary)";
    statusMsg = "KG Course を更新しています...";
    setRepairing("kgc");
    try {
      const kgcOk = await repair("kgc");
      if (!kgcOk) {
        statusColor = "var(--orange, #ff9500)";
        statusMsg = recoveryFailureMessage();
        return;
      }

      statusMsg = "KG Course を更新しました。Luna・KWIC の Cookie を更新しています...";
      setRepairing("luna");
      setRepairing("kwic");
      const lunaOk = await repair("luna");
      const kwicOk = await repair("kwic");

      if (lunaOk && kwicOk) {
        statusColor = "var(--green)";
        statusMsg = "KG Course・Luna・KWIC の Cookie を更新しました";
      } else {
        statusColor = "var(--orange, #ff9500)";
        statusMsg = recoveryFailureMessage();
      }
    } finally {
      kgcRefreshBusy = false;
      await loadCookieSummaries();
      clearStatusLater();
    }
  }

  function armReset() {
    resetArmed = true;
    statusColor = "var(--red)";
    statusMsg = "Luna・KWIC・KG Course の認証 Cookie を削除します。設定や保存データは削除しません。";
    if (resetTimer) clearTimeout(resetTimer);
    resetTimer = setTimeout(() => {
      resetArmed = false;
      statusMsg = "";
    }, 7000);
  }

  async function clearCookiesAndRelogin() {
    if (!resetArmed) {
      armReset();
      return;
    }

    resetArmed = false;
    if (resetTimer) clearTimeout(resetTimer);
    resetBusy = true;
    statusColor = "var(--text-secondary)";
    statusMsg = "認証 Cookie を削除してログイン画面を開いています...";
    try {
      const result = await resetUniversityLogin();
      if (isDemoActive()) {
        statusColor = "var(--green)";
        statusMsg = "デモモードの認証状態を初期化しました";
      } else {
        kg = result.core
          ? { state: "ok", label: "有効・再ログイン済み" }
          : { state: "ng", label: "再ログインできませんでした" };
        luna = result.core?.luna_authenticated
          ? { state: "ok", label: "有効・再ログイン済み" }
          : { state: "ng", label: "再ログインできませんでした" };
        kwic = result.core?.kwic_authenticated
          ? { state: "ok", label: "有効・再ログイン済み" }
          : { state: "ng", label: "再ログインできませんでした" };
        statusColor = luna.state === "ok" && kwic.state === "ok"
          ? "var(--green)"
          : "var(--orange, #ff9500)";
        statusMsg = luna.state === "ok" && kwic.state === "ok"
          ? `${result.deleted} 件の Cookie を削除し、完全再ログインしました`
          : `${result.deleted} 件の Cookie を削除しましたが、一部の主要サービスに再ログインできませんでした`;
      }
    } catch (e) {
      statusColor = "var(--red)";
      statusMsg = "失敗: " + String(e);
    } finally {
      resetBusy = false;
      await loadCookieSummaries();
      clearStatusLater();
    }
  }

  let secretStore = $state("keychain");
  let secStatusMsg = $state("");
  let secStatusColor = $state("");

  async function loadSecurityConfig() {
    if (isDemoActive()) return;
    try {
      const cfg = await invoke<{ secret_store?: string }>("get_security_config");
      secretStore = cfg.secret_store === "file" ? "file" : "keychain";
    } catch (e) {
      console.error("Failed to load security config:", e);
    }
  }

  async function saveSecurityConfig() {
    if (isDemoActive()) return;
    try {
      await invoke("save_security_config", { config: { secret_store: secretStore } });
      secStatusColor = "var(--green)";
      secStatusMsg = "保存しました";
    } catch (e) {
      secStatusColor = "var(--red)";
      secStatusMsg = "保存失敗: " + String(e);
    }
    await loadSecurityConfig();
    await loadSecretStatus();
    setTimeout(() => { secStatusMsg = ""; }, 4000);
  }

  onMount(() => {
    void (async () => {
      await loadStoredStates();
      await loadCookieSummaries();
      await checkAll(false);
    })();
    void loadSecurityConfig();
  });

  onDestroy(() => {
    if (resetTimer) clearTimeout(resetTimer);
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
    <p class="panel-desc">Luna と KWIC を主要サービスとして監視します。KG Course は必要なデータを取得するときに復旧を試みます。通信エラーと再認証が必要な状態を区別します。</p>
  </div>
</div>

<div class="card-label">セッション状態</div>
<div class="card">
  <div class="row">
    <span class="row-label">KG Course <span class="service-role auxiliary">補助・短期</span></span>
    <div class="row-input">
      <div class="session-indicator">
        {#if kg.state === "loading"}<span class="spinner-sm"></span>
        {:else if kg.state === "ok"}<span class="session-dot ok"></span>
        {:else if kg.state === "saved"}<span class="session-dot saved"></span>
        {:else if kg.state === "error"}<span class="session-dot unknown"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {kg.label}
      </div>
      <button class="btn-test session-update" disabled={checking} onclick={refreshKgcAndCore}>
        {kgcRefreshBusy ? "更新中..." : "KGCを更新"}
      </button>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna LMS <span class="service-role core">主要</span></span>
    <div class="row-input">
      <div class="session-indicator">
        {#if luna.state === "loading"}<span class="spinner-sm"></span>
        {:else if luna.state === "ok"}<span class="session-dot ok"></span>
        {:else if luna.state === "saved"}<span class="session-dot saved"></span>
        {:else if luna.state === "error"}<span class="session-dot unknown"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {luna.label}
      </div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">KWIC Portal <span class="service-role core">主要</span></span>
    <div class="row-input">
      <div class="session-indicator">
        {#if kwic.state === "loading"}<span class="spinner-sm"></span>
        {:else if kwic.state === "ok"}<span class="session-dot ok"></span>
        {:else if kwic.state === "saved"}<span class="session-dot saved"></span>
        {:else if kwic.state === "error"}<span class="session-dot unknown"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {kwic.label}
      </div>
    </div>
  </div>
</div>

<div class="card-label">保存済み Cookie の概要</div>
<div class="card cookie-summary-card">
  <div class="hint">SSO 復旧用データ: {ssoEvidence ? "あり（有効性は復旧時に確認）" : "未確認・なし"}</div>
  {#each [
    { key: "kgc" as const, label: "KG Course" },
    { key: "luna" as const, label: "Luna LMS" },
    { key: "kwic" as const, label: "KWIC Portal" },
  ] as service}
    {@const summary = cookieSummary(service.key)}
    {@const verified = health.find(item => item.service === service.key)?.last_verified_at}
    <div class="cookie-summary-row">
      <strong class="cookie-service-name">{service.label}</strong>
      <div class="cookie-summary-detail">
        {#if summary?.saved}
          <span>{formatDateTime(summary.saved_at)}</span>
          <span>{summary.active_cookie_count} 件{summary.session_cookie_count ? `（セッション型 ${summary.session_cookie_count} 件）` : ""}</span>
          <span>最短 Cookie 期限: {expiryLabel(summary)}（ログイン期限とは異なります）</span>
        {:else}
          <span>保存内容なし</span>
        {/if}
        {#if verified}<span>最終確認: {formatDateTime(verified)}</span>{/if}
      </div>
      <span class:saved={summary?.saved} class="cookie-save-state">
        {summary?.saved ? "保存済み" : "未保存"}
      </span>
    </div>
  {/each}
  {#if cookieSummaryError}
    <div class="hint cookie-summary-error">Cookie の概要を読み込めませんでした。</div>
  {:else}
    <div class="hint cookie-summary-note">最後に保存できた Cookie の概要です。保存済みでも、大学側のセッションが有効とは限りません。認証情報の内容は表示しません。</div>
  {/if}
</div>

<div class="card-label">秘密情報の保存先</div>
<div class="card">
  <div class="row">
    <span class="row-label">保存先</span>
    <div class="row-input sec-store">
      <select bind:value={secretStore} onchange={saveSecurityConfig}>
        <option value="keychain">OSで鍵を保護（推奨）</option>
        <option value="file">端末情報で暗号化</option>
      </select>
      <div class="hint">APIキー・トークン・Cookie は暗号化して保存します。推奨設定では、ランダムな暗号鍵を OS の資格情報ストアで保護します。「端末情報で暗号化」はキーチェーンを使用しませんが、同一ユーザーのプロセスから復号され得ます。切り替えは移行先への保存が完了してから適用します。</div>
      {#if secretStatus}
        <div class="hint" role="status" style:color={secretStatus.state === "ready" ? "var(--green)" : "var(--red)"}>
          {storageLabel(secretStatus.state)}
        </div>
        {#if loginPersistencePending}
          <div class="hint" role="status">ログイン済みですが、認証情報の保存は未完了です。再試行すると保存とログインの確定を続行します。</div>
        {/if}
        {#each secretRecoveryErrors as error}
          <div class="hint" role="status" style:color="var(--red)">{error}</div>
        {/each}
        {#if secretStatus.state !== "ready" || secretRecoveryErrors.length > 0 || loginPersistencePending}
          <button class="btn-test" disabled={secretRetryBusy} onclick={retrySecretStore}>
            {secretRetryBusy ? "再試行中..." : "保存先を再確認"}
          </button>
        {/if}
      {/if}
      {#if secStatusMsg}
        <div class="hint" style="color:{secStatusColor};">{secStatusMsg}</div>
      {/if}
    </div>
  </div>
</div>

<div class:ready={coreReady} class:stored={!coreReady && coreStored} class="core-summary">
  <strong>{coreReady ? "主要サービスは利用可能です" : coreStored ? "主要サービスのセッションは保存されています" : "主要サービスの状態を確認してください"}</strong>
  <span>{coreReady ? "KG Course が切れていても通常利用を継続できます。" : coreStored ? "保存済みの状態を確認しています。" : "Luna または KWIC が未接続の場合は、主要サービスの復旧を試してください。"}</span>
</div>

<div class="action-bar">
  <button class="btn-test" disabled={checking} onclick={() => checkAll()}>状態を確認</button>
  <button class="btn-test primary" disabled={checking || disconnectedCore.length === 0} onclick={repairDisconnected}>
    {repairBusy ? "Luna・KWICを復旧中..." : `Luna・KWICを復旧${disconnectedCore.length ? ` (${disconnectedCore.length})` : ""}`}
  </button>
  <button class:armed={resetArmed} class="btn-test danger" disabled={checking} onclick={clearCookiesAndRelogin}>
    {resetBusy ? "初期化中..." : resetArmed ? "もう一度押して完全再ログイン" : "Cookieを削除して完全再ログイン"}
  </button>
  {#if statusMsg}
    <span class="hint action-status" style="color:{statusColor};">{statusMsg}</span>
  {/if}
</div>

<style>
  .service-role {
    display: inline-flex;
    align-items: center;
    margin-left: 6px;
    padding: 2px 6px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 650;
    vertical-align: 2px;
  }
  .cookie-summary-card {
    overflow: hidden;
  }
  .cookie-summary-row {
    display: grid;
    grid-template-columns: minmax(110px, 0.7fr) minmax(0, 2fr) auto;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .cookie-service-name {
    font-size: 12px;
  }
  .cookie-summary-detail {
    display: flex;
    flex-wrap: wrap;
    gap: 3px 12px;
    min-width: 0;
    color: var(--text-secondary);
    font-size: 10.5px;
  }
  .cookie-summary-detail span {
    white-space: nowrap;
  }
  .cookie-save-state {
    justify-self: end;
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 650;
    white-space: nowrap;
  }
  .cookie-save-state.saved {
    color: var(--green, #34c759);
  }
  .cookie-summary-note,
  .cookie-summary-error {
    display: block;
    padding: 10px 14px;
  }
  .cookie-summary-error {
    color: var(--orange, #ff9500);
  }
  @media (max-width: 700px) {
    .cookie-summary-row {
      grid-template-columns: 1fr auto;
    }
    .cookie-summary-detail {
      grid-column: 1 / -1;
      grid-row: 2;
    }
  }
  .service-role.core {
    color: var(--green, #34c759);
    background: color-mix(in srgb, var(--green, #34c759) 12%, transparent);
  }
  .service-role.auxiliary {
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--text-secondary) 10%, transparent);
  }
  .core-summary {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-top: 10px;
    padding: 11px 14px;
    border: 1px solid color-mix(in srgb, var(--orange, #ff9500) 25%, transparent);
    border-radius: 12px;
    background: color-mix(in srgb, var(--orange, #ff9500) 7%, transparent);
    color: var(--text-secondary);
    font-size: 11px;
  }
  .core-summary strong {
    color: var(--orange, #ff9500);
    font-size: 12px;
  }
  .core-summary.ready {
    border-color: color-mix(in srgb, var(--green, #34c759) 24%, transparent);
    background: color-mix(in srgb, var(--green, #34c759) 7%, transparent);
  }
  .core-summary.ready strong {
    color: var(--green, #34c759);
  }
  .core-summary.stored {
    border-color: color-mix(in srgb, var(--accent) 24%, transparent);
    background: color-mix(in srgb, var(--accent) 7%, transparent);
  }
  .core-summary.stored strong {
    color: var(--accent);
  }
  .action-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
    align-items: center;
    margin-top: 10px;
  }
  .action-status {
    flex-basis: 100%;
    margin-top: 2px;
  }
  .row-input {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  /* Secret-store row stacks select + hint + status vertically (the default
     .row-input here is a horizontal flex used by the status rows). */
  .sec-store {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
  }
  .session-indicator {
    flex: 1;
    min-width: 0;
  }
  .session-update {
    flex: 0 0 auto;
    min-width: 88px;
  }
  :global(.settings-main .btn-test.primary) {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 28%, transparent);
  }
  :global(.settings-main .btn-test.primary:hover:not(:disabled)) {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  :global(.settings-main .btn-test.danger) {
    background: color-mix(in srgb, var(--red) 8%, transparent);
    color: var(--red);
    border-color: color-mix(in srgb, var(--red) 24%, transparent);
  }
  :global(.settings-main .btn-test.danger:hover:not(:disabled)),
  :global(.settings-main .btn-test.danger.armed) {
    background: var(--red);
    color: #fff;
    border-color: var(--red);
  }
  :global(.settings-main .session-dot.unknown) {
    background: var(--orange, #ff9500);
  }
  :global(.settings-main .session-dot.saved) {
    background: var(--accent);
    opacity: 0.65;
  }
</style>
