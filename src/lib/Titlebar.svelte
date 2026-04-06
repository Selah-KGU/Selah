<script lang="ts">
  import { authState, theme, cachedFetch } from "./stores";
  import type { StudentInfo } from "./stores";
  import { logout, fetchStudentProfile, openSettingsWindow, openProfileEditWindow } from "./api";
  import Icon from "./Icon.svelte";
  import kgLogoRaw from "../assets/kg-logo.svg?raw";

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
        profile = await cachedFetch("profile", fetchStudentProfile);
      } catch (e) {
        console.warn("Failed to fetch profile:", e);
      } finally {
        profileLoading = false;
      }
    }
  }

  async function toggleTheme() {
    const { emit } = await import("@tauri-apps/api/event");
    const { invoke } = await import("@tauri-apps/api/core");
    theme.update((t) => {
      const effective = t === "system"
        ? (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light")
        : t;
      const next = effective === "dark" ? "light" : "dark";
      document.documentElement.setAttribute("data-theme", next);
      localStorage.setItem("selah-theme", next);
      emit("theme-changed", next);
      invoke("set_app_theme", { theme: next });
      return next;
    });
  }
</script>

<div class="titlebar" data-tauri-drag-region>
  <div class="titlebar-left" data-tauri-drag-region>
    <span class="logo" aria-label="関西学院大学">{@html kgLogoRaw}</span>
  </div>
  <div class="titlebar-right">
    <button class="tb-btn" onclick={toggleTheme} title="テーマ切替" aria-label="テーマ切替">
      {#if $theme === "dark" || ($theme === "system" && typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches)}
        <Icon name="moon" size={14} />
      {:else}
        <Icon name="sun" size={14} />
      {/if}
    </button>
    <button class="tb-btn" onclick={() => openSettingsWindow()} title="設定" aria-label="設定">
      <Icon name="gear" size={14} />
    </button>
    {#if $authState.authenticated}
      <button class="user-badge" onclick={toggleProfile}>
        <span class="user-name">{$authState.displayName || $authState.username}</span>
        {#if $authState.faculty}
          <span class="user-faculty">{$authState.faculty}</span>
        {/if}
      </button>
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
    z-index: 100;
    flex-shrink: 0;
  }

  .titlebar-left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .logo {
    height: 18px;
    opacity: 0.85;
    display: flex;
    align-items: center;
    color: #231f20;
  }
  .logo :global(svg) {
    height: 24px;
    width: auto;
  }
  :global([data-theme="dark"]) .logo {
    color: var(--text-primary);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .logo {
      color: var(--text-primary);
    }
  }

  .titlebar-right {
    display: flex;
    align-items: center;
    gap: 6px;
    -webkit-app-region: no-drag;
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
    background: var(--accent-light);
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

  .profile-avatar {
    color: var(--accent);
    flex-shrink: 0;
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
