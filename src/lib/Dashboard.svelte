<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { activeTab, aiRefreshRequested } from "./stores";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./Icon.svelte";
  import Titlebar from "./Titlebar.svelte";
  import Timetable from "./views/Timetable.svelte";
  import GradesView from "./views/GradesView.svelte";
  import Registration from "./views/Registration.svelte";
  import Syllabus from "./views/Syllabus.svelte";
  import LunaTodo from "./views/LunaTodo.svelte";
  import NotificationsUnified from "./views/NotificationsUnified.svelte";
  import ChangeInfo from "./views/ChangeInfo.svelte";

  interface Tab {
    id: string;
    label: string;
    icon: IconName;
    section?: string;
  }

  const tabs: Tab[] = [
    { id: "timetable", label: "時間割", icon: "calendar", section: "授業" },
    { id: "todo", label: "TODO", icon: "checkmark.circle" },
    { id: "grades", label: "成績照会", icon: "chart.bar" },
    { id: "registration", label: "履修登録", icon: "list.clipboard" },
    { id: "syllabus", label: "シラバス検索", icon: "book" },
    { id: "notifications", label: "お知らせ", icon: "bell", section: "お知らせ" },
    { id: "changes", label: "変更情報", icon: "arrow.triangle.swap" },
  ];

  // Track which tabs have been visited (lazy mount: create once, then keep alive)
  let visited = $state(new Set<string>(["timetable"]));
  $effect(() => {
    const tab = $activeTab;
    if (!visited.has(tab)) {
      visited = new Set([...visited, tab]);
    }
  });

  let unlistenRefresh: (() => void) | null = null;
  onMount(async () => {
    unlistenRefresh = await listen('ai-refresh-request', () => {
      console.log('[Dashboard] ai-refresh-request received');
      activeTab.set('timetable');
      aiRefreshRequested.set(true);
    });
    console.log('[Dashboard] ai-refresh-request listener registered');
  });
  onDestroy(() => { if (unlistenRefresh) unlistenRefresh(); });
</script>

<div class="dashboard">
  <nav class="sidebar" data-tauri-drag-region>
    <div class="sidebar-drag-area" data-tauri-drag-region></div>
    <div class="sidebar-scroll">
      {#each tabs as tab}
        {#if tab.section && tab.section !== (tabs[tabs.indexOf(tab) - 1]?.section ?? "")}
          <div class="section-label">{tab.section}</div>
        {/if}
        <button
          class="nav-item"
          class:active={$activeTab === tab.id}
          onclick={() => activeTab.set(tab.id)}
        >
          <Icon name={tab.icon} size={16} />
          <span class="nav-label">{tab.label}</span>
        </button>
      {/each}
    </div>
  </nav>

  <div class="main-area">
    <Titlebar />
    <div class="content">
      <div class="view-panel" class:active={$activeTab === "timetable"}>
        <Timetable />
      </div>
      {#if visited.has("todo")}
        <div class="view-panel" class:active={$activeTab === "todo"}>
          <LunaTodo />
        </div>
      {/if}
      {#if visited.has("grades")}
        <div class="view-panel" class:active={$activeTab === "grades"}>
          <GradesView />
        </div>
      {/if}
      {#if visited.has("registration")}
        <div class="view-panel" class:active={$activeTab === "registration"}>
          <Registration />
        </div>
      {/if}
      {#if visited.has("syllabus")}
        <div class="view-panel" class:active={$activeTab === "syllabus"}>
          <Syllabus />
        </div>
      {/if}
      {#if visited.has("notifications")}
        <div class="view-panel" class:active={$activeTab === "notifications"}>
          <NotificationsUnified />
        </div>
      {/if}
      {#if visited.has("changes")}
        <div class="view-panel" class:active={$activeTab === "changes"}>
          <ChangeInfo />
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .dashboard {
    display: flex;
    height: 100vh;
  }

  .sidebar {
    width: 210px;
    background: var(--bg-sidebar);
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border-right: 0.5px solid var(--glass-border);
    flex-shrink: 0;
    padding-top: 20px; /* space for macOS traffic lights */
    display: flex;
    flex-direction: column;
    box-shadow: var(--glass-highlight);
  }

  .sidebar-scroll {
    padding: 4px 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow-y: auto;
    flex: 1;
  }

  .sidebar-drag-area {
    height: 12px;
    flex-shrink: 0;
    -webkit-app-region: drag;
  }

  .section-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    padding: 18px 10px 5px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .section-label:first-child {
    padding-top: 4px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 400;
    color: var(--text-primary);
    background: transparent;
    transition: all 0.15s ease;
    width: 100%;
    text-align: left;
    border: 0.5px solid transparent;
  }

  .nav-item:hover {
    background: var(--bg-hover);
  }

  .nav-item.active {
    background: var(--glass-bg);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    color: var(--accent);
    font-weight: 500;
    box-shadow: var(--glass-highlight), 0 1px 4px rgba(0, 0, 0, 0.04);
    border: 0.5px solid var(--glass-border);
  }

  .nav-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .content {
    flex: 1;
    overflow: hidden;
    background: transparent;
    position: relative;
  }

  .view-panel {
    display: none;
    position: absolute;
    inset: 0;
    overflow: auto;
    padding: 24px;
  }

  .view-panel.active {
    display: block;
  }
</style>
