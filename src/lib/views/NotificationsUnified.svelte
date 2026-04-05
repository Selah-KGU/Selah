<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { fetchNotifications, lunaInvoke } from "../api";
  import { cachedFetch, onCacheUpdate, lunaAuthState } from "../stores";
  import type { NotificationsData } from "../stores";
  import { notifyNewKwic, notifyNewLuna } from "../notify";
  import ViewLoader from "../ViewLoader.svelte";
  import Icon from "../Icon.svelte";

  interface LunaNotification {
    date: string;
    course_info: string;
    module: string;
    content: string;
    url: string;
    idnumber: string;
  }

  let activeTab = $state<"kwic" | "luna">("kwic");
  let kwicLoading = $state(true);
  let lunaLoading = $state(false);
  let kwicError = $state("");
  let lunaError = $state("");
  let kwicData = $state<NotificationsData | null>(null);
  let lunaNotifications = $state<LunaNotification[]>([]);

  // SWR: update UI when background polling brings fresh data
  const unsubKwic = onCacheUpdate<NotificationsData>("notifications", (fresh) => {
    kwicData = fresh;
    if (fresh?.entries) notifyNewKwic(fresh.entries);
  });
  const unsubLuna = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => {
    lunaNotifications = fresh ?? [];
    notifyNewLuna(lunaNotifications);
  });
  onDestroy(() => { unsubKwic(); unsubLuna(); });

  onMount(async () => {
    // Load KWIC notifications immediately
    try {
      kwicData = await cachedFetch("notifications", fetchNotifications);
      if (kwicData?.entries) notifyNewKwic(kwicData.entries);
    } catch (e: any) {
      kwicError = e?.message || String(e);
    } finally {
      kwicLoading = false;
    }

    // Load Luna notifications if authenticated
    if ($lunaAuthState.authenticated) {
      loadLunaNotifications();
    }
  });

  async function loadLunaNotifications() {
    lunaLoading = true;
    lunaError = "";
    try {
      lunaNotifications = await cachedFetch("luna_updates", () => lunaInvoke<LunaNotification[]>("luna_fetch_updates")) ?? [];
      notifyNewLuna(lunaNotifications);
    } catch (e: any) {
      lunaError = String(e);
    }
    lunaLoading = false;
  }

  async function openLunaDetail(path: string, title: string) {
    if (!path) return;
    try {
      await lunaInvoke("luna_open_detail_window", { path, title });
    } catch (e: any) {
      console.error("Failed to open detail window:", e);
    }
  }

  let kwicCount = $derived(kwicData?.entries?.length ?? 0);
  let lunaCount = $derived((lunaNotifications ?? []).length);
</script>

<div class="view">
  <h2>お知らせ</h2>

  <div class="segmented-control">
    <button class="segment" class:active={activeTab === "kwic"} onclick={() => activeTab = "kwic"}>
      <Icon name="globe" size={13} />
      KWIC
      {#if kwicCount > 0}<span class="count-badge">{kwicCount}</span>{/if}
    </button>
    <button class="segment" class:active={activeTab === "luna"} onclick={() => activeTab = "luna"}>
      <Icon name="moon.stars" size={13} />
      Luna 更新
      {#if lunaCount > 0}<span class="count-badge">{lunaCount}</span>{/if}
    </button>
  </div>

  {#if activeTab === "kwic"}
    <ViewLoader loading={kwicLoading} error={kwicError} empty={kwicData?.entries.length === 0} emptyMessage="お知らせはありません">
      {#if kwicData}
        <div class="notif-list">
          {#each kwicData.entries as n, i}
            <div class="notif-item" style="animation: slide-up 0.3s ease {Math.min(i * 0.04, 0.4)}s both;">
              <div class="notif-header">
                {#if n.category}
                  <span class="notif-badge">{n.category}</span>
                {/if}
                <span class="notif-title">{n.title}</span>
                <span class="notif-date">{n.date}</span>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </ViewLoader>

  {:else}
    <ViewLoader loading={lunaLoading} error={lunaError} empty={lunaNotifications.length === 0 && !lunaLoading} emptyMessage="更新通知はありません">
      {#if !$lunaAuthState.authenticated && lunaNotifications.length === 0 && !lunaLoading}
        <div class="state-msg">Luna LMSに接続されていません</div>
      {:else}
        <div class="update-list">
          {#each lunaNotifications as n, i}
            <button
              class="update-card"
              class:has-link={!!n.url}
              style="animation: slide-up 0.3s ease {Math.min(i * 0.04, 0.4)}s both;"
              onclick={() => openLunaDetail(n.url, n.content || n.module)}
              disabled={!n.url}
            >
              <div class="update-header">
                {#if n.module}
                  <span class="update-badge">{n.module}</span>
                {/if}
                <span class="update-text">{n.content}</span>
                <span class="update-date">{n.date}</span>
              </div>
              <div class="update-course">{n.course_info}</div>
            </button>
          {/each}
        </div>
      {/if}
    </ViewLoader>
  {/if}
</div>

<style>
  .segmented-control {
    display: flex;
    background: var(--bg-tertiary);
    border-radius: 8px;
    padding: 2px;
    margin-bottom: 12px;
    gap: 2px;
  }
  .segment {
    flex: 1;
    padding: 6px 10px;
    border: none;
    background: none;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
  }
  .segment:hover { color: var(--text-primary); }
  .segment.active {
    background: var(--bg-card);
    color: var(--text-primary);
    font-weight: 600;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08);
  }
  .count-badge {
    font-size: 10px;
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 9px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    text-align: center;
  }
  .state-msg {
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
    padding: 40px 0;
  }

  /* KWIC Notifications */
  .notif-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .notif-item {
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    border-radius: 10px;
    padding: 12px 16px;
    box-shadow: var(--shadow-sm);
    transition: background 0.15s ease, transform 0.15s ease, box-shadow 0.15s ease;
  }
  .notif-item:hover {
    background: var(--bg-hover);
    transform: translateX(2px);
    box-shadow: var(--shadow-md);
  }
  .notif-item:active {
    transform: scale(0.995);
  }
  .notif-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .notif-badge {
    font-size: 10px;
    padding: 2px 8px;
    background: var(--accent-light);
    color: var(--accent);
    border-radius: 6px;
    font-weight: 600;
    white-space: nowrap;
    letter-spacing: 0.3px;
  }
  .notif-title {
    flex: 1;
    font-weight: 500;
    font-size: 13px;
    color: var(--text-primary);
  }
  .notif-date {
    font-size: 12px;
    color: var(--text-tertiary);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  /* Luna Update Notifications */
  .update-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .update-card {
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    border-radius: 10px;
    padding: 12px 16px;
    box-shadow: var(--shadow-sm);
    transition: background 0.15s ease, transform 0.15s ease, box-shadow 0.15s ease;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    width: 100%;
  }
  .update-card:disabled {
    cursor: default;
    opacity: 0.7;
  }
  .update-card:disabled:hover {
    background: var(--bg-card);
    transform: none;
    box-shadow: var(--shadow-sm);
  }
  .update-card:hover {
    background: var(--bg-hover);
    transform: translateX(2px);
    box-shadow: var(--shadow-md);
  }
  .update-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .update-badge {
    font-size: 10px;
    padding: 2px 8px;
    background: var(--accent-light);
    color: var(--accent);
    border-radius: 6px;
    font-weight: 600;
    white-space: nowrap;
    letter-spacing: 0.3px;
  }
  .update-text {
    flex: 1;
    font-weight: 500;
    font-size: 13px;
    color: var(--text-primary);
  }
  .update-date {
    font-size: 12px;
    color: var(--text-tertiary);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .update-course {
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-tertiary);
  }
</style>
