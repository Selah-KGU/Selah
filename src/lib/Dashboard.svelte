<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { activeTab, aiRefreshRequested, unreadNotifCount, unreadMailCount, readIdsStore, onCacheUpdate, getCached, loadReadIds, notifKey } from "./stores";
  import { get } from "svelte/store";
  import type { NotificationsData } from "./stores";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./Icon.svelte";
  import Titlebar from "./Titlebar.svelte";
  import HomePage from "./views/HomePage.svelte";
  import type { MailMessage, KwicPortalHome } from "./api";
  import { updateAiReadiness } from "./api";
  import type { LunaNotification } from "./types";

  interface Tab {
    id: string;
    label: string;
    icon: IconName;
    section?: string;
    external?: () => void;
  }

  const tabs: Tab[] = [
    { id: "home", label: "ホーム", icon: "house" },
    { id: "mail", label: "メール", icon: "envelope" },
    { id: "ict-tools", label: "ツール", icon: "square.grid.2x2" },
    { id: "timetable", label: "時間割", icon: "calendar", section: "授業" },
    { id: "live", label: "LIVE", icon: "broadcast" },
    { id: "todo", label: "TODO", icon: "checkmark.circle" },
    { id: "grades", label: "成績照会", icon: "chart.bar" },
    { id: "registration", label: "履修登録", icon: "list.clipboard" },
    { id: "syllabus", label: "シラバス検索", icon: "book" },
    { id: "notifications", label: "お知らせ", icon: "bell", section: "インフォ" },
    { id: "changes", label: "変更情報", icon: "arrow.triangle.swap" },
  ];

  const viewLoaders: Record<string, () => Promise<{ default: any }>> = {
    mail: () => import("./views/MailView.svelte"),
    "ict-tools": () => import("./views/IctTools.svelte"),
    timetable: () => import("./views/Timetable.svelte"),
    live: () => import("./views/Live.svelte"),
    todo: () => import("./views/LunaTodo.svelte"),
    grades: () => import("./views/GradesView.svelte"),
    registration: () => import("./views/Registration.svelte"),
    syllabus: () => import("./views/Syllabus.svelte"),
    notifications: () => import("./views/NotificationsUnified.svelte"),
    changes: () => import("./views/ChangeInfo.svelte"),
    agent: () => import("./views/AgentChat.svelte"),
    settings: () => import("./views/Settings.svelte"),
  };

  // Track which tabs have been visited (lazy mount: create once, then keep alive)
  let visited = $state(new Set<string>(["home"]));
  let lazyViews = $state<Record<string, any>>({});
  let lazyErrors = $state<Record<string, string>>({});
  const loadingViews = new Set<string>();

  async function ensureViewLoaded(tab: string) {
    if (tab === "home" || lazyViews[tab] || loadingViews.has(tab) || !viewLoaders[tab]) return;
    loadingViews.add(tab);
    try {
      const mod = await viewLoaders[tab]();
      lazyViews = { ...lazyViews, [tab]: mod.default };
      if (lazyErrors[tab]) {
        const next = { ...lazyErrors };
        delete next[tab];
        lazyErrors = next;
      }
    } catch (e) {
      lazyErrors = { ...lazyErrors, [tab]: e instanceof Error ? e.message : String(e) };
    } finally {
      loadingViews.delete(tab);
    }
  }

  $effect(() => {
    const tab = $activeTab;
    if (!visited.has(tab)) {
      visited = new Set([...visited, tab]);
    }
    void ensureViewLoaded(tab);
  });

  function badgeCount(tabId: string): number {
    if (tabId === "notifications") return $unreadNotifCount;
    if (tabId === "mail") return $unreadMailCount;
    return 0;
  }

  let unlistenRefresh: (() => void) | null = null;

  // --- Notification badge: compute from cache (works before NotificationsUnified is visited) ---

  function recalcNotifBadge() {
    const kgcItems = getCached<NotificationsData>("notifications")?.entries ?? [];
    const lunaItems = getCached<LunaNotification[]>("luna_updates") ?? [];
    const kwicHome = getCached<KwicPortalHome>("kwic_home");
    const readIds = get(readIdsStore);
    const kgcRead = new Set(readIds.kgc);
    const lunaRead = new Set(readIds.luna);
    const kwicRead = new Set(readIds.kwic);
    let count = 0;
    for (const n of kgcItems) {
      const readKey = n.id || notifKey(n.title, n.date);
      if (!kgcRead.has(readKey)) count++;
    }
    for (const n of lunaItems) {
      const readKey = (n.url || n.idnumber || "") || notifKey(n.content, n.date);
      if (!lunaRead.has(readKey)) count++;
    }
    if (kwicHome) {
      for (const sec of kwicHome.sections) {
        if (sec.title === "メインリンク" || sec.title === "注目コンテンツ" || sec.title === "授業のお知らせ") continue;
        for (const item of sec.items) {
          const readKey = item.id || notifKey(item.title, item.date);
          if (!kwicRead.has(readKey)) count++;
        }
      }
    }
    unreadNotifCount.set(count);
  }

  const unsubNotif = onCacheUpdate<NotificationsData>("notifications", (fresh) => {
    recalcNotifBadge();
  });
  const unsubLuna = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => {
    recalcNotifBadge();
  });
  const unsubKwicHome = onCacheUpdate<KwicPortalHome>("kwic_home", (fresh) => {
    recalcNotifBadge();
  });
  // Recalc when read IDs change (e.g. user marks notification read in NotificationsUnified)
  $effect(() => { $readIdsStore; recalcNotifBadge(); });

  // Keep mail unread count updated from cache (works even before MailView is visited)
  const unsubMail = onCacheUpdate<MailMessage[]>("mail_inbox", (msgs) => {
    if (msgs) {
      unreadMailCount.set(msgs.filter(m => !m.isRead).length);
    }
  });

  // Initialize from cache on first render
  {
    const cached = getCached<MailMessage[]>("mail_inbox");
    if (cached) unreadMailCount.set(cached.filter(m => !m.isRead).length);
    // Load read IDs from DB then compute initial notification badge
    loadReadIds().catch(() => {}).finally(() => recalcNotifBadge());
  }
  onMount(async () => {
    unlistenRefresh = await listen('ai-refresh-request', () => {
      activeTab.set('timetable');
      aiRefreshRequested.set(true);
    });
    const unlistenTrayTab = await listen<string>('tray-open-tab', (event) => {
      if (event.payload) activeTab.set(event.payload);
    });
    const unlistenOpenAgent = await listen("open-agent-tab", () => {
      activeTab.set("agent");
    });
    // Initialize AI readiness stores
    updateAiReadiness().catch(() => {});
    // Re-check when AI config changes (e.g. user edits settings)
    const unlistenAiCfg = await listen('ai-config-changed', () => {
      updateAiReadiness().catch(() => {});
    });
    const _prevDestroy = unlistenRefresh;
    unlistenRefresh = () => { _prevDestroy?.(); unlistenTrayTab(); unlistenOpenAgent(); unlistenAiCfg(); };
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
        {@const MailView = lazyViews.mail}
        <div class="view-panel" class:active={$activeTab === "mail"}>
          {#if MailView}<MailView />{:else}<div class="lazy-view-status">{lazyErrors.mail || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("ict-tools")}
        {@const IctTools = lazyViews["ict-tools"]}
        <div class="view-panel" class:active={$activeTab === "ict-tools"}>
          {#if IctTools}<IctTools />{:else}<div class="lazy-view-status">{lazyErrors["ict-tools"] || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("timetable")}
        {@const Timetable = lazyViews.timetable}
        <div class="view-panel" class:active={$activeTab === "timetable"}>
          {#if Timetable}<Timetable />{:else}<div class="lazy-view-status">{lazyErrors.timetable || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("live")}
        {@const Live = lazyViews.live}
        <div class="view-panel" class:active={$activeTab === "live"}>
          {#if Live}<Live />{:else}<div class="lazy-view-status">{lazyErrors.live || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("todo")}
        {@const LunaTodo = lazyViews.todo}
        <div class="view-panel" class:active={$activeTab === "todo"}>
          {#if LunaTodo}<LunaTodo />{:else}<div class="lazy-view-status">{lazyErrors.todo || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("grades")}
        {@const GradesView = lazyViews.grades}
        <div class="view-panel" class:active={$activeTab === "grades"}>
          {#if GradesView}<GradesView />{:else}<div class="lazy-view-status">{lazyErrors.grades || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("registration")}
        {@const Registration = lazyViews.registration}
        <div class="view-panel" class:active={$activeTab === "registration"}>
          {#if Registration}<Registration />{:else}<div class="lazy-view-status">{lazyErrors.registration || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("syllabus")}
        {@const Syllabus = lazyViews.syllabus}
        <div class="view-panel" class:active={$activeTab === "syllabus"}>
          {#if Syllabus}<Syllabus />{:else}<div class="lazy-view-status">{lazyErrors.syllabus || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("notifications")}
        {@const NotificationsUnified = lazyViews.notifications}
        <div class="view-panel" class:active={$activeTab === "notifications"}>
          {#if NotificationsUnified}<NotificationsUnified />{:else}<div class="lazy-view-status">{lazyErrors.notifications || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("changes")}
        {@const ChangeInfo = lazyViews.changes}
        <div class="view-panel" class:active={$activeTab === "changes"}>
          {#if ChangeInfo}<ChangeInfo />{:else}<div class="lazy-view-status">{lazyErrors.changes || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("agent")}
        {@const AgentChat = lazyViews.agent}
        <div class="view-panel" class:active={$activeTab === "agent"}>
          {#if AgentChat}<AgentChat />{:else}<div class="lazy-view-status">{lazyErrors.agent || "読み込み中…"}</div>{/if}
        </div>
      {/if}
      {#if visited.has("settings")}
        {@const Settings = lazyViews.settings}
        <div class="view-panel" class:active={$activeTab === "settings"}>
          {#if Settings}<Settings />{:else}<div class="lazy-view-status">{lazyErrors.settings || "読み込み中…"}</div>{/if}
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

  :global(body.platform-windows) .sidebar {
    padding-top: 10px;
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

  :global(body.platform-windows) .sidebar-drag-area {
    height: 6px;
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

  .lazy-view-status {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-tertiary);
    font-size: 13px;
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
