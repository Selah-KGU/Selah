<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { kwicOpenDetail, lunaInvoke } from "../api";
  import type { MailMessage } from "../api";
  import { cachedBackendFetch, getCached, onCacheUpdate, lunaAuthState, kwicAuthState, mailAuthState, activeTab, readIdsStore, notifKey, markRead, markBatchRead } from "../stores";
  import type { NotificationsData } from "../stores";
  import type { KwicPortalHome } from "../api";
  import ViewLoader from "../ViewLoader.svelte";
  import type { LunaNotification } from "../types";

  // Unified notification item for all sources
  interface UnifiedNotif {
    id: string;
    title: string;
    date: string;
    category: string;      // e.g. 学部名, module name
    tab: string;            // one of the 4 KWIC-style tabs
    source: "kgc" | "luna" | "kwic" | "mail";
    important: boolean;
    url?: string;
    courseInfo?: string;
    // KWIC detail params
    informationType?: string;
    personCategoryCd?: string;
    categoryCd?: string;
  }

  const TAB_ORDER = [
    "呼出し・重要なお知らせ",
    "学部・研究科からのお知らせ",
    "授業のお知らせ",
    "その他",
  ] as const;
  type TabName = typeof TAB_ORDER[number];

  let selectedTab = $state<TabName>("授業のお知らせ");
  let loading = $state(true);
  let error = $state("");
  let kgcData = $state<NotificationsData | null>(null);
  let lunaNotifications = $state<LunaNotification[]>([]);
  let kwicHome = $state<KwicPortalHome | null>(null);
  let mailMessages = $state<MailMessage[]>([]);
  let readSets = $derived({
    kgc: new Set($readIdsStore.kgc),
    luna: new Set($readIdsStore.luna),
    kwic: new Set($readIdsStore.kwic),
  });

  function isNotifRead(n: { source: string; id: string; title: string; date: string }): boolean {
    if (n.source === "mail") return false; // mail has its own read state
    const key = n.id || notifKey(n.title, n.date);
    const set = readSets[n.source as keyof typeof readSets];
    return set ? set.has(key) : false;
  }

  // KWIC detail view state (removed - now opens in window)

  /** Extract all KWIC notification items for native push (handled by Dashboard now) */

  // SWR: update UI when background polling brings fresh data
  const unsubKgc = onCacheUpdate<NotificationsData>("notifications", (fresh) => {
    kgcData = fresh;
  });
  const unsubLuna = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => {
    lunaNotifications = fresh ?? [];
  });
  const unsubKwicHome = onCacheUpdate<KwicPortalHome>("kwic_home", (fresh) => {
    kwicHome = fresh ?? null;
  });
  const unsubMail = onCacheUpdate<MailMessage[]>("mail_inbox", (fresh) => {
    mailMessages = fresh ?? [];
  });
  onDestroy(() => { unsubKgc(); unsubLuna(); unsubKwicHome(); unsubMail(); });

  onMount(async () => {
    // Restore cached data immediately so UI is never blank
    const cachedKgc = getCached<NotificationsData>("notifications");
    const cachedLuna = getCached<LunaNotification[]>("luna_updates");
    const cachedKwic = getCached<KwicPortalHome>("kwic_home");
    const cachedMail = getCached<MailMessage[]>("mail_inbox");
    if (cachedKgc) kgcData = cachedKgc;
    if (cachedLuna) lunaNotifications = cachedLuna;
    if (cachedKwic) kwicHome = cachedKwic;
    if (cachedMail) mailMessages = cachedMail;
    if (cachedKgc || cachedLuna || cachedKwic || cachedMail) loading = false;

    try {
      const [kgc, luna, kwic, mail] = await Promise.allSettled([
        cachedBackendFetch("notifications"),
        $lunaAuthState.authenticated
          ? cachedBackendFetch("luna_updates")
          : Promise.resolve([]),
        $kwicAuthState.authenticated
          ? cachedBackendFetch<KwicPortalHome>("kwic_home")
          : Promise.resolve(null),
        $mailAuthState.authenticated
          ? cachedBackendFetch<MailMessage[]>("mail_inbox")
          : Promise.resolve([]),
      ]);
      if (kgc.status === "fulfilled" && kgc.value) {
        kgcData = kgc.value as NotificationsData;
      }
      if (luna.status === "fulfilled" && luna.value) {
        lunaNotifications = luna.value as LunaNotification[];
      }
      if (kwic.status === "fulfilled" && kwic.value) {
        kwicHome = kwic.value as KwicPortalHome;
      }
      if (mail.status === "fulfilled" && mail.value) {
        mailMessages = mail.value as MailMessage[];
      }
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  });

  // Build unified + categorized notifications
  let allNotifs = $derived.by(() => {
    const items: UnifiedNotif[] = [];
    const seen = new Set<string>();
    const addUniq = (n: UnifiedNotif) => {
      const key = `${n.source}:${n.id}:${n.title}:${n.date}:${n.category}`;
      if (seen.has(key)) return;
      seen.add(key);
      items.push(n);
    };

    // KGC → 授業のお知らせ
    if (kgcData?.entries) {
      for (const n of kgcData.entries) {
        addUniq({
          id: n.id, title: n.title, date: n.date, category: n.category,
          tab: "授業のお知らせ", source: "kgc", important: false,
        });
      }
    }

    // Luna → 授業のお知らせ
    for (const n of lunaNotifications) {
      addUniq({
        id: n.url || n.idnumber || "", title: n.content, date: n.date, category: n.module || n.course_info,
        tab: "授業のお知らせ", source: "luna", important: false,
        url: n.url, courseInfo: n.course_info,
      });
    }

    // Mail → その他
    for (const m of mailMessages) {
      if (!m.isRead) {
        const sender = m.from?.emailAddress?.name || m.from?.emailAddress?.address || "不明";
        addUniq({
          id: m.id,
          title: m.subject || "(件名なし)",
          date: m.receivedDateTime ? new Date(m.receivedDateTime).toLocaleDateString("ja-JP", { month: "numeric", day: "numeric" }) : "",
          category: sender,
          tab: "その他",
          source: "mail",
          important: false,
        });
      }
    }

    // KWIC sections → map by section title (exclude 授業のお知らせ, use KGC/Luna for that)
    if (kwicHome) {
      const kwicTabMap: Record<string, TabName> = {
        "呼出し・重要なお知らせ": "呼出し・重要なお知らせ",
        "学部・研究科からのお知らせ": "学部・研究科からのお知らせ",
        "その他": "その他",
      };
      for (const sec of kwicHome.sections) {
        const tab = kwicTabMap[sec.title];
        if (!tab) continue; // skip メインリンク, 注目コンテンツ etc.
        for (const item of sec.items) {
          addUniq({
            id: item.id, title: item.title, date: item.date, category: item.category || sec.title,
            tab, source: "kwic", important: item.important,
            informationType: item.information_type,
            personCategoryCd: item.person_category_cd,
            categoryCd: item.category_cd,
          });
        }
      }
    }

    return items;
  });

  // Group by tab
  let groupedByTab = $derived.by(() => {
    const map = new Map<TabName, UnifiedNotif[]>();
    for (const tab of TAB_ORDER) map.set(tab, []);
    for (const n of allNotifs) {
      map.get(n.tab as TabName)?.push(n);
    }
    // Sort each group by date descending
    for (const [, items] of map) {
      items.sort((a, b) => {
        const da = new Date(a.date.replace(/\//g, "-")).getTime() || 0;
        const db = new Date(b.date.replace(/\//g, "-")).getTime() || 0;
        return db - da;
      });
    }
    return map;
  });

  let tabCounts = $derived.by(() => {
    const counts: Record<string, number> = {};
    for (const tab of TAB_ORDER) {
      const items = groupedByTab.get(tab) ?? [];
      counts[tab] = items.filter(n => !isNotifRead(n)).length;
    }
    return counts;
  });

  let currentItems = $derived(groupedByTab.get(selectedTab) ?? []);

  // Course filter for class-tab (授業のお知らせ)
  let selectedCourse = $state("all");
  let classCourses = $derived.by(() => {
    const items = groupedByTab.get("授業のお知らせ") ?? [];
    const counts = new Map<string, number>();
    for (const n of items) {
      const key = n.courseInfo || n.category || "";
      if (key) counts.set(key, (counts.get(key) || 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]);
  });
  let filteredItems = $derived.by(() => {
    if (selectedTab !== "授業のお知らせ" || selectedCourse === "all") return currentItems;
    return currentItems.filter(n => (n.courseInfo || n.category || "") === selectedCourse);
  });
  // Cap rendered DOM nodes — older entries reachable via "show more"
  // pagination instead of mounting hundreds of buttons up front.
  const NOTIF_PAGE_SIZE = 50;
  let notifVisibleCount = $state(NOTIF_PAGE_SIZE);
  $effect(() => {
    selectedTab; selectedCourse;
    notifVisibleCount = NOTIF_PAGE_SIZE;
  });
  let visibleNotifs = $derived(filteredItems.slice(0, notifVisibleCount));

  async function markAllRead() {
    const items = filteredItems.filter(n => n.source !== "mail" && !isNotifRead(n));
    if (items.length === 0) return;
    // Group by source
    const bySource = new Map<string, string[]>();
    for (const n of items) {
      const key = n.id || notifKey(n.title, n.date);
      const list = bySource.get(n.source) ?? [];
      list.push(key);
      bySource.set(n.source, list);
    }
    // DB-first: await each write, store auto-updates on success
    for (const [source, ids] of bySource) {
      await markBatchRead(source, ids).catch(console.error);
    }
  }

  async function openNotif(n: UnifiedNotif) {
    // Mark as read (DB-first)
    if (n.source !== "mail") {
      const key = n.id || notifKey(n.title, n.date);
      markRead(n.source, key).catch(console.error);
    }

    if (n.source === "mail") {
      activeTab.set("mail");
    } else if (n.source === "luna" && n.url) {
      try {
        await lunaInvoke("luna_open_detail_window", { path: n.url, title: n.title, courseName: n.courseInfo || null });
      } catch (e) { console.error("Failed to open Luna detail:", e); }
    } else if (n.source === "kwic" && n.id) {
      try {
        await kwicOpenDetail({
          id: n.id,
          title: n.title,
          information_type: n.informationType || "",
          person_category_cd: n.personCategoryCd || "",
          category_cd: n.categoryCd || "",
        });
      } catch (e) { console.error("Failed to open KWIC detail:", e); }
    }
  }
</script>

<div class="view">
  <div class="title-row">
    <h2>お知らせ</h2>
    <button class="mark-all-btn" onclick={markAllRead} disabled={filteredItems.filter(n => n.source !== "mail" && !isNotifRead(n)).length === 0}>
      この分類を既読
    </button>
  </div>

  <div class="segmented-control" role="tablist">
    {#each TAB_ORDER as tab}
      <button class="segment" class:active={selectedTab === tab} role="tab" aria-selected={selectedTab === tab} onclick={() => { selectedTab = tab; selectedCourse = "all"; }}>
        {#if tab === "呼出し・重要なお知らせ"}
          重要
        {:else if tab === "学部・研究科からのお知らせ"}
          学部
        {:else if tab === "授業のお知らせ"}
          授業
        {:else}
          その他
        {/if}
        {#if tabCounts[tab] > 0}<span class="count-badge">{tabCounts[tab]}</span>{/if}
      </button>
    {/each}
  </div>

  {#if selectedTab === "授業のお知らせ" && classCourses.length > 1}
    <div class="filters">
      <button class="chip" class:active={selectedCourse === "all"} onclick={() => selectedCourse = "all"}>
        すべて
      </button>
      {#each classCourses as [course, count]}
        <button class="chip" class:active={selectedCourse === course} onclick={() => selectedCourse = course}>
          {course} <span class="chip-count">{count}</span>
        </button>
      {/each}
    </div>
  {/if}

  <ViewLoader {loading} {error} empty={filteredItems.length === 0 && !loading} emptyMessage="お知らせはありません">
    <div class="notif-list">
      {#each visibleNotifs as n}
        <button
          class="notif-item"
          class:clickable={n.source === "luna" || n.source === "kwic" || n.source === "mail"}
          class:read={isNotifRead(n)}
          onclick={() => openNotif(n)}
        >
          <div class="notif-header">
            {#if n.category}
              <span class="notif-badge" class:badge-kwic={n.source === "kwic"} class:badge-luna={n.source === "luna"} class:badge-mail={n.source === "mail"}>{n.category}</span>
            {/if}
            <span class="notif-title">{n.title}</span>
            <span class="notif-date">{n.date}</span>
          </div>
          {#if n.courseInfo}
            <div class="notif-course">{n.courseInfo}</div>
          {/if}
        </button>
      {/each}
      {#if filteredItems.length > notifVisibleCount}
        <button class="notif-more" onclick={() => notifVisibleCount += NOTIF_PAGE_SIZE}>
          もっと見る ({filteredItems.length - notifVisibleCount} 件)
        </button>
      {/if}
    </div>
  </ViewLoader>
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
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 12px;
  }
  .title-row h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .mark-all-btn {
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    font-size: 11px;
    padding: 5px 10px;
    border-radius: 7px;
    cursor: pointer;
    font-family: inherit;
    transition: background 0.15s, color 0.15s, opacity 0.15s;
  }
  .mark-all-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .mark-all-btn:disabled { opacity: 0.45; cursor: default; }

  /* KGC Notifications */
  .notif-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .notif-more {
    margin-top: 8px;
    padding: 10px 14px;
    border-radius: 10px;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
    transition: background 0.15s, color 0.15s;
  }
  .notif-more:hover { background: var(--bg-hover); color: var(--text-primary); }
  .notif-item {
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    border-radius: 10px;
    padding: 12px 16px;
    box-shadow: var(--shadow-sm);
    transition: background 0.15s ease, transform 0.15s ease, box-shadow 0.15s ease;
    font-family: inherit;
    color: inherit;
    text-align: left;
    width: 100%;
  }
  .notif-item.clickable {
    cursor: pointer;
  }
  .notif-item.read {
    opacity: 0.55;
  }
  .notif-item.clickable:hover {
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
  .badge-kwic {
    background: rgba(124, 58, 237, 0.1);
    color: #7c3aed;
  }
  .badge-luna {
    background: rgba(245, 158, 11, 0.1);
    color: #d97706;
  }
  .badge-mail {
    background: rgba(0, 122, 255, 0.1);
    color: #0066cc;
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

  .notif-course {
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-tertiary);
  }
  @keyframes notif-enter {
    from { transform: translateY(12px); }
    to { transform: translateY(0); }
  }

  /* ── Course filter chips ── */
  .filters {
    display: flex;
    gap: 5px;
    overflow-x: auto;
    margin-bottom: 12px;
    scrollbar-width: none;
    padding-bottom: 2px;
    cursor: grab;
  }
  .filters:active { cursor: grabbing; }
  .filters::-webkit-scrollbar { display: none; }
  .chip {
    flex-shrink: 0;
    padding: 5px 14px;
    border-radius: 16px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    transition: all 0.2s cubic-bezier(0.2, 0.8, 0.2, 1);
    white-space: nowrap;
  }
  .chip:hover { background: var(--bg-hover); }
  .chip.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
    box-shadow: 0 1px 6px rgba(0, 40, 85, 0.2);
  }
  .chip-count {
    font-size: 10px;
    font-weight: 600;
    opacity: 0.6;
    margin-left: 2px;
  }
  .chip.active .chip-count {
    opacity: 0.8;
  }
</style>
