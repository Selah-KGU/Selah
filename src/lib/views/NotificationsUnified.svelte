<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { fetchNotifications, lunaInvoke, kwicFetchHome, kwicOpenDetail, mailFetchInbox, markNotificationRead, getReadNotifications } from "../api";
  import type { MailMessage, ReadIdsResponse } from "../api";
  import { cachedFetch, onCacheUpdate, lunaAuthState, kwicAuthState, mailAuthState, activeTab, unreadNotifCount, unreadMailCount } from "../stores";
  import type { NotificationsData } from "../stores";
  import type { KwicPortalHome } from "../api";
  import { notifyNewKgc, notifyNewLuna, notifyNewKwic, notifyNewMail } from "../notify";
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
  let readIds = $state<ReadIdsResponse>({ kgc: [], luna: [], kwic: [] });
  let readSets = $derived({
    kgc: new Set(readIds.kgc),
    luna: new Set(readIds.luna),
    kwic: new Set(readIds.kwic),
  });

  function notifKey(title: string, date: string): string {
    return `${title.trim().replace(/\s+/g, "")}|${date}`;
  }

  function isNotifRead(n: { source: string; id: string; title: string; date: string }): boolean {
    if (n.source === "mail") return false; // mail has its own read state
    const key = n.id || notifKey(n.title, n.date);
    const set = readSets[n.source as keyof typeof readSets];
    return set ? set.has(key) : false;
  }

  // KWIC detail view state (removed - now opens in window)

  /** Extract all KWIC notification items and push native notifications for new ones */
  function notifyKwicItems(home: KwicPortalHome) {
    const items = home.sections
      .filter(s => s.title !== "メインリンク" && s.title !== "注目コンテンツ")
      .flatMap(s => s.items.map(i => ({
        id: i.id, title: i.title, date: i.date,
        category: i.category || s.title, important: i.important,
      })));
    if (items.length) notifyNewKwic(items);
  }

  // SWR: update UI when background polling brings fresh data
  const unsubKgc = onCacheUpdate<NotificationsData>("notifications", (fresh) => {
    kgcData = fresh;
    if (fresh?.entries) notifyNewKgc(fresh.entries);
  });
  const unsubLuna = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => {
    lunaNotifications = fresh ?? [];
    notifyNewLuna(lunaNotifications);
  });
  const unsubKwicHome = onCacheUpdate<KwicPortalHome>("kwic_home", (fresh) => {
    kwicHome = fresh ?? null;
    if (fresh) notifyKwicItems(fresh);
  });
  const unsubMail = onCacheUpdate<MailMessage[]>("mail_inbox", (fresh) => {
    mailMessages = fresh ?? [];
    notifyNewMail(mailMessages);
  });
  onDestroy(() => { unsubKgc(); unsubLuna(); unsubKwicHome(); unsubMail(); });

  onMount(async () => {
    loading = true;
    try {
      const [kgc, luna, kwic, mail, readResult] = await Promise.allSettled([
        cachedFetch("notifications", fetchNotifications),
        $lunaAuthState.authenticated
          ? cachedFetch("luna_updates", () => lunaInvoke<LunaNotification[]>("luna_fetch_updates"))
          : Promise.resolve([]),
        $kwicAuthState.authenticated
          ? cachedFetch<KwicPortalHome>("kwic_home", kwicFetchHome)
          : Promise.resolve(null),
        $mailAuthState.authenticated
          ? cachedFetch<MailMessage[]>("mail_inbox", () => mailFetchInbox(20, 0))
          : Promise.resolve([]),
        getReadNotifications(),
      ]);
      if (readResult.status === "fulfilled" && readResult.value) {
        readIds = readResult.value as ReadIdsResponse;
      }
      if (kgc.status === "fulfilled" && kgc.value) {
        kgcData = kgc.value as NotificationsData;
        if (kgcData?.entries) notifyNewKgc(kgcData.entries);
      }
      if (luna.status === "fulfilled" && luna.value) {
        lunaNotifications = luna.value as LunaNotification[];
        notifyNewLuna(lunaNotifications);
      }
      if (kwic.status === "fulfilled" && kwic.value) {
        kwicHome = kwic.value as KwicPortalHome;
        notifyKwicItems(kwicHome);
      }
      if (mail.status === "fulfilled" && mail.value) {
        mailMessages = mail.value as MailMessage[];
        notifyNewMail(mailMessages);
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
      const key = `${n.title.trim().replace(/\s+/g, "")}|${n.date}`;
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

  // Sync unread totals to stores (must be in $effect, not $derived)
  $effect(() => {
    let totalNotif = 0;
    let totalMail = 0;
    for (const tab of TAB_ORDER) {
      const items = groupedByTab.get(tab) ?? [];
      for (const n of items) {
        if (isNotifRead(n)) continue;
        if (n.source === "mail") totalMail++;
        else totalNotif++;
      }
    }
    unreadNotifCount.set(totalNotif);
    unreadMailCount.set(totalMail);
  });

  let currentItems = $derived(groupedByTab.get(selectedTab) ?? []);

  async function openNotif(n: UnifiedNotif) {
    // Mark as read locally
    if (n.source !== "mail") {
      const key = n.id || notifKey(n.title, n.date);
      markNotificationRead(n.source, key).catch(console.error);
      // Optimistic update
      readIds = {
        ...readIds,
        [n.source]: [...readIds[n.source as keyof ReadIdsResponse], key],
      };
    }

    if (n.source === "mail") {
      activeTab.set("mail");
    } else if (n.source === "luna" && n.url) {
      try {
        await lunaInvoke("luna_open_detail_window", { path: n.url, title: n.title });
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
  <h2>お知らせ</h2>

  <div class="segmented-control" role="tablist">
    {#each TAB_ORDER as tab}
      <button class="segment" class:active={selectedTab === tab} role="tab" aria-selected={selectedTab === tab} onclick={() => { selectedTab = tab; }}>
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

  <ViewLoader {loading} {error} empty={currentItems.length === 0 && !loading} emptyMessage="お知らせはありません">
      <div class="notif-list">
        {#each currentItems as n, i}
          <button
            class="notif-item"
            class:clickable={n.source === "luna" || n.source === "kwic" || n.source === "mail"}
            class:read={isNotifRead(n)}
            style="animation: notif-enter 0.3s ease {Math.min(i * 0.04, 0.4)}s both;"
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
  .state-msg {
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
    padding: 40px 0;
  }

  /* KGC Notifications */
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
</style>
