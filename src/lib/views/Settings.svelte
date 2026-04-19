<script lang="ts">
  import { onDestroy } from "svelte";
  import { activeSettingsPanel, devModeActive } from "../stores";
  import type { SettingsPanel } from "../stores";
  import SettingsAi from "./settings/SettingsAi.svelte";
  import SettingsSession from "./settings/SettingsSession.svelte";
  import SettingsMail from "./settings/SettingsMail.svelte";
  import SettingsCalendar from "./settings/SettingsCalendar.svelte";
  import SettingsNotification from "./settings/SettingsNotification.svelte";
  import SettingsDownload from "./settings/SettingsDownload.svelte";
  import SettingsAbout from "./settings/SettingsAbout.svelte";
  import SettingsDebug from "./settings/SettingsDebug.svelte";

  interface NavItem {
    id: SettingsPanel;
    label: string;
    icon: string;
    devOnly?: boolean;
  }

  const navItems: NavItem[] = [
    { id: "ai", label: "AI 設定", icon: "sparkles" },
    { id: "session", label: "セッション", icon: "lock" },
    { id: "mail", label: "メール", icon: "envelope" },
    { id: "calendar", label: "カレンダー", icon: "calendar" },
    { id: "notification", label: "通知", icon: "bell" },
    { id: "download", label: "ダウンロード", icon: "arrow.down" },
    { id: "about", label: "このアプリについて", icon: "info" },
    { id: "debug", label: "デバッグ", icon: "wrench", devOnly: true },
  ];
  const AUTO_SAVE_DEBOUNCE_MS = 1600;

  let visibleNav = $derived(navItems.filter(n => !n.devOnly || $devModeActive));
  let saveBusy = $state(false);
  let savePendingPanel = $state<SettingsPanel | null>(null);
  let saveState = $state<"idle" | "saving" | "saved" | "error">("idle");
  let saveHint = $state("変更は自動保存されます");
  let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let hintClearTimer: ReturnType<typeof setTimeout> | null = null;

  let aiPanel = $state<any>(null);
  let mailPanel = $state<any>(null);
  let calendarPanel = $state<any>(null);
  let notificationPanel = $state<any>(null);
  let downloadPanel = $state<any>(null);

  const saveEnabledPanels: SettingsPanel[] = ["ai", "mail", "calendar", "notification", "download"];
  let shouldShowSaveBar = $derived(saveEnabledPanels.includes($activeSettingsPanel));

  // If dev mode gets turned off while debug panel is active, bounce back to About
  $effect(() => {
    if (!$devModeActive && $activeSettingsPanel === "debug") {
      activeSettingsPanel.set("about");
    }
  });

  async function saveCurrentPanel(panel: SettingsPanel = $activeSettingsPanel) {
    if (!saveEnabledPanels.includes(panel)) return;
    if (saveBusy) {
      savePendingPanel = panel;
      return;
    }
    saveBusy = true;
    saveState = "saving";
    saveHint = "保存中...";
    try {
      switch (panel) {
        case "ai":
          await aiPanel?.save?.();
          break;
        case "mail":
          await mailPanel?.save?.();
          break;
        case "calendar":
          await calendarPanel?.save?.();
          break;
        case "notification":
          await notificationPanel?.save?.();
          break;
        case "download":
          await downloadPanel?.save?.();
          break;
      }
      saveState = "saved";
      saveHint = "自動保存しました";
      if (hintClearTimer) clearTimeout(hintClearTimer);
      hintClearTimer = setTimeout(() => {
        saveState = "idle";
        saveHint = "変更は自動保存されます";
      }, 1800);
    } catch {
      saveState = "error";
      saveHint = "保存に失敗しました";
    } finally {
      saveBusy = false;
      if (savePendingPanel) {
        const next = savePendingPanel;
        savePendingPanel = null;
        if (next === $activeSettingsPanel) {
          void saveCurrentPanel(next);
        }
      }
    }
  }

  function queueAutoSave() {
    if (!shouldShowSaveBar) return;
    const panelAtTrigger = $activeSettingsPanel;
    if (autoSaveTimer) clearTimeout(autoSaveTimer);
    autoSaveTimer = setTimeout(() => {
      if (panelAtTrigger === $activeSettingsPanel) {
        void saveCurrentPanel(panelAtTrigger);
      }
    }, AUTO_SAVE_DEBOUNCE_MS);
  }

  async function switchPanel(panel: SettingsPanel) {
    if (panel === $activeSettingsPanel) return;
    if (saveEnabledPanels.includes($activeSettingsPanel)) {
      if (autoSaveTimer) {
        clearTimeout(autoSaveTimer);
        autoSaveTimer = null;
      }
      await saveCurrentPanel($activeSettingsPanel);
    }
    activeSettingsPanel.set(panel);
  }

  onDestroy(() => {
    if (autoSaveTimer) clearTimeout(autoSaveTimer);
    if (hintClearTimer) clearTimeout(hintClearTimer);
  });

  function iconPath(id: string): string {
    switch (id) {
      case "sparkles":
        return "M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2zM15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z";
      case "lock":
        return "M3 5h14v10H3zM7 5V3.5a3 3 0 016 0V5";
      case "envelope":
        return "M2 4h16v12H2zM2 4l8 7 8-7";
      case "calendar":
        return "M2.5 3.5h15v13h-15zM2.5 7.5h15M6 2v3M14 2v3";
      case "bell":
        return "M10 2.5a5 5 0 015 5v3l1.5 2H3.5l1.5-2v-3a5 5 0 015-5zM8 14.5a2 2 0 004 0";
      case "arrow.down":
        return "M10 3v10M6 9l4 4 4-4M3 14v2a1 1 0 001 1h12a1 1 0 001-1v-2";
      case "info":
        return "M10 17.5a7.5 7.5 0 100-15 7.5 7.5 0 000 15zM10 9v4.5M10 7v0.1";
      case "wrench":
        return "M15 7a3 3 0 11-5 0 3 3 0 015 0zM12.5 9.5l-7 7M5 15l1.5 1.5";
      default:
        return "";
    }
  }
</script>

<div class="settings-root">
  <aside class="settings-sidebar">
    {#each visibleNav as item}
      <button
        class="nav-item"
        class:active={$activeSettingsPanel === item.id}
        onclick={() => { void switchPanel(item.id); }}
      >
        <svg class="nav-icon" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">
          <path d={iconPath(item.icon)} />
        </svg>
        <span class="nav-label">{item.label}</span>
      </button>
    {/each}
  </aside>

  <main class="settings-main" oninput={queueAutoSave} onchange={queueAutoSave}>
    <div class="settings-scroll">
      {#if $activeSettingsPanel === "ai"}
        <SettingsAi bind:this={aiPanel} />
      {:else if $activeSettingsPanel === "session"}
        <SettingsSession />
      {:else if $activeSettingsPanel === "mail"}
        <SettingsMail bind:this={mailPanel} />
      {:else if $activeSettingsPanel === "calendar"}
        <SettingsCalendar bind:this={calendarPanel} />
      {:else if $activeSettingsPanel === "notification"}
        <SettingsNotification bind:this={notificationPanel} />
      {:else if $activeSettingsPanel === "download"}
        <SettingsDownload bind:this={downloadPanel} />
      {:else if $activeSettingsPanel === "about"}
        <SettingsAbout />
      {:else if $activeSettingsPanel === "debug" && $devModeActive}
        <SettingsDebug />
      {/if}
    </div>

    {#if shouldShowSaveBar}
      <div class="settings-bottom-save">
        <span class="auto-save-hint" class:saving={saveState === "saving"} class:saved={saveState === "saved"} class:error={saveState === "error"}>
          {saveHint}
        </span>
      </div>
    {/if}
  </main>
</div>

<style>
  .settings-root {
    display: flex;
    height: 100%;
    min-height: 0;
    margin: -24px;
    background: transparent;
  }

  .settings-sidebar {
    width: 168px;
    flex-shrink: 0;
    padding: 16px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-right: 0.5px solid var(--border);
    overflow-y: auto;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 7px;
    font-size: 12px;
    font-weight: 400;
    color: var(--text-primary);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    transition: background 0.12s, color 0.12s;
  }

  .nav-item:hover {
    background: var(--bg-hover);
  }

  .nav-item.active {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
    font-weight: 500;
  }

  .nav-icon {
    width: 15px;
    height: 15px;
    flex-shrink: 0;
    opacity: 0.75;
  }

  .nav-item.active .nav-icon {
    opacity: 1;
  }

  .nav-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .settings-main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding: 18px 24px 0;
  }

  .settings-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 14px;
  }

  .settings-bottom-save {
    display: flex;
    justify-content: flex-end;
    padding: 10px 0 12px;
    background: var(--bg-primary);
    border-top: 0.5px solid var(--border);
  }

  .auto-save-hint {
    font-size: 11px;
    color: var(--text-secondary);
    transition: color 0.15s ease;
  }

  .auto-save-hint.saving {
    color: var(--accent);
  }

  .auto-save-hint.saved {
    color: var(--green, #34c759);
    font-weight: 600;
  }

  .auto-save-hint.error {
    color: var(--red);
  }

  :global(.settings-main .hero-card) {
    background: var(--bg-secondary);
    border-radius: 10px;
    box-shadow: 0 0.5px 1px rgba(0,0,0,0.04), 0 1px 3px rgba(0,0,0,0.04);
    margin-bottom: 16px;
    padding: 10px 14px;
    display: flex;
    gap: 10px;
  }
  :global(.settings-main .hero-icon) {
    width: 30px; height: 30px;
    border-radius: 7px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.15), rgba(0, 122, 255, 0.15));
  }
  :global(.settings-main .hero-icon svg) {
    width: 18px; height: 18px;
  }
  :global(.settings-main .hero-text) { flex: 1; min-width: 0; }
  :global(.settings-main .panel-title) {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.01em;
    margin: 0;
  }
  :global(.settings-main .panel-desc) {
    font-size: 10.5px;
    color: var(--text-secondary);
    line-height: 1.4;
  }
  :global(.settings-main .card-label) {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.01em;
    padding: 0;
    margin: 14px 2px 6px;
  }
  :global(.settings-main .card-label:first-child) { margin-top: 0; }
  :global(.settings-main .card) {
    background: var(--bg-secondary);
    border-radius: 10px;
    box-shadow: 0 0.5px 1px rgba(0,0,0,0.04), 0 1px 3px rgba(0,0,0,0.04);
    margin-bottom: 10px;
    overflow: hidden;
  }
  :global(.settings-main .row) {
    padding: 9px 14px;
    border-top: 0.5px solid var(--border);
    display: flex;
    align-items: center;
    gap: 10px;
  }
  :global(.settings-main .card > .row:first-child) { border-top: none; }
  :global(.settings-main .row-label) {
    font-size: 12px;
    color: var(--text-primary);
    white-space: nowrap;
    min-width: 86px;
  }
  :global(.settings-main .row-input) { flex: 1; min-width: 0; }
  :global(.settings-main .row-input input),
  :global(.settings-main .row-input select) {
    width: 100%;
    padding: 5px 8px;
    font-size: 12px;
    font-family: inherit;
    color: var(--text-primary);
    background: var(--bg-tertiary);
    border: 0.5px solid var(--border-strong);
    border-radius: 8px;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  :global(.settings-main .row-input input:focus),
  :global(.settings-main .row-input select:focus) {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent);
  }
  :global(.settings-main .row-input input[type="password"]) {
    font-family: monospace;
    letter-spacing: 0.05em;
  }
  :global(.settings-main .hint) {
    font-size: 10.5px;
    color: var(--text-tertiary);
    margin-top: 3px;
    line-height: 1.4;
  }
  :global(.settings-main .btn-test) {
    flex-shrink: 0;
    padding: 5px 10px;
    font-size: 10.5px;
    font-weight: 600;
    font-family: inherit;
    border-radius: 6px;
    cursor: pointer;
    border: 0.5px solid var(--border-strong);
    background: var(--bg-hover);
    color: var(--text-secondary);
    white-space: nowrap;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  :global(.settings-main .btn-test:hover:not(:disabled)) {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  :global(.settings-main .btn-test:disabled) {
    opacity: 0.5;
    cursor: not-allowed;
  }
  :global(.settings-main .session-indicator) {
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  :global(.settings-main .session-dot) {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  :global(.settings-main .session-dot.ok) { background: var(--green, #34c759); }
  :global(.settings-main .session-dot.ng) { background: var(--red); }
  :global(.settings-main .spinner-sm) {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: settings-spin 0.6s linear infinite;
  }
  :global(.settings-main .status-msg) {
    font-size: 11px;
    line-height: 1.4;
  }
  :global(.settings-main .status-msg.success) { color: var(--green, #34c759); }
  :global(.settings-main .status-msg.error) { color: var(--red); }
  :global(.settings-main .status-msg.loading) { color: var(--text-secondary); }

  @keyframes settings-spin {
    to { transform: rotate(360deg); }
  }
</style>
