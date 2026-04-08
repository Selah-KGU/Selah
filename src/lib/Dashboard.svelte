<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { activeTab, aiRefreshRequested, unreadNotifCount, unreadMailCount, onCacheUpdate, getCached } from "./stores";
  import type { NotificationsData } from "./stores";
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
  import HomePage from "./views/HomePage.svelte";
  import MailView from "./views/MailView.svelte";
  import type { MailMessage, KwicPortalHome, ReadIdsResponse } from "./api";
  import { getReadNotifications } from "./api";
  import type { LunaNotification } from "./types";

  interface Tab {
    id: string;
    label: string;
    icon: IconName;
    section?: string;
    external?: () => void;
  }

  const tabs: Tab[] = [
    { id: "home", label: "ホーム", icon: "square.grid.2x2" },
    { id: "mail", label: "メール", icon: "envelope" },
    { id: "timetable", label: "時間割", icon: "calendar", section: "授業" },
    { id: "todo", label: "TODO", icon: "checkmark.circle" },
    { id: "grades", label: "成績照会", icon: "chart.bar" },
    { id: "registration", label: "履修登録", icon: "list.clipboard" },
    { id: "syllabus", label: "シラバス検索", icon: "book" },
    { id: "notifications", label: "お知らせ", icon: "bell", section: "お知らせ" },
    { id: "changes", label: "変更情報", icon: "arrow.triangle.swap" },
    { id: "facility", label: "施設予約", icon: "building.2", section: "ツール", external: () => invoke("open_facility_reservation") },
  ];

  // Track which tabs have been visited (lazy mount: create once, then keep alive)
  let visited = $state(new Set<string>(["home"]));
  $effect(() => {
    const tab = $activeTab;
    if (!visited.has(tab)) {
      visited = new Set([...visited, tab]);
    }
  });

  function badgeCount(tabId: string): number {
    if (tabId === "notifications") return $unreadNotifCount;
    if (tabId === "mail") return $unreadMailCount;
    return 0;
  }

  let unlistenRefresh: (() => void) | null = null;

  // --- Notification badge: compute from cache (works before NotificationsUnified is visited) ---
  let readIds: ReadIdsResponse = { kgc: [], luna: [], kwic: [] };
  function isReadById(source: string, id: string): boolean {
    const ids = readIds[source as keyof ReadIdsResponse];
    return ids ? ids.includes(id) : false;
  }

  function recalcNotifBadge() {
    const kgcItems = getCached<NotificationsData>("notifications")?.entries ?? [];
    const lunaItems = getCached<LunaNotification[]>("luna_updates") ?? [];
    const kwicHome = getCached<KwicPortalHome>("kwic_home");
    let count = 0;
    for (const n of kgcItems) {
      const key = n.id || `${n.title.trim().replace(/\s+/g, "")}|${n.date}`;
      if (!isReadById("kgc", key)) count++;
    }
    for (const n of lunaItems) {
      const key = (n.url || n.idnumber || "") || `${n.content.trim().replace(/\s+/g, "")}|${n.date}`;
      if (!isReadById("luna", key)) count++;
    }
    if (kwicHome) {
      for (const sec of kwicHome.sections) {
        if (sec.title === "メインリンク" || sec.title === "注目コンテンツ") continue;
        for (const item of sec.items) {
          const key = item.id || `${item.title.trim().replace(/\s+/g, "")}|${item.date}`;
          if (!isReadById("kwic", key)) count++;
        }
      }
    }
    unreadNotifCount.set(count);
  }

  const unsubNotif = onCacheUpdate<NotificationsData>("notifications", () => recalcNotifBadge());
  const unsubLuna = onCacheUpdate<LunaNotification[]>("luna_updates", () => recalcNotifBadge());
  const unsubKwicHome = onCacheUpdate<KwicPortalHome>("kwic_home", () => recalcNotifBadge());

  // Keep mail unread count updated from cache (works even before MailView is visited)
  const unsubMail = onCacheUpdate<MailMessage[]>("mail_inbox", (msgs) => {
    if (msgs) unreadMailCount.set(msgs.filter(m => !m.isRead).length);
  });

  // Initialize from cache on first render
  {
    const cached = getCached<MailMessage[]>("mail_inbox");
    if (cached) unreadMailCount.set(cached.filter(m => !m.isRead).length);
    // Load read IDs then compute initial notification badge
    getReadNotifications().then(ids => { readIds = ids; recalcNotifBadge(); }).catch(() => {});
    recalcNotifBadge(); // also compute immediately with empty readIds (all unread)
  }
  onMount(async () => {
    unlistenRefresh = await listen('ai-refresh-request', () => {
      activeTab.set('timetable');
      aiRefreshRequested.set(true);
    });
  });
  onDestroy(() => { if (unlistenRefresh) unlistenRefresh(); unsubMail(); unsubNotif(); unsubLuna(); unsubKwicHome(); });
</script>

<div class="dashboard">
  <nav class="sidebar" data-tauri-drag-region aria-label="メインナビゲーション">
    <div class="sidebar-drag-area" data-tauri-drag-region></div>
    <div class="sidebar-scroll">
      {#each tabs as tab}
        {#if tab.section && tab.section !== (tabs[tabs.indexOf(tab) - 1]?.section ?? "")}
          <div class="section-label">{tab.section}</div>
        {/if}
        <button
          class="nav-item"
          class:active={$activeTab === tab.id}
          aria-current={$activeTab === tab.id ? 'page' : undefined}
          onclick={() => tab.external ? tab.external() : activeTab.set(tab.id)}
        >
          <Icon name={tab.icon} size={16} />
          <span class="nav-label">{tab.label}</span>
          {#if badgeCount(tab.id) > 0}<span class="nav-badge">{badgeCount(tab.id)}</span>{/if}
        </button>
      {/each}
    </div>
  </nav>

  <div class="main-area">
    <Titlebar />
    <div class="content">
      <div class="view-panel" class:active={$activeTab === "home"}>
        <HomePage />
      </div>
      {#if visited.has("mail")}
        <div class="view-panel" class:active={$activeTab === "mail"}>
          <MailView />
        </div>
      {/if}
      {#if visited.has("timetable")}
        <div class="view-panel" class:active={$activeTab === "timetable"}>
          <Timetable />
        </div>
      {/if}
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

  .nav-badge {
    margin-left: auto;
    font-size: 10px;
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 9px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    text-align: center;
    line-height: 16px;
    flex-shrink: 0;
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
