<script lang="ts">
  import { authState, theme, cachedFetch, sessionExpired, reloginInProgress, cacheStatus, activeTab, agentReady } from "./stores";
  import type { StudentInfo, RefreshItemStatus } from "./stores";
  import { logout, fetchStudentProfile, openSettingsWindow, openProfileEditWindow, initiateRelogin, refreshAllData, updateAiReadiness } from "./api";
  import { emit } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Icon from "./Icon.svelte";
  import selahLogoUrl from "../assets/logo.png";

  const isWindows = navigator.userAgent.includes('Windows');
  const appWindow = getCurrentWindow();

  function minimizeWindow() { appWindow.minimize(); }
  function toggleMaximize() { appWindow.toggleMaximize(); }
  function closeWindow() { appWindow.close(); }

  let showProfile = $state(false);
  let profile = $state<StudentInfo | null>(null);
  let profileLoading = $state(false);

  async function handleLogout() {
    await logout();
    showProfile = false;
    profile = null;
  }

  async function toggleProfile() {
    showProfile = !showProfile;
    if (showProfile && !profile) {
      profileLoading = true;
      try {
        profile = await cachedFetch("student_profile", fetchStudentProfile);
      } catch (e) {
        console.warn("Failed to fetch profile:", e);
      } finally {
        profileLoading = false;
      }
    }
  }

  let reloginLoading = $state(false);
  let refreshing = $state(false);
  let showSyncPanel = $state(false);

  function toggleSyncPanel() {
    showSyncPanel = !showSyncPanel;
  }

  async function handleRefreshAll() {
    if (refreshing) return;
    showSyncPanel = true;
    refreshing = true;
    try {
      await refreshAllData();
    } finally {
      refreshing = false;
    }
  }

  function formatRelativeTime(ts: number): string {
    if (!ts) return "";
    const diff = Math.floor((Date.now() - ts) / 1000);
    if (diff < 60) return "たった今";
    if (diff < 3600) return `${Math.floor(diff / 60)}分前`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}時間前`;
    return `${Math.floor(diff / 86400)}日前`;
  }

  async function handleRelogin() {
    reloginLoading = true;
    try {
      await initiateRelogin();
    } finally {
      reloginLoading = false;
    }
  }

  function toggleTheme() {
    theme.update((t) => {
      const effective = t === "system"
        ? (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light")
        : t;
      const next = effective === "dark" ? "light" : "dark";
      document.documentElement.setAttribute("data-theme", next);
      localStorage.setItem("selah-theme", next);
      emit("theme-changed", next);
      invoke("set_app_theme", { theme: next }).catch(console.error);
      return next;
    });
  }

  function openDownloadsWindow() {
    invoke("open_downloads_window").catch(console.error);
  }

  function toggleSettings() {
    if ($activeTab === "settings") {
      activeTab.set("home");
      return;
    }
    void openSettingsWindow();
  }

  const AGENT_HINT_KEY = "selah-agent-hint-dismissed-v2";
  let showAgentHint = $state(typeof localStorage !== "undefined" && !localStorage.getItem(AGENT_HINT_KEY));

  function dismissAgentHint(ev?: MouseEvent) {
    if (ev) ev.stopPropagation();
    showAgentHint = false;
    try { localStorage.setItem(AGENT_HINT_KEY, "1"); } catch { /* ignore */ }
  }

  function toggleAgent() {
    if (!$agentReady && $activeTab !== 'agent') return;
    activeTab.update((t) => (t === "agent" ? "home" : "agent"));
  }
</script>

<div class="titlebar" data-tauri-drag-region>
  <div class="titlebar-left" data-tauri-drag-region>
    <div class="brand-anchor">
      <button
        class="brand-btn"
        class:active={$activeTab === 'agent'}
        onclick={toggleAgent}
        aria-label={$activeTab === 'agent' ? 'ホームに戻る' : !$agentReady ? 'Agent 利用不可（AI設定を確認してください）' : 'Selah Agent を開く'}
        title={$activeTab === 'agent' ? 'ホームに戻る' : !$agentReady ? 'Agent 利用不可' : 'Selah Agent'}
      >
        <img class="brand-logo" src={selahLogoUrl} alt="" />
        <span class="brand">
          {#if $activeTab === 'agent'}
            <span class="brand-name">エージェント</span>
            <span class="brand-tagline">……そばで、一緒に考える</span>
          {:else}
            <span class="brand-name">Selah</span>
            <span class="brand-tagline">新月の下で、知性を繋ぐ</span>
          {/if}
        </span>
      </button>
      {#if showAgentHint && $activeTab !== 'agent'}
        <div class="agent-hint" role="tooltip">
          <span class="agent-hint-arrow" aria-hidden="true"></span>
          <span class="agent-hint-text">……ここを押すと、わたしがいる</span>
          <button class="agent-hint-close" onclick={dismissAgentHint} aria-label="閉じる" title="閉じる">×</button>
        </div>
      {/if}
    </div>
  </div>
  <div class="titlebar-right">
    <button class="tb-btn" onclick={toggleTheme} title="テーマ切替" aria-label="テーマ切替">
      {#if $theme === "dark" || ($theme === "system" && typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches)}
        <Icon name="moon" size={14} />
      {:else}
        <Icon name="sun" size={14} />
      {/if}
    </button>
    <button class="tb-btn" onclick={openDownloadsWindow} title="ダウンロード" aria-label="ダウンロード">
      <Icon name="arrow.down.circle" size={14} />
    </button>
    <button
      class="tb-btn"
      class:active={$activeTab === "settings"}
      onclick={toggleSettings}
      title={$activeTab === "settings" ? "ホームに戻る" : "設定"}
      aria-label={$activeTab === "settings" ? "ホームに戻る" : "設定"}
    >
      <Icon name="gear" size={14} />
    </button>
    {#if $authState.authenticated && !$sessionExpired && !$reloginInProgress}
      <button
        class="sync-badge"
        class:syncing={refreshing || $cacheStatus.fullRefreshing || $cacheStatus.refreshingCount > 0}
        onclick={toggleSyncPanel}
        title={$cacheStatus.lastUpdated ? `最終更新: ${formatRelativeTime($cacheStatus.lastUpdated)}` : "データを更新"}
      >
        <svg class="sync-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0 1 15-6.7L21 8"/>
          <path d="M3 22v-6h6"/><path d="M21 12a9 9 0 0 1-15 6.7L3 16"/>
        </svg>
        {#if $cacheStatus.lastUpdated}
          <span class="sync-time">{formatRelativeTime($cacheStatus.lastUpdated)}</span>
        {/if}
      </button>
    {/if}
    {#if $reloginInProgress}
      <span class="titlebar-status" title="再ログイン中…">
        <span class="titlebar-status-spinner"></span>
        <span class="titlebar-status-text">認証中</span>
      </span>
    {:else if $sessionExpired}
      <button class="reauth-badge" onclick={handleRelogin} disabled={reloginLoading} title="セッションが期限切れです。クリックして再認証してください。">
        {#if reloginLoading}
          <span class="reauth-spinner"></span>
        {:else}
          <Icon name="exclamationmark.triangle" size={13} />
        {/if}
        <span class="reauth-text">再認証</span>
      </button>
    {:else if $authState.authenticated}
      <button class="user-badge" onclick={toggleProfile}>
        <span class="user-name">{$authState.displayName || $authState.username}</span>
        {#if $authState.faculty}
          <span class="user-faculty">{$authState.faculty}</span>
        {/if}
      </button>
    {:else}
      <span class="titlebar-status" title="セッション復元中…">
        <span class="titlebar-status-spinner"></span>
      </span>
    {/if}
    {#if isWindows}
      <div class="window-controls">
        <button class="win-ctrl" onclick={minimizeWindow} title="最小化">
          <svg width="10" height="1" viewBox="0 0 10 1"><line x1="0" y1="0.5" x2="10" y2="0.5" stroke="currentColor" stroke-width="1"/></svg>
        </button>
        <button class="win-ctrl" onclick={toggleMaximize} title="最大化">
          <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" stroke-width="1" fill="none"/></svg>
        </button>
        <button class="win-ctrl win-close" onclick={closeWindow} title="閉じる">
          <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" stroke-width="1.2"/><line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" stroke-width="1.2"/></svg>
        </button>
      </div>
    {/if}
  </div>
</div>

{#if showProfile}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="profile-backdrop" onclick={() => showProfile = false}></div>
  <div class="profile-popover">
    {#if profileLoading}
      <div class="profile-loading"><span class="profile-spinner"></span></div>
    {:else if profile}
      <div class="profile-header">
        <div>
          <div class="profile-name">{profile.name || $authState.displayName}</div>
          {#if profile.name_en}<div class="profile-name-en">{profile.name_en}</div>{/if}
        </div>
      </div>
      <div class="profile-grid">
        <div class="profile-label">学生番号</div>
        <div class="profile-value">{profile.student_id || "—"}</div>
        <div class="profile-label">学部・研究科</div>
        <div class="profile-value">{profile.faculty || "—"}</div>
        <div class="profile-label">学科</div>
        <div class="profile-value">{profile.department || "—"}</div>
        {#if profile.major}
          <div class="profile-label">専攻・コース</div>
          <div class="profile-value">{profile.major}</div>
        {/if}
        {#if profile.student_type}
          <div class="profile-label">学生区分</div>
          <div class="profile-value">{profile.student_type}</div>
        {/if}
        {#if profile.affiliation_type}
          <div class="profile-label">所属区分</div>
          <div class="profile-value">{profile.affiliation_type}</div>
        {/if}
        {#if profile.status}
          <div class="profile-label">学生状態</div>
          <div class="profile-value">{profile.status}</div>
        {/if}
        {#if profile.class}
          <div class="profile-label">クラス</div>
          <div class="profile-value">{profile.class}</div>
        {/if}
        {#if profile.address}
          <div class="profile-label">住所・電話</div>
          <div class="profile-value">{profile.address}</div>
        {/if}
      </div>
      <div class="profile-actions">
        <button class="profile-edit-btn" onclick={() => { showProfile = false; openProfileEditWindow(); }}>
          個人情報を編集
        </button>
        <button class="profile-logout-btn" onclick={handleLogout}>
          ログアウト
        </button>
      </div>
    {:else}
      <div class="profile-empty">学生情報を取得できませんでした</div>
    {/if}
  </div>
{/if}

{#if showSyncPanel}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="sync-backdrop" onclick={() => showSyncPanel = false}></div>
  <div class="sync-popover">
    <div class="sync-popover-header">
      <div class="sync-popover-header-left">
        <span class="sync-popover-title">データ同期</span>
        {#if $cacheStatus.fullRefreshing}
          {@const done = $cacheStatus.items.filter(i => i.status === "done" || i.status === "error").length}
          {@const total = $cacheStatus.items.length}
          <span class="sync-popover-progress">{done}/{total}</span>
        {/if}
      </div>
      <button
        class="sync-popover-refresh-btn"
        onclick={handleRefreshAll}
        disabled={refreshing || $cacheStatus.fullRefreshing}
      >
        {#if refreshing || $cacheStatus.fullRefreshing}
          <span class="sync-popover-spinner"></span>
          更新中
        {:else}
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0 1 15-6.7L21 8"/>
            <path d="M3 22v-6h6"/><path d="M21 12a9 9 0 0 1-15 6.7L3 16"/>
          </svg>
          すべて更新
        {/if}
      </button>
    </div>
    {#if $cacheStatus.fullRefreshing}
      <div class="sync-progress-bar">
        <div class="sync-progress-fill" style="width: {$cacheStatus.items.length ? ($cacheStatus.items.filter(i => i.status === 'done' || i.status === 'error').length / $cacheStatus.items.length * 100) : 0}%"></div>
      </div>
    {/if}
    {#if $cacheStatus.lastUpdated}
      <div class="sync-popover-meta">最終更新: {formatRelativeTime($cacheStatus.lastUpdated)}</div>
    {/if}
    {#if $cacheStatus.items.length > 0}
      {@const grouped = Object.groupBy($cacheStatus.items, (it: RefreshItemStatus) => it.platform)}
      <div class="sync-popover-list">
        {#each Object.entries(grouped) as [platform, items]}
          <div class="sync-group">
            <div class="sync-group-label">{platform}</div>
            {#each items as item}
              <div class="sync-item">
                <span class="sync-item-indicator" class:pending={item.status === "pending"} class:running={item.status === "running"} class:done={item.status === "done"} class:error={item.status === "error"}>
                  {#if item.status === "running"}
                    <span class="sync-item-spinner"></span>
                  {:else if item.status === "done"}
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                  {:else if item.status === "error"}
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                  {:else}
                    <span class="sync-item-dot"></span>
                  {/if}
                </span>
                <span class="sync-item-label">{item.label}</span>
              </div>
            {/each}
          </div>
        {/each}
      </div>
    {:else}
      <div class="sync-popover-empty">「すべて更新」をクリックしてデータを同期してください</div>
    {/if}
  </div>
{/if}

<style>
  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 52px;
    padding: 0 16px;
    background: var(--bg-titlebar);
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border-bottom: 0.5px solid var(--glass-border);
    box-shadow: var(--glass-highlight);
    -webkit-app-region: drag;
    position: relative;
    z-index: 100;
    flex-shrink: 0;
  }

  .titlebar-left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .brand-anchor {
    position: relative;
    display: inline-flex;
  }

  .brand-btn {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 3px 10px 3px 4px;
    border: none;
    background: transparent;
    border-radius: 10px;
    cursor: pointer;
    text-align: left;
    -webkit-app-region: no-drag;
    transition: background 0.15s, box-shadow 0.15s, transform 0.15s;
  }

  .agent-hint {
    position: absolute;
    top: calc(100% + 8px);
    left: 12px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px 6px 12px;
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-primary));
    border: 0.5px solid color-mix(in srgb, var(--accent) 35%, var(--glass-border));
    border-radius: 14px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.12);
    font-size: 11.5px;
    color: var(--text-primary);
    white-space: nowrap;
    -webkit-app-region: no-drag;
    z-index: 150;
    animation: agent-hint-bob 2.4s ease-in-out infinite;
    pointer-events: auto;
  }
  .agent-hint-arrow {
    position: absolute;
    top: -5px;
    left: 18px;
    width: 10px;
    height: 10px;
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-primary));
    border-left: 0.5px solid color-mix(in srgb, var(--accent) 35%, var(--glass-border));
    border-top: 0.5px solid color-mix(in srgb, var(--accent) 35%, var(--glass-border));
    transform: rotate(45deg);
  }
  .agent-hint-text {
    font-weight: 500;
    letter-spacing: 0.2px;
  }
  .agent-hint-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    margin-left: 2px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .agent-hint-close:hover {
    background: color-mix(in srgb, var(--text-primary) 10%, transparent);
    color: var(--text-primary);
  }
  @keyframes agent-hint-bob {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-2px); }
  }
  .brand-logo {
    height: 28px;
    width: auto;
    display: block;
    transition: filter 0.15s;
  }
  .brand-btn:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }

  .brand {
    display: flex;
    flex-direction: column;
    gap: 1px;
    user-select: none;
  }
  .brand-name {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: 0.5px;
    line-height: 1;
  }
  .brand-tagline {
    font-size: 10px;
    color: var(--text-tertiary);
    letter-spacing: 0.3px;
    line-height: 1;
  }

  .titlebar-right {
    display: flex;
    align-items: center;
    gap: 6px;
    -webkit-app-region: no-drag;
  }

  .window-controls {
    display: flex;
    align-items: center;
    margin-left: 4px;
  }

  .win-ctrl {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
    border-radius: 4px;
  }

  .win-ctrl:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .win-ctrl.win-close:hover {
    background: #e81123;
    color: #fff;
  }

  .user-badge {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    font-size: 12px;
    color: var(--text-secondary);
    background: var(--bg-tertiary);
    border-radius: 20px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .user-badge:hover {
    background: var(--bg-hover);
  }

  .titlebar-status {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    font-size: 11px;
    color: var(--text-secondary);
    border-radius: 20px;
  }

  .titlebar-status-spinner {
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .titlebar-status-text {
    white-space: nowrap;
  }

  .reauth-badge {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 12px;
    font-size: 12px;
    font-weight: 600;
    color: var(--orange, #ff9500);
    background: color-mix(in srgb, var(--orange, #ff9500) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--orange, #ff9500) 30%, transparent);
    border-radius: 20px;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
    animation: pulse-glow 2s ease-in-out infinite;
  }

  .reauth-badge:hover {
    background: color-mix(in srgb, var(--orange, #ff9500) 20%, transparent);
    border-color: color-mix(in srgb, var(--orange, #ff9500) 50%, transparent);
  }

  .reauth-badge:disabled {
    cursor: wait;
    opacity: 0.7;
  }

  .reauth-text {
    white-space: nowrap;
  }

  .reauth-spinner {
    width: 13px;
    height: 13px;
    border: 1.5px solid color-mix(in srgb, var(--orange, #ff9500) 30%, transparent);
    border-top-color: var(--orange, #ff9500);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes pulse-glow {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--orange, #ff9500) 20%, transparent); }
    50% { box-shadow: 0 0 8px 2px color-mix(in srgb, var(--orange, #ff9500) 15%, transparent); }
  }

  .user-name {
    color: var(--text-primary);
    font-weight: 500;
  }

  .user-faculty {
    font-size: 10px;
    opacity: 0.7;
  }

  .tb-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border-radius: 6px;
    color: var(--text-secondary);
    font-size: 11px;
  }

  .tb-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .tb-btn.active {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 7%, transparent);
  }

  .sync-badge {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    font-size: 12px;
    color: var(--text-secondary);
    background: var(--bg-tertiary);
    border-radius: 20px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
    white-space: nowrap;
  }

  .sync-badge:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .sync-badge.syncing .sync-icon {
    animation: spin 1s linear infinite;
  }

  .sync-badge.syncing {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-tertiary));
  }

  .sync-icon {
    flex-shrink: 0;
    transition: color 0.15s;
  }

  .sync-time {
    font-variant-numeric: tabular-nums;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  /* Sync status popover */
  .sync-backdrop {
    position: fixed;
    inset: 0;
    z-index: 199;
  }

  .sync-popover {
    position: fixed;
    top: 52px;
    right: 80px;
    z-index: 200;
    width: 280px;
    max-height: 420px;
    overflow-y: auto;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.18);
    padding: 14px;
    animation: pop-in 0.2s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }

  .sync-popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .sync-popover-header-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .sync-popover-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .sync-popover-refresh-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 10px;
    font-size: 11px;
    font-weight: 500;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .sync-popover-refresh-btn:hover {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }

  .sync-popover-refresh-btn:disabled {
    cursor: wait;
    opacity: 0.7;
  }

  .sync-popover-spinner {
    width: 10px;
    height: 10px;
    border: 1.5px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .sync-popover-progress {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .sync-progress-bar {
    height: 2px;
    background: var(--border);
    border-radius: 1px;
    margin-bottom: 10px;
    overflow: hidden;
  }

  .sync-progress-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 1px;
    transition: width 0.3s ease;
  }

  .sync-popover-meta {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-bottom: 12px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border);
  }

  .sync-popover-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .sync-group-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 4px;
  }

  .sync-group {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .sync-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 6px;
    font-size: 12px;
    color: var(--text-secondary);
    border-radius: 6px;
    transition: background 0.1s;
  }

  .sync-item:has(.running) {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }

  .sync-item-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  .sync-item-indicator.done {
    color: var(--green, #34c759);
  }

  .sync-item-indicator.error {
    color: var(--red, #ff3b30);
  }

  .sync-item-indicator.running {
    color: var(--accent);
  }

  .sync-item-spinner {
    width: 10px;
    height: 10px;
    border: 1.5px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .sync-item-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--border-strong);
    opacity: 0.4;
  }

  .sync-item-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sync-item-indicator.done + .sync-item-label {
    color: var(--text-tertiary);
  }

  .sync-popover-empty {
    font-size: 12px;
    color: var(--text-tertiary);
    text-align: center;
    padding: 20px 8px;
    line-height: 1.5;
  }

  /* Profile popover */
  .profile-backdrop {
    position: fixed;
    inset: 0;
    z-index: 199;
  }

  .profile-popover {
    position: fixed;
    top: 52px;
    right: 16px;
    z-index: 200;
    width: 320px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.18);
    padding: 16px;
    animation: pop-in 0.2s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }

  @keyframes pop-in {
    from { opacity: 0; transform: translateY(-6px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .profile-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border);
  }

  .profile-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .profile-name-en {
    font-size: 11px;
    color: var(--text-secondary);
    margin-top: 1px;
  }

  .profile-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 12px;
    font-size: 12px;
  }

  .profile-label {
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .profile-value {
    color: var(--text-primary);
    font-weight: 500;
    word-break: break-all;
  }

  .profile-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }

  .profile-spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .profile-empty {
    text-align: center;
    padding: 16px;
    color: var(--text-secondary);
    font-size: 13px;
  }
  .profile-edit-btn {
    display: block;
    width: 100%;
    margin-top: 12px;
    padding: 8px 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent, #002855);
    background: transparent;
    border: 1px solid var(--accent, #002855);
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .profile-edit-btn:hover {
    background: var(--accent, #002855);
    color: #fff;
  }
  .profile-actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  .profile-actions .profile-edit-btn {
    flex: 1;
    margin-top: 0;
  }
  .profile-logout-btn {
    flex: 1;
    padding: 8px 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--red, #ff3b30);
    background: transparent;
    border: 1px solid var(--red, #ff3b30);
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .profile-logout-btn:hover {
    background: var(--red, #ff3b30);
    color: #fff;
  }
</style>
