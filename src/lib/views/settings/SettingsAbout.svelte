<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { devModeActive } from "../../stores";
  import { logout, isDemoActive } from "../../api";
  import { openExternalUrl } from "../../system";
  import {
    appUpdateState,
    checkForAppUpdate,
    distributionChannel,
    downloadAndInstallAppUpdate,
    updaterManagedByStore,
  } from "../../updater";
  import { get } from "svelte/store";
  import logoUrl from "../../../assets/logo.png";

  const RELEASES_URL = "https://github.com/Selah-KGU/Selah/releases/latest";

  let version = $state("...");
  let deleteConfirmOpen = $state(false);
  let deleteErr = $state("");

  let tapCount = 0;
  let tapTimer: ReturnType<typeof setTimeout> | null = null;
  let shakeKey = $state(0);

  async function loadVersion() {
    try {
      const info = await invoke<{ app_version?: string }>("debug_info");
      if (info.app_version) version = info.app_version;
    } catch (e) {
      console.error("version fetch failed:", e);
    }
  }

  function handleVersionTap() {
    if (get(devModeActive)) return;
    tapCount++;
    if (tapTimer) clearTimeout(tapTimer);
    const remaining = 7 - tapCount;
    if (remaining > 0 && remaining <= 3) {
      shakeKey++;
    }
    if (tapCount >= 7) {
      devModeActive.set(true);
      tapCount = 0;
    }
    tapTimer = setTimeout(() => { tapCount = 0; }, 1500);
  }

  async function openUrl(url: string) {
    try { await openExternalUrl(url, { allowInDemo: true }); } catch (e) { console.error(e); }
  }

  async function logoutAndReturn() {
    try {
      await logout();
      location.reload();
    } catch (e) {
      console.error("logout failed:", e);
    }
  }

  function formatBytes(bytes: number | null): string {
    if (bytes == null || bytes <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    let value = bytes;
    let index = 0;
    while (value >= 1024 && index < units.length - 1) {
      value /= 1024;
      index++;
    }
    return `${value >= 100 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
  }

  function progressSummary(): string {
    if ($appUpdateState.phase === "installing") {
      return "ダウンロードは完了しました。更新を適用しています。";
    }
    if ($appUpdateState.totalBytes && $appUpdateState.totalBytes > 0) {
      return `${formatBytes($appUpdateState.downloadedBytes)} / ${formatBytes($appUpdateState.totalBytes)}`;
    }
    return `${formatBytes($appUpdateState.downloadedBytes)} を受信しました`;
  }

  function updateProviderLabel(): string {
    if (distributionChannel === "appstore") return "Mac App Store";
    if (distributionChannel === "msstore") return "Microsoft Store";
    return "GitHub Releases";
  }

  function startDelete() {
    deleteErr = "";
    deleteConfirmOpen = true;
  }

  async function confirmDelete() {
    if (isDemoActive()) {
      deleteErr = "演示モードではローカルデータ削除は実行しません。";
      return;
    }
    try {
      await invoke("delete_all_local_data");
      try { localStorage.clear(); } catch { /* noop */ }
      location.reload();
    } catch (e) {
      deleteErr = "データの削除に失敗しました: " + String(e);
    }
  }

  onMount(() => {
    void loadVersion();
  });
</script>

<div class="card">
  <div class="about-hero">
    <button type="button" class="about-icon-btn" onclick={handleVersionTap} aria-label="開発モードを有効化">
      <img
        class="about-icon"
        src={logoUrl}
        alt="Selah"
        draggable="false"
      />
    </button>
    <div class="about-name">Selah</div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    {#key shakeKey}
      <div
        class={`about-version ${shakeKey > 0 ? `shaking-${shakeKey % 2}` : ""}`}
        onclick={handleVersionTap}
      >バージョン {version}</div>
    {/key}
    <div class="about-tagline">新月の下で、知性を繋ぐ。すべての関学生に。</div>
    <div class="about-sep"></div>
  </div>
  <div>
    <div class="about-row"><span class="al">開発者</span><span class="ar">Selah-KGU</span></div>
    <div class="about-row"><span class="al">対応大学</span><span class="ar">関西学院大学</span></div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="about-row clickable" onclick={() => openUrl("https://github.com/Selah-KGU/Selah")}>
      <span class="al">ソースコード</span>
      <span class="ar" style="color:var(--accent)">GitHub<span style="color:var(--text-tertiary);font-weight:400;margin-left:4px">›</span></span>
    </div>
    <div class="about-row"><span class="al">ライセンス</span><span class="ar">PolyForm Noncommercial 1.0.0</span></div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="about-row clickable" onclick={() => openUrl("https://github.com/Selah-KGU/Selah/blob/main/THIRD_PARTY_NOTICES.md")}>
      <span class="al">第三者ライセンス</span>
      <span class="ar" style="color:var(--accent)">Notices<span style="color:var(--text-tertiary);font-weight:400;margin-left:4px">›</span></span>
    </div>
  </div>
  <div class="about-copy">
    © 2026 Selah-KGU. All rights reserved.<br />
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="link" onclick={() => openUrl("https://selah.jp/terms.html")}>利用規約</span>
    <span style="margin:0 4px;color:var(--text-tertiary)">|</span>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="link" onclick={() => openUrl("https://selah.jp/privacy.html")}>プライバシーポリシー</span>
  </div>
</div>

<div class="card-label" style="margin-top:10px;">アプリ更新</div>
<div class="card">
  <div class="row">
    <span class="row-label">現在</span>
    <div class="row-input">
      <div class="update-inline">
        <div class="update-current">{version}</div>
        {#if !updaterManagedByStore}
          <button
            class="btn-test"
            onclick={() => void checkForAppUpdate()}
            disabled={$appUpdateState.checking || $appUpdateState.phase === "downloading" || $appUpdateState.phase === "installing"}
          >
            {$appUpdateState.checking ? "確認中..." : "更新を確認"}
          </button>
        {/if}
      </div>
      <div class="hint update-status">{$appUpdateState.status}</div>
    </div>
  </div>

  {#if updaterManagedByStore}
    <div class="row">
      <span class="row-label">配信元</span>
      <div class="row-input">
        <div class="hint update-manual-hint">{updateProviderLabel()} から更新されます。</div>
      </div>
    </div>
  {:else if $appUpdateState.phase === "downloading" || $appUpdateState.phase === "installing"}
    <div class="row">
      <span class="row-label">進行状況</span>
      <div class="row-input">
        <div class="update-progress-meta">
          <span>{$appUpdateState.progressPercent != null ? `${$appUpdateState.progressPercent}%` : "取得中"}</span>
          <span>{progressSummary()}</span>
        </div>
        <div class="update-progress-track">
          <div
            class="update-progress-fill"
            class:indeterminate={$appUpdateState.progressPercent == null && $appUpdateState.phase === "downloading"}
            style={`width: ${$appUpdateState.progressPercent != null ? $appUpdateState.progressPercent : 34}%`}
          ></div>
        </div>
      </div>
    </div>
  {/if}

  {#if $appUpdateState.available}
    <div class="row">
      <span class="row-label">新しい版</span>
      <div class="row-input">
        <div class="update-version">{$appUpdateState.version}</div>
        {#if $appUpdateState.notes}
          <pre class="update-notes">{$appUpdateState.notes}</pre>
        {/if}
        <div class="update-actions">
          <button
            class="btn-test"
            onclick={() => void downloadAndInstallAppUpdate()}
            disabled={$appUpdateState.checking || $appUpdateState.phase === "downloading" || $appUpdateState.phase === "installing"}
          >ダウンロードして更新</button>
          <button
            class="btn-test"
            onclick={() => openUrl(RELEASES_URL)}
            disabled={$appUpdateState.checking || $appUpdateState.phase === "downloading" || $appUpdateState.phase === "installing"}
          >Releases を開く</button>
        </div>
      </div>
    </div>
  {:else}
    <div class="row">
      <span class="row-label">手動更新</span>
      <div class="row-input">
        <div class="update-inline">
          <div class="hint update-manual-hint">{updateProviderLabel()} から最新版を取得できます。</div>
          <button class="btn-test" onclick={() => openUrl(RELEASES_URL)}>Releases を開く</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<div class="card" style="margin-top:10px;padding:0;">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="about-row clickable" style="justify-content:center;" onclick={logoutAndReturn}>
    <span style="color:var(--red);font-weight:500;">ログアウトしてログイン画面へ戻る</span>
  </div>
</div>

{#if !deleteConfirmOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="card" style="margin-top:10px;padding:0;cursor:pointer;" onclick={startDelete}>
    <div class="about-row" style="justify-content:center;pointer-events:none;">
      <span style="color:var(--red);font-weight:500;">すべてのローカルデータを削除</span>
    </div>
    <div class="delete-warn">
      データベース、設定、Cookie、キーチェーンの秘密情報など、アプリが保存したすべてのデータを完全に削除します。この操作は取り消せません。
    </div>
  </div>
{:else}
  <div class="card delete-confirm" style="margin-top:10px;padding:12px;">
    <div style="font-size:12px;font-weight:600;color:var(--red);margin-bottom:6px;">本当に削除しますか？</div>
    <div style="font-size:11px;color:var(--text-secondary);margin-bottom:10px;line-height:1.4;">
      すべてのローカルデータ（データベース、設定ファイル、Cookie、キーチェーンの秘密情報など）が完全に削除されます。この操作は取り消せません。
    </div>
    {#if deleteErr}
      <div style="font-size:11px;color:var(--red);margin-bottom:8px;">{deleteErr}</div>
    {/if}
    <div style="display:flex;gap:8px;justify-content:flex-end;">
      <button class="btn-test" onclick={() => { deleteConfirmOpen = false; }}>キャンセル</button>
      <button class="btn-test danger-solid" onclick={confirmDelete}>削除する</button>
    </div>
  </div>
{/if}

<style>
  :global(.settings-main .about-hero) {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: 20px 0 0;
  }
  :global(.settings-main .about-icon) {
    width: 72px;
    height: 72px;
    -webkit-user-select: none;
    user-select: none;
  }
  :global(.settings-main .about-icon-btn) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    margin-bottom: 10px;
    border: none;
    background: transparent;
    cursor: pointer;
  }
  :global(.settings-main .about-name) {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  :global(.settings-main .about-version) {
    font-size: 11px;
    color: var(--text-secondary);
    margin-top: 1px;
    cursor: default;
    -webkit-user-select: none;
    user-select: none;
  }
  :global(.settings-main .about-version.shaking-0) {
    animation: shake-0 0.3s ease;
  }
  :global(.settings-main .about-version.shaking-1) {
    animation: shake-1 0.3s ease;
  }
  :global(.settings-main .about-tagline) {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 6px;
    font-style: italic;
  }
  :global(.settings-main .about-sep) {
    width: 36px;
    height: 0.5px;
    background: var(--border-strong);
    margin: 14px auto;
  }
  :global(.settings-main .about-row) {
    display: flex;
    justify-content: space-between;
    padding: 7px 14px;
    border-top: 0.5px solid var(--border);
    font-size: 12px;
  }
  :global(.settings-main .about-row:first-child) {
    border-top: none;
  }
  :global(.settings-main .about-row .al) {
    color: var(--text-secondary);
  }
  :global(.settings-main .about-row .ar) {
    color: var(--text-primary);
    font-weight: 500;
  }
  :global(.settings-main .about-row.clickable) {
    cursor: pointer;
  }
  :global(.settings-main .about-copy) {
    font-size: 10px;
    color: var(--text-tertiary);
    text-align: center;
    padding: 12px 14px 10px;
  }
  :global(.settings-main .about-copy .link) {
    cursor: pointer;
    color: var(--accent);
  }
  :global(.settings-main .update-inline) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  :global(.settings-main .update-current) {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.2;
  }
  :global(.settings-main .update-status) {
    margin-top: 4px;
    margin-bottom: 0;
    line-height: 1.45;
  }
  :global(.settings-main .update-progress-meta) {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 11px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }
  :global(.settings-main .update-progress-track) {
    margin-top: 8px;
    height: 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-primary) 10%, transparent);
    overflow: hidden;
  }
  :global(.settings-main .update-progress-fill) {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, color-mix(in srgb, var(--accent) 82%, #fff 18%), var(--accent));
    transition: width 0.18s ease;
  }
  :global(.settings-main .update-progress-fill.indeterminate) {
    animation: update-indeterminate 1.1s ease-in-out infinite;
  }
  :global(.settings-main .update-version) {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
  }
  :global(.settings-main .update-notes) {
    margin: 8px 0 0;
    padding: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font: inherit;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-secondary);
    background: transparent;
    border: none;
  }
  :global(.settings-main .update-actions) {
    display: flex;
    gap: 8px;
    margin-top: 10px;
    flex-wrap: wrap;
  }
  :global(.settings-main .update-manual-hint) {
    margin: 0;
  }
  :global(.settings-main .delete-warn) {
    padding: 0 12px 8px;
    font-size: 10px;
    color: var(--text-secondary);
    line-height: 1.35;
    pointer-events: none;
  }
  :global(.settings-main .delete-confirm) {
    border: 1px solid var(--red);
  }
  :global(.settings-main .btn-test.danger-solid) {
    background: var(--red);
    color: #fff;
    border-color: var(--red);
  }

  @keyframes -global-shake-0 {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(-3px); }
    75% { transform: translateX(3px); }
  }
  @keyframes -global-shake-1 {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(3px); }
    75% { transform: translateX(-3px); }
  }
  @keyframes update-indeterminate {
    0% { transform: translateX(-120%); }
    100% { transform: translateX(320%); }
  }
</style>
