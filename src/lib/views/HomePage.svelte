<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { authState, lunaAuthState, activeTab, cachedFetch, onCacheUpdate } from "../stores";
  import type { TimetableData, TimetableEntry, NotificationsData, NotificationEntry } from "../stores";
  import { fetchTimetable, fetchNotifications, lunaInvoke } from "../api";
  import { invoke } from "@tauri-apps/api/core";

  // ============ Types ============

  interface LunaTodoItem {
    course_name: string;
    content_type: string;
    content_name: string;
    url: string;
    deadline: string;
    status: string;
    feedback: string;
  }

  interface LunaNotification {
    date: string;
    course_info: string;
    module: string;
    content: string;
    url: string;
    idnumber: string;
  }

  interface UnifiedNotif {
    source: "kwic" | "luna";
    title: string;
    category: string;
    date: string;
    url?: string;
  }

  interface LunaCourse {
    idnumber: string;
    name: string;
    teacher: string;
    period: number;
    day: number;
  }

  interface LunaTimetable {
    courses: LunaCourse[];
  }

  // ============ Period Times ============

  const PERIOD_TIMES: Record<number, { start: string; end: string; startH: number; startM: number; endH: number; endM: number }> = {
    1: { start: "9:00",  end: "10:30", startH: 9, startM: 0,  endH: 10, endM: 30 },
    2: { start: "11:00", end: "12:30", startH: 11, startM: 0,  endH: 12, endM: 30 },
    3: { start: "13:30", end: "15:00", startH: 13, startM: 30, endH: 15, endM: 0 },
    4: { start: "15:10", end: "16:40", startH: 15, startM: 10, endH: 16, endM: 40 },
    5: { start: "16:50", end: "18:20", startH: 16, startM: 50, endH: 18, endM: 20 },
  };

  const DAY_LABELS = ["日", "月", "火", "水", "木", "金", "土"];

  // ============ State ============

  let timetableData = $state<TimetableData | null>(null);
  let lunaTimetable = $state<LunaTimetable | null>(null);
  let todoItems = $state<LunaTodoItem[]>([]);
  let kwicNotifs = $state<NotificationEntry[]>([]);
  let lunaNotifs = $state<LunaNotification[]>([]);
  let now = $state(new Date());
  let loading = $state(true);

  // ============ Derived ============

  const GREETINGS: Record<string, string[]> = {
    night: [
      "おやすみなさい", "夜更かしはほどほどに", "今日もおつかれ",
      "もう寝た方がいいよ", "静かな夜だね", "明日に備えよう",
      "夜風が気持ちいいね", "星がきれいだよ", "いい夢見てね",
      "そろそろ休もう",
    ],
    morning: [
      "おはよう", "いい朝だね", "今日もがんばろう",
      "すっきり起きれた？", "朝ごはん食べた？", "いい一日にしよう",
      "目覚めはどう？", "今日は何する？", "新しい一日だね",
      "コーヒーでも飲もう", "早起きえらい", "空気がおいしいね",
    ],
    day: [
      "こんにちは", "調子はどう？", "いい天気だね",
      "お昼食べた？", "午後もがんばろう", "ちょっと休憩しよう",
      "散歩でもどう？", "眠くない？", "水分とった？",
      "いい感じだね", "順調？", "ファイト",
    ],
    evening: [
      "おつかれさま", "今日もよくやったね", "ゆっくり休んでね",
      "一日おつかれ", "お腹すいた？", "今日はどうだった？",
      "夕焼けきれいだよ", "もうひと息", "リラックスしよう",
      "自分を褒めよう", "帰り道気をつけて", "明日も楽しみだね",
    ],
  };

  // Pick a greeting that stays stable per calendar day
  let greeting = $derived.by(() => {
    const h = now.getHours();
    const slot = h < 5 ? "night" : h < 11 ? "morning" : h < 17 ? "day" : "evening";
    const pool = GREETINGS[slot];
    const daySeed = now.getFullYear() * 400 + now.getMonth() * 32 + now.getDate();
    return pool[daySeed % pool.length];
  });

  let dateLabel = $derived.by(() => {
    const m = now.getMonth() + 1;
    const d = now.getDate();
    const dayStr = DAY_LABELS[now.getDay()];
    return `${m}月${d}日（${dayStr}）`;
  });

  let todaySummary = $derived.by(() => {
    if (!timetableData?.entries.length) return null;
    const todayDay = DAY_LABELS[now.getDay()];
    const classes = timetableData.entries.filter(e => e.day === todayDay && !e.is_cancelled);
    if (!classes.length) return "今日は授業がありません";
    const nowMin = now.getHours() * 60 + now.getMinutes();
    const remaining = classes.filter(e => {
      const pt = PERIOD_TIMES[e.period];
      return pt && nowMin < pt.endH * 60 + pt.endM;
    });
    if (!remaining.length) return "今日の授業はすべて終了";
    return `今日はあと${remaining.length}コマ`;
  });

  let nextClass = $derived.by(() => {
    if (!timetableData?.entries.length) return null;
    const todayDay = DAY_LABELS[now.getDay()];
    const nowMin = now.getHours() * 60 + now.getMinutes();
    const todayClasses = timetableData.entries
      .filter(e => e.day === todayDay && !e.is_cancelled)
      .sort((a, b) => a.period - b.period);
    for (const entry of todayClasses) {
      const pt = PERIOD_TIMES[entry.period];
      if (!pt) continue;
      const startMin = pt.startH * 60 + pt.startM;
      const endMin = pt.endH * 60 + pt.endM;
      if (nowMin < endMin) {
        return { entry, time: pt, live: nowMin >= startMin };
      }
    }
    return null;
  });

  let upcomingDays = $derived.by(() => {
    if (!timetableData?.entries.length) {
      console.log("[HomePage] no timetable entries");
      return [];
    }
    const todayDow = now.getDay(); // 0=Sun..6=Sat
    const nowMin = now.getHours() * 60 + now.getMinutes();

    // Build map: day name → non-cancelled entries
    const dayMap = new Map<string, TimetableEntry[]>();
    for (const e of timetableData.entries) {
      if (e.is_cancelled) continue;
      const arr = dayMap.get(e.day) ?? [];
      arr.push(e);
      dayMap.set(e.day, arr);
    }

    console.log("[HomePage] days with classes:", [...dayMap.keys()], "todayDow:", todayDow, "DAY_LABELS[todayDow]:", DAY_LABELS[todayDow]);

    const result: { label: string; relLabel: string; entries: TimetableEntry[] }[] = [];

    // Scan up to 14 days ahead, find first 2 days that have classes
    for (let offset = 0; offset < 14 && result.length < 2; offset++) {
      const dow = (todayDow + offset) % 7;
      const dayStr = DAY_LABELS[dow];
      const dayEntries = dayMap.get(dayStr);
      if (!dayEntries?.length) continue;

      // If today: skip if all classes already ended
      if (offset === 0) {
        const lastEnd = Math.max(...dayEntries.map(e => {
          const pt = PERIOD_TIMES[e.period];
          return pt ? pt.endH * 60 + pt.endM : 0;
        }));
        if (nowMin >= lastEnd) continue;
      }

      const sorted = [...dayEntries].sort((a, b) => a.period - b.period);
      const d = new Date(now);
      d.setDate(d.getDate() + offset);
      const relLabel = offset === 0 ? "今日" : offset === 1 ? "明日" : `${offset}日後`;
      const label = offset === 0 ? "今日" : offset === 1 ? "明日" : `${d.getMonth() + 1}/${d.getDate()}(${dayStr})`;
      result.push({ label, relLabel, entries: sorted });
    }

    console.log("[HomePage] upcomingDays result:", result.map(r => `${r.label}: ${r.entries.length}コマ`));
    return result;
  });

  let urgentTodos = $derived.by(() => {
    const limit = new Date(now);
    limit.setDate(limit.getDate() + 5);
    return todoItems
      .filter(t => {
        if (t.status.includes("提出済")) return false;
        if (!t.deadline) return false;
        const d = new Date(t.deadline.replace(/\//g, "-"));
        return d >= now && d <= limit;
      })
      .sort((a, b) => {
        const da = new Date(a.deadline.replace(/\//g, "-")).getTime();
        const db = new Date(b.deadline.replace(/\//g, "-")).getTime();
        return da - db;
      });
  });

  let recentNotifs = $derived.by(() => {
    const merged: UnifiedNotif[] = [];
    for (const n of kwicNotifs) {
      merged.push({ source: "kwic", title: n.title, category: n.category, date: n.date });
    }
    for (const n of lunaNotifs) {
      merged.push({ source: "luna", title: n.content, category: n.module || n.course_info, date: n.date, url: n.url });
    }
    // Sort by date descending (newer first), take 3
    merged.sort((a, b) => {
      const da = new Date(a.date.replace(/\//g, "-")).getTime() || 0;
      const db = new Date(b.date.replace(/\//g, "-")).getTime() || 0;
      return db - da;
    });
    return merged.slice(0, 3);
  });

  let totalUpcoming = $derived(urgentTodos.length);

  // ============ Lifecycle ============

  let clockInterval: ReturnType<typeof setInterval>;
  let hasLoadedOnce = false;

  onMount(async () => {
    clockInterval = setInterval(() => { now = new Date(); }, 30_000);
    await loadData();
    hasLoadedOnce = true;
  });
  onDestroy(() => {
    clearInterval(clockInterval);
    unsubTimetable();
    unsubLunaTimetable();
    unsubTodo();
    unsubKwicNotifs();
    unsubLunaNotifs();
    unsubAuth();
  });

  const unsubTimetable = onCacheUpdate<TimetableData>("timetable", (fresh) => { timetableData = fresh; });
  const unsubLunaTimetable = onCacheUpdate<LunaTimetable>("luna_timetable", (fresh) => { lunaTimetable = fresh; });
  const unsubTodo = onCacheUpdate<LunaTodoItem[]>("luna_todo", (fresh) => { todoItems = fresh; });
  const unsubKwicNotifs = onCacheUpdate<NotificationsData>("notifications", (fresh) => { kwicNotifs = fresh?.entries ?? []; });
  const unsubLunaNotifs = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => { lunaNotifs = fresh ?? []; });

  // Re-fetch when auth state changes (e.g. after re-login from session expiry)
  const unsubAuth = authState.subscribe((state) => {
    if (hasLoadedOnce && state.authenticated && (!timetableData?.entries?.length || (!kwicNotifs.length && !lunaNotifs.length))) {
      loadData();
    }
  });

  async function loadData() {
    loading = true;
    try {
      const [tt, td, nt, ln, lt] = await Promise.allSettled([
        cachedFetch<TimetableData>("timetable", fetchTimetable),
        $lunaAuthState.authenticated
          ? cachedFetch<LunaTodoItem[]>("luna_todo", () => lunaInvoke<LunaTodoItem[]>("luna_fetch_todo"))
          : Promise.resolve([]),
        cachedFetch<NotificationsData>("notifications", fetchNotifications),
        $lunaAuthState.authenticated
          ? cachedFetch<LunaNotification[]>("luna_updates", () => lunaInvoke<LunaNotification[]>("luna_fetch_updates"))
          : Promise.resolve([]),
        $lunaAuthState.authenticated
          ? cachedFetch<LunaTimetable>("luna_timetable", () => lunaInvoke<LunaTimetable>("luna_fetch_timetable", {}))
          : Promise.resolve(null),
      ]);
      console.log("[HomePage] loadData results:", tt.status, td.status, nt.status, ln.status, lt.status);
      if (tt.status === "fulfilled" && tt.value) {
        timetableData = tt.value;
        console.log("[HomePage] timetable loaded:", tt.value.entries?.length, "entries, days:", [...new Set(tt.value.entries?.map((e: TimetableEntry) => e.day))]);
      } else {
        console.log("[HomePage] timetable failed:", tt.status === "rejected" ? tt.reason : "no value");
      }
      if (td.status === "fulfilled" && td.value) todoItems = td.value;
      if (nt.status === "fulfilled" && nt.value) {
        kwicNotifs = nt.value.entries ?? [];
        console.log("[HomePage] kwic notifications loaded:", kwicNotifs.length);
      }
      if (ln.status === "fulfilled" && ln.value) {
        lunaNotifs = (ln.value as LunaNotification[]) ?? [];
        console.log("[HomePage] luna notifications loaded:", lunaNotifs.length);
      }
      if (lt.status === "fulfilled" && lt.value) {
        lunaTimetable = lt.value as LunaTimetable;
        console.log("[HomePage] luna timetable loaded:", lunaTimetable.courses?.length, "courses");
      }
    } catch (err) { console.error("[HomePage] loadData error:", err); }
    loading = false;
  }

  function daysUntil(deadline: string): number {
    const d = new Date(deadline.replace(/\//g, "-"));
    return Math.ceil((d.getTime() - now.getTime()) / 86400000);
  }

  function navigate(tab: string) {
    activeTab.set(tab);
  }

  async function openDetail(entry: TimetableEntry) {
    // Prefer Luna if authenticated and course matches
    if ($lunaAuthState.authenticated && lunaTimetable?.courses) {
      const dayIdx = DAY_LABELS.indexOf(entry.day);
      const lunaMatch = lunaTimetable.courses.find(c => c.day === dayIdx && c.period === entry.period);
      if (lunaMatch) {
        try {
          await invoke("luna_open_detail_window", {
            path: "", title: lunaMatch.name, mode: "course", idnumber: lunaMatch.idnumber,
            kwicPath: entry.detail_path || null,
          });
          return;
        } catch (e) {
          console.error("Failed to open Luna detail:", e);
        }
      }
    }
    // Fallback to KWIC
    try {
      await invoke("open_detail_window", { path: entry.detail_path, courseName: entry.course_name });
    } catch (e) {
      console.error("Failed to open detail:", e);
    }
  }

  async function openLunaDetail(path: string, title: string) {
    if (!path) return;
    try {
      await invoke("luna_open_detail_window", { path, title });
    } catch (e) {
      console.error("Failed to open Luna detail:", e);
    }
  }

  function openNotif(n: UnifiedNotif) {
    if (n.source === "luna" && n.url) {
      openLunaDetail(n.url, n.title);
    } else {
      navigate("notifications");
    }
  }

  function openTodo(item: LunaTodoItem) {
    if (item.url) {
      openLunaDetail(item.url, item.content_name || item.content_type);
    } else {
      navigate("todo");
    }
  }
</script>

<div class="home">
  <!-- ===== Header: two lines ===== -->
  <div class="header">
    <div class="header-line1">
      <span class="header-date">{dateLabel}</span>
      <span class="header-id">{$authState.studentId}</span>
    </div>
    <span class="header-greeting">{greeting}</span>
  </div>

  <!-- ===== NOW / NEXT — hero row ===== -->
  {#if nextClass}
    {@const nc = nextClass}
    <section class="section">
      <button class="section-head" onclick={() => navigate("timetable")}>
        <span>{nc.live ? "いま" : "つぎの授業"}</span>
        <span class="arrow">›</span>
      </button>
      <div class="scroll-row">
        <button class="hero-card" class:hero-live={nc.live} onclick={() => openDetail(nc.entry)}>
          <span class="hero-tag">{nc.live ? "NOW" : "NEXT"}</span>
          <span class="hero-course">{nc.entry.course_name}</span>
          <span class="hero-meta">{nc.entry.room ? `${nc.entry.room} · ` : ""}{nc.time.start}–{nc.time.end}</span>
        </button>
      </div>
    </section>
  {/if}

  <!-- ===== Recent Notifications ===== -->
  <section class="section">
    <button class="section-head" onclick={() => navigate("notifications")}>
      <span>お知らせ</span>
      <span class="arrow">›</span>
    </button>
    {#if recentNotifs.length > 0}
      <div class="notif-cards">
        {#each recentNotifs as n}
          <button class="notif-card" onclick={() => openNotif(n)}>
            <div class="notif-card-top">
              <span class="notif-source" class:luna={n.source === 'luna'}>{n.source === 'kwic' ? 'KWIC' : 'Luna'}</span>
              <span class="notif-cat">{n.category}</span>
            </div>
            <span class="notif-title">{n.title}</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty-text">お知らせはありません</p>
    {/if}
  </section>

  <!-- ===== Schedule + Deadlines — shared card row ===== -->
  <section class="section">
    <button class="section-head" onclick={() => navigate("timetable")}>
      <span>スケジュール</span>
      <span class="arrow">›</span>
    </button>
    {#if loading}
      <div class="scroll-row">
        <div class="card-skel"></div>
        <div class="card-skel"></div>
      </div>
    {:else if upcomingDays.length === 0 && urgentTodos.length === 0}
      <p class="empty-text">直近の予定はありません</p>
    {:else}
      <div class="scroll-row">
        {#each upcomingDays as day}
          <div class="tile tile-schedule">
            <span class="tile-tag">{day.label}</span>
            <div class="tile-body">
              {#each day.entries as entry, i}
                {#if i > 0}<div class="tile-divider"></div>{/if}
                <button class="tile-entry" onclick={() => openDetail(entry)}>
                  <span class="tile-period">{entry.period}限</span>
                  <div class="tile-info">
                    <span class="tile-main">{entry.course_name}</span>
                    {#if entry.room}<span class="tile-sub">{entry.room}</span>{/if}
                  </div>
                  <span class="tile-chevron">›</span>
                </button>
              {/each}
            </div>
          </div>
        {/each}
        {#each urgentTodos as item}
          {@const d = daysUntil(item.deadline)}
          <button class="tile tile-dl" class:tile-crit={d <= 1} class:tile-warn={d > 1 && d <= 3} onclick={() => openTodo(item)}>
            <div class="dl-header">
              <span class="dl-course">{item.course_name}</span>
              <span class="dl-type">{item.content_type}</span>
            </div>
            <div class="dl-sep"></div>
            <span class="tile-dl-name">{item.content_name}</span>
            <span class="tile-dl-badge" class:crit={d <= 1} class:warn={d > 1 && d <= 3}>{d <= 0 ? "今日〆" : d === 1 ? "明日〆" : `${d}日後〆`}</span>
          </button>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: 28px;
    padding-bottom: 40px;
  }

  /* ===== Header ===== */
  .header {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .header-line1 {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .header-date {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .header-greeting {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .header-id {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-left: auto;
  }

  /* ===== Notifications ===== */
  .notif-list {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .notif-cards {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .notif-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--glass-border);
    border-radius: 10px;
    background: var(--bg-card);
    cursor: pointer;
    font-family: inherit;
    color: inherit;
    text-align: left;
    transition: transform 0.12s, box-shadow 0.12s;
  }

  .notif-card:hover {
    transform: scale(1.01);
    box-shadow: 0 2px 8px rgba(0,0,0,0.06);
  }

  .notif-card-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .notif-source {
    flex-shrink: 0;
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--accent);
    color: #fff;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }
  .notif-source.luna {
    background: var(--orange);
  }

  .notif-cat {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }

  .notif-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ===== Section ===== */
  .section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .section-head {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    cursor: pointer;
    font-family: inherit;
    font-size: 16px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.01em;
    padding: 0;
    text-align: left;
    width: fit-content;
    transition: color 0.12s;
  }

  .section-head:hover { color: var(--accent); }

  .arrow {
    font-size: 18px;
    font-weight: 400;
    color: var(--text-tertiary);
    transition: color 0.12s;
  }

  .section-head:hover .arrow { color: var(--accent); }

  .empty-text {
    margin: 0;
    font-size: 14px;
    color: var(--text-tertiary);
  }

  /* ===== Horizontal scroll row ===== */
  .scroll-row {
    display: flex;
    gap: 12px;
    overflow-x: auto;
    scroll-snap-type: x proximity;
    -webkit-overflow-scrolling: touch;
    padding-bottom: 4px;
    /* hide scrollbar */
    scrollbar-width: none;
  }

  .scroll-row::-webkit-scrollbar { display: none; }

  /* ===== Hero Card (NOW/NEXT) ===== */
  .hero-card {
    flex-shrink: 0;
    width: 280px;
    padding: 20px;
    border-radius: 16px;
    border: none;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
    display: flex;
    flex-direction: column;
    gap: 6px;
    transition: transform 0.15s ease;
    scroll-snap-align: start;
  }

  /* Light mode: soft blue gradient */
  .hero-card {
    color: var(--text-primary);
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 12%, transparent) 0%, color-mix(in srgb, var(--blue) 8%, transparent) 50%, color-mix(in srgb, var(--accent) 15%, transparent) 100%);
    background-size: 200% 200%;
    animation: hero-gradient 8s ease infinite;
  }

  @media (prefers-color-scheme: dark) {
    .hero-card {
      color: #fff;
      background: linear-gradient(135deg, #003d7a 0%, #002855 50%, #004a99 100%);
      background-size: 200% 200%;
      animation: hero-gradient 8s ease infinite;
    }
    .hero-card.hero-live {
      background: linear-gradient(135deg, #1b8a4a 0%, #0d5c31 50%, #24a85e 100%);
      background-size: 200% 200%;
    }
  }

  :global([data-theme="dark"]) .hero-card {
    color: #fff;
    background: linear-gradient(135deg, #003d7a 0%, #002855 50%, #004a99 100%);
    background-size: 200% 200%;
    animation: hero-gradient 8s ease infinite;
  }

  :global([data-theme="dark"]) .hero-card.hero-live {
    background: linear-gradient(135deg, #1b8a4a 0%, #0d5c31 50%, #24a85e 100%);
    background-size: 200% 200%;
  }

  @keyframes hero-gradient {
    0% { background-position: 0% 50%; }
    50% { background-position: 100% 50%; }
    100% { background-position: 0% 50%; }
  }

  .hero-card:hover { transform: scale(1.02); }

  .hero-card.hero-live {
    background: linear-gradient(135deg, color-mix(in srgb, var(--green) 15%, transparent) 0%, color-mix(in srgb, var(--green) 8%, transparent) 50%, color-mix(in srgb, var(--green) 18%, transparent) 100%);
    background-size: 200% 200%;
  }

  .hero-tag {
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.1em;
    color: var(--text-tertiary);
  }

  .hero-course {
    font-size: 18px;
    font-weight: 700;
    line-height: 1.25;
    letter-spacing: -0.01em;
  }

  .hero-meta {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    margin-top: auto;
  }

  /* ===== Unified Tile Card ===== */
  .tile {
    flex-shrink: 0;
    width: 180px;
    min-height: 180px;
    padding: 10px 12px;
    border-radius: 14px;
    border: none;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
    color: var(--text-primary);
    background: var(--bg-card);
    display: flex;
    flex-direction: column;
    gap: 4px;
    transition: transform 0.12s ease;
    scroll-snap-align: start;
  }

  .tile:hover { transform: scale(1.02); }

  .tile-tag {
    font-size: 13px;
    font-weight: 700;
    color: var(--accent);
    letter-spacing: 0.02em;
  }

  .tile-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }

  .tile-schedule {
    cursor: default;
  }
  .tile-schedule:hover { transform: none; }

  .tile-entry {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 6px;
    border: none;
    background: var(--bg-hover, rgba(128,128,128,0.06));
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
    color: inherit;
    text-align: left;
    transition: background 0.12s;
  }
  .tile-entry:hover {
    background: rgba(128,128,128,0.12);
  }

  .tile-divider {
    height: 0;
    margin: 0;
  }

  .tile-chevron {
    flex-shrink: 0;
    font-size: 12px;
    color: var(--text-tertiary);
    margin-left: auto;
  }

  .tile-row {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .tile-period {
    flex-shrink: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--accent);
    width: 26px;
  }

  .tile-info {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-width: 0;
  }

  .tile-main {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tile-sub {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  /* Deadline tile variants */
  .tile-dl {
    gap: 6px;
    justify-content: flex-start;
  }

  .tile-dl.tile-crit {
    background: color-mix(in srgb, var(--red) 10%, var(--bg-card));
  }

  .tile-dl.tile-warn {
    background: color-mix(in srgb, var(--orange) 8%, var(--bg-card));
  }

  .dl-header {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .dl-course {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dl-type {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .dl-sep {
    height: 1px;
    background: var(--glass-border);
  }

  .tile-dl-badge {
    font-size: 11px;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 5px;
    background: var(--blue);
    color: #fff;
    width: fit-content;
    margin-top: auto;
  }

  .tile-dl-badge.crit { background: var(--red); }
  .tile-dl-badge.warn { background: var(--orange); }

  .tile-dl-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    white-space: normal;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 1.35;
  }

  /* ===== Skeleton ===== */
  .card-skel {
    flex-shrink: 0;
    width: 220px;
    height: 140px;
    border-radius: 14px;
    background: var(--bg-card);
    animation: shimmer 1.5s ease-in-out infinite;
  }

  @keyframes shimmer {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 0.25; }
  }
</style>
