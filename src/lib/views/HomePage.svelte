<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { authState, lunaAuthState, kwicAuthState, activeTab, cachedFetch, onCacheUpdate } from "../stores";
  import type { TimetableData, TimetableEntry, NotificationsData, NotificationEntry, AiChatMessage } from "../stores";
  import { fetchTimetable, fetchNotifications, lunaInvoke, kwicFetchHome, kwicFetchSubportal, kwicOpenLink, kwicOpenDetail, kwicFetchDetail, getAiConfig, aiChat, fetchWeather } from "../api";
  import type { KwicPortalHome, KwicPortalSection, KwicPortalNotification, KwicNotificationDetail, KwicSubportalData, WeatherData } from "../api";
  import type { LunaTodoItem, LunaNotification, LunaCourse } from "../types";
  import { PERIOD_TIMES, DAY_LABELS } from "../types";
  import { invoke } from "@tauri-apps/api/core";

  // ============ Types ============

  interface UnifiedNotif {
    source: "kgc" | "luna" | "kwic";
    title: string;
    category: string;
    date: string;
    url?: string;
    body?: string;
    // KWIC detail params
    kwicId?: string;
    informationType?: string;
    personCategoryCd?: string;
    categoryCd?: string;
  }

  interface AiNotifResult {
    summary: string;
    important: { title: string; reason: string; index: number }[];
    suggestions: string[];
  }

  interface AiNotifCache {
    timestamp: number;
    result: AiNotifResult;
    sources: UnifiedNotif[];
  }

  interface LunaTimetable {
    courses: LunaCourse[];
  }

  // ============ State ============

  let timetableData = $state<TimetableData | null>(null);
  let lunaTimetable = $state<LunaTimetable | null>(null);
  let todoItems = $state<LunaTodoItem[]>([]);
  let kgcNotifs = $state<NotificationEntry[]>([]);
  let lunaNotifs = $state<LunaNotification[]>([]);
  let kwicHome = $state<KwicPortalHome | null>(null);
  let now = $state(new Date());
  // Day-level date: only reassigned when the calendar date or greeting-slot changes
  let todayDate = $state(new Date());
  let loading = $state(true);

  function greetingSlot(d: Date) {
    const h = d.getHours();
    return h < 5 ? 0 : h < 11 ? 1 : h < 17 ? 2 : 3;
  }

  // KWIC subportal state
  let subportalData = $state<KwicSubportalData | null>(null);
  let subportalLoading = $state(false);
  let subportalError = $state("");

  // AI smart notification state
  let aiEnabled = $state(false);
  let aiNotifResult = $state<AiNotifResult | null>(null);
  let aiNotifLoading = $state(false);
  let aiNotifError = $state("");
  let aiNotifSources = $state<UnifiedNotif[]>([]);
  let aiReplyLanguage = $state("");

  async function openSubportal(item: { url: string; title: string }) {
    // Extract tagCd from URL like /portal/subportal?tagCd=1
    const match = item.url.match(/tagCd=(\d+)/);
    if (!match) {
      // Fallback: open in browser for non-subportal links
      await invoke("open_external_url", { url: item.url }).catch(e => console.error("open_external_url failed:", e));
      return;
    }
    subportalLoading = true;
    subportalError = "";
    subportalData = null;
    try {
      subportalData = await kwicFetchSubportal(match[1]);
      if (!subportalData.title) subportalData.title = item.title;
    } catch (e: any) {
      subportalError = e?.message || String(e);
    }
    subportalLoading = false;
  }

  /** Open merged subportal (fetches multiple tagCds, merges links+notifications) */
  let showIctFacility = $state(false);

  function closeSubportal() {
    subportalData = null;
    subportalError = "";
    showIctFacility = false;
  }

  // ============ Derived ============

  // ============ Weather ============

  const WMO_DESCRIPTIONS: Record<number, { label: string; icon: string }> = {
    0: { label: "快晴", icon: "☀️" },
    1: { label: "晴れ", icon: "🌤" },
    2: { label: "くもり", icon: "⛅" },
    3: { label: "曇天", icon: "☁️" },
    45: { label: "霧", icon: "🌫" },
    48: { label: "霧氷", icon: "🌫" },
    51: { label: "小雨", icon: "🌦" },
    53: { label: "雨", icon: "🌧" },
    55: { label: "強い雨", icon: "🌧" },
    56: { label: "着氷性の霧雨", icon: "🌧" },
    57: { label: "着氷性の雨", icon: "🌧" },
    61: { label: "小雨", icon: "🌦" },
    63: { label: "雨", icon: "🌧" },
    65: { label: "大雨", icon: "🌧" },
    66: { label: "着氷性の雨", icon: "🧊" },
    67: { label: "着氷性の大雨", icon: "🧊" },
    71: { label: "小雪", icon: "🌨" },
    73: { label: "雪", icon: "❄️" },
    75: { label: "大雪", icon: "❄️" },
    77: { label: "霧雪", icon: "🌨" },
    80: { label: "にわか雨", icon: "🌦" },
    81: { label: "にわか雨", icon: "🌧" },
    82: { label: "激しいにわか雨", icon: "⛈" },
    85: { label: "にわか雪", icon: "🌨" },
    86: { label: "激しいにわか雪", icon: "❄️" },
    95: { label: "雷雨", icon: "⛈" },
    96: { label: "雷雨（雹）", icon: "⛈" },
    99: { label: "激しい雷雨（雹）", icon: "⛈" },
  };

  let weather = $state<WeatherData | null>(null);
  let tomorrowWeather = $state<WeatherData["tomorrow"]>(null);

  // Weather cycling between today and tomorrow
  let weatherShowTomorrow = $state(false);
  let weatherCycleInterval: ReturnType<typeof setInterval> | undefined;

  function startWeatherCycle() {
    stopWeatherCycle();
    if (!tomorrowWeather) return;
    weatherCycleInterval = setInterval(() => {
      weatherShowTomorrow = !weatherShowTomorrow;
    }, 6000);
  }

  function stopWeatherCycle() {
    if (weatherCycleInterval) {
      clearInterval(weatherCycleInterval);
      weatherCycleInterval = undefined;
    }
  }

  function applyWeather(data: WeatherData) {
    weather = data;
    tomorrowWeather = data.tomorrow;
    if (tomorrowWeather) startWeatherCycle();
  }

  function getWeatherInfo(code: number) {
    return WMO_DESCRIPTIONS[code] ?? { label: "不明", icon: "🌡" };
  }

  const GREETINGS: Record<string, string[]> = {
    night: [
      "おやすみなさい", "夜更かしはほどほどに",
      "明日に備えよう", "そろそろ休もう",
    ],
    morning: [
      "おはよう", "いい朝だね",
      "今日もがんばろう", "いい一日にしよう",
    ],
    day: [
      "こんにちは", "午後もがんばろう",
      "もうひとふんばり", "いい調子",
    ],
    evening: [
      "おつかれさま", "今日もおつかれ",
      "ゆっくり休んでね", "もうひと息",
    ],
  };

  // Pick a greeting that stays stable per calendar day
  let greeting = $derived.by(() => {
    const h = todayDate.getHours();
    const slot = h < 5 ? "night" : h < 11 ? "morning" : h < 17 ? "day" : "evening";
    const pool = GREETINGS[slot];
    const daySeed = todayDate.getFullYear() * 400 + todayDate.getMonth() * 32 + todayDate.getDate();
    return pool[daySeed % pool.length];
  });

  let dateLabel = $derived.by(() => {
    const m = todayDate.getMonth() + 1;
    const d = todayDate.getDate();
    const dayStr = DAY_LABELS[todayDate.getDay()];
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

  let heroClasses = $derived.by(() => {
    if (!timetableData?.entries.length) return [];
    const todayDay = DAY_LABELS[now.getDay()];
    const nowMin = now.getHours() * 60 + now.getMinutes();
    const todayClasses = timetableData.entries
      .filter(e => e.day === todayDay && !e.is_cancelled)
      .sort((a, b) => a.period - b.period);
    const result: { entry: TimetableEntry; time: typeof PERIOD_TIMES[number]; live: boolean }[] = [];
    for (const entry of todayClasses) {
      const pt = PERIOD_TIMES[entry.period];
      if (!pt) continue;
      const startMin = pt.startH * 60 + pt.startM;
      const endMin = pt.endH * 60 + pt.endM;
      if (nowMin < endMin) {
        result.push({ entry, time: pt, live: nowMin >= startMin });
        if (result.length >= 2) break;
      }
    }
    return result;
  });

  let upcomingDays = $derived.by(() => {
    if (!timetableData?.entries.length) {
      return [];
    }
    const todayDow = todayDate.getDay(); // 0=Sun..6=Sat
    const nowMin = now.getHours() * 60 + now.getMinutes();

    // Build map: day name → non-cancelled entries
    const dayMap = new Map<string, TimetableEntry[]>();
    for (const e of timetableData.entries) {
      if (e.is_cancelled) continue;
      const arr = dayMap.get(e.day) ?? [];
      arr.push(e);
      dayMap.set(e.day, arr);
    }

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

    return result;
  });

  let urgentTodos = $derived.by(() => {
    const limit = new Date(todayDate);
    limit.setDate(limit.getDate() + 5);
    return todoItems
      .filter(t => {
        if (t.status.includes("提出済")) return false;
        if (!t.deadline) return false;
        const d = new Date(t.deadline.replace(/\//g, "-"));
        return d >= todayDate && d <= limit;
      })
      .sort((a, b) => {
        const da = new Date(a.deadline.replace(/\//g, "-")).getTime();
        const db = new Date(b.deadline.replace(/\//g, "-")).getTime();
        return da - db;
      });
  });

  let recentNotifs = $derived.by(() => {
    const merged: UnifiedNotif[] = [];
    const seen = new Set<string>();
    const addUniq = (n: UnifiedNotif) => {
      // Deduplicate by normalized title + date
      const key = `${n.title.trim().replace(/\s+/g, "")}|${n.date}`;
      if (seen.has(key)) return;
      seen.add(key);
      merged.push(n);
    };
    for (const n of kgcNotifs) {
      addUniq({ source: "kgc", title: n.title, category: n.category, date: n.date });
    }
    for (const n of lunaNotifs) {
      addUniq({ source: "luna", title: n.content, category: n.module || n.course_info, date: n.date, url: n.url });
    }
    // KWIC notification sections
    if (kwicHome) {
      const notifSections = kwicHome.sections.filter(s => s.title !== "メインリンク" && s.title !== "注目コンテンツ");
      for (const sec of notifSections) {
        for (const item of sec.items) {
          addUniq({
            source: "kwic", title: item.title, category: item.category || sec.title, date: item.date,
            kwicId: item.id, informationType: item.information_type,
            personCategoryCd: item.person_category_cd, categoryCd: item.category_cd,
          });
        }
      }
    }
    // Sort by date descending (newer first), take 5
    merged.sort((a, b) => {
      const da = new Date(a.date.replace(/\//g, "-")).getTime() || 0;
      const db = new Date(b.date.replace(/\//g, "-")).getTime() || 0;
      return db - da;
    });
    return merged.slice(0, 3);
  });

  let totalUpcoming = $derived(urgentTodos.length);

  // ============ AI Suggestion Cycling ============

  let aiSuggestionIndex = $state(0);
  let aiSuggestionFade = $state(true);
  let suggestionInterval: ReturnType<typeof setInterval> | undefined;

  function startSuggestionCycle() {
    stopSuggestionCycle();
    if (!aiNotifResult?.suggestions?.length) return;
    suggestionInterval = setInterval(() => {
      aiSuggestionFade = false;
      setTimeout(() => {
        aiSuggestionIndex = (aiSuggestionIndex + 1) % (aiNotifResult?.suggestions?.length || 1);
        aiSuggestionFade = true;
      }, 400);
    }, 8000);
  }

  function stopSuggestionCycle() {
    if (suggestionInterval) {
      clearInterval(suggestionInterval);
      suggestionInterval = undefined;
    }
  }

  let displayText = $derived.by(() => {
    if (aiNotifResult?.suggestions?.length) {
      return aiNotifResult.suggestions[aiSuggestionIndex % aiNotifResult.suggestions.length];
    }
    return greeting;
  });

  let isAiSuggestion = $derived(!!(aiNotifResult?.suggestions?.length));

  // ============ Lifecycle ============

  let clockInterval: ReturnType<typeof setInterval>;
  let hasLoadedOnce = false;

  onMount(async () => {
    clockInterval = setInterval(() => {
      const prev = now;
      now = new Date();
      // Only reassign todayDate when calendar date or greeting slot changes
      if (now.getDate() !== prev.getDate() || greetingSlot(now) !== greetingSlot(prev)) {
        todayDate = now;
      }
    }, 30_000);
    cachedFetch<WeatherData>("weather", fetchWeather).then(applyWeather).catch(() => {});
    await loadData();
    hasLoadedOnce = true;
  });
  onDestroy(() => {
    clearInterval(clockInterval);
    stopSuggestionCycle();
    stopWeatherCycle();
    unsubTimetable();
    unsubLunaTimetable();
    unsubTodo();
    unsubKgcNotifs();
    unsubLunaNotifs();
    unsubKwicHome();
    unsubWeather();
    unsubAuth();
    unsubLunaAuth();
    unsubKwicAuth();
  });

  const unsubTimetable = onCacheUpdate<TimetableData>("timetable", (fresh) => { timetableData = fresh; });
  const unsubLunaTimetable = onCacheUpdate<LunaTimetable>("luna_timetable", (fresh) => { lunaTimetable = fresh; });
  const unsubTodo = onCacheUpdate<LunaTodoItem[]>("luna_todo", (fresh) => { todoItems = fresh; });
  const unsubKgcNotifs = onCacheUpdate<NotificationsData>("notifications", (fresh) => { kgcNotifs = fresh?.entries ?? []; });
  const unsubLunaNotifs = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => { lunaNotifs = fresh ?? []; });
  const unsubKwicHome = onCacheUpdate<KwicPortalHome>("kwic_home", (fresh) => { kwicHome = fresh ?? null; });
  const unsubWeather = onCacheUpdate<WeatherData>("weather", (fresh) => { if (fresh) applyWeather(fresh); });

  // Re-fetch when auth state changes (e.g. after re-login from session expiry)
  const unsubAuth = authState.subscribe((state) => {
    if (hasLoadedOnce && state.authenticated && (!timetableData?.entries?.length || (!kgcNotifs.length && !lunaNotifs.length))) {
      loadData();
    }
  });

  // Re-fetch Luna data when Luna authenticates after initial load
  const unsubLunaAuth = lunaAuthState.subscribe((state) => {
    if (hasLoadedOnce && state.authenticated && !todoItems.length && !lunaNotifs.length) {
      Promise.allSettled([
        cachedFetch<LunaTodoItem[]>("luna_todo", () => lunaInvoke<LunaTodoItem[]>("luna_fetch_todo")),
        cachedFetch<LunaNotification[]>("luna_updates", () => lunaInvoke<LunaNotification[]>("luna_fetch_updates")),
        cachedFetch<LunaTimetable>("luna_timetable", () => lunaInvoke<LunaTimetable>("luna_fetch_timetable", {})),
      ]).then(([td, ln, lt]) => {
        if (td.status === "fulfilled" && td.value) todoItems = td.value;
        if (ln.status === "fulfilled" && ln.value) lunaNotifs = ln.value as LunaNotification[];
        if (lt.status === "fulfilled" && lt.value) lunaTimetable = lt.value as LunaTimetable;
      });
    }
  });

  // Re-fetch KWIC data when KWIC authenticates after initial load
  const unsubKwicAuth = kwicAuthState.subscribe((state) => {
    if (hasLoadedOnce && state.authenticated && !kwicHome) {
      cachedFetch<KwicPortalHome>("kwic_home", kwicFetchHome).then(kh => {
        if (kh) kwicHome = kh;
      }).catch(() => {});
    }
  });

  async function loadData() {
    loading = true;
    try {
      const [tt, td, nt, ln, lt, kh] = await Promise.allSettled([
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
        $kwicAuthState.authenticated
          ? cachedFetch<KwicPortalHome>("kwic_home", kwicFetchHome)
          : Promise.resolve(null),
      ]);
      if (tt.status === "fulfilled" && tt.value) {
        timetableData = tt.value;
      } else {
        // timetable load failed
      }
      if (td.status === "fulfilled" && td.value) todoItems = td.value;
      if (nt.status === "fulfilled" && nt.value) {
        kgcNotifs = nt.value.entries ?? [];
      }
      if (ln.status === "fulfilled" && ln.value) {
        lunaNotifs = (ln.value as LunaNotification[]) ?? [];
      }
      if (lt.status === "fulfilled" && lt.value) {
        lunaTimetable = lt.value as LunaTimetable;
      }
      if (kh.status === "fulfilled" && kh.value) {
        kwicHome = kh.value as KwicPortalHome;
      }
    } catch (err) { console.error("[HomePage] loadData error:", err); }
    loading = false;
    // Check AI config after data is ready
    checkAiConfig();
  }

  const AI_CACHE_KEY = "ai-notif-cache";
  const AI_REFRESH_MS = 12 * 60 * 60 * 1000; // 12 hours

  async function checkAiConfig() {
    try {
      const cfg = await getAiConfig();
      aiEnabled = !!(cfg.api_key && cfg.api_key.trim());
      aiReplyLanguage = cfg.reply_language || "";
      if (aiEnabled) await loadAiNotifs(false);
    } catch { aiEnabled = false; }
  }

  async function loadAiNotifs(force: boolean) {
    if (!force) {
      try {
        const raw = localStorage.getItem(AI_CACHE_KEY);
        if (raw) {
          const cache: AiNotifCache = JSON.parse(raw);
          if (Date.now() - cache.timestamp < AI_REFRESH_MS) {
            aiNotifResult = cache.result;
            aiNotifSources = cache.sources || [];
            startSuggestionCycle();
            return;
          }
        }
      } catch { /* ignore bad cache */ }
    }
    await fetchAiNotifs();
  }

  async function fetchAiNotifs() {
    aiNotifLoading = true;
    aiNotifError = "";
    try {
      // Collect all notifications with rich context
      const stripHtml = (html: string) => html.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/\s+/g, " ").trim();
      const truncate = (s: string, max: number) => s.length > max ? s.slice(0, max) + "…" : s;

      const allNotifs: { source: string; title: string; category: string; date: string; extra: string; body: string }[] = [];
      for (const n of kgcNotifs) {
        allNotifs.push({ source: "KGC", title: n.title, category: n.category, date: n.date, extra: "", body: "" });
      }
      for (const n of lunaNotifs) {
        const parts: string[] = [];
        if (n.course_info) parts.push(`科目: ${n.course_info}`);
        if (n.module) parts.push(`種別: ${n.module}`);
        allNotifs.push({ source: "Luna", title: n.content, category: n.course_info || "", date: n.date, extra: parts.join(", "), body: "" });
      }
      if (kwicHome) {
        for (const sec of kwicHome.sections) {
          if (sec.title === "メインリンク" || sec.title === "注目コンテンツ") continue;
          for (const item of sec.items) {
            const flags: string[] = [];
            if (item.important) flags.push("★重要");
            if (item.category) flags.push(`分類: ${item.category}`);
            allNotifs.push({ source: "KWIC", title: item.title, category: sec.title, date: item.date, extra: flags.join(", "), body: "" });
          }
        }
      }

      // Fetch detail content for KWIC and Luna notifications (up to 10 each)
      const kwicItems: { idx: number; item: { id: string; information_type: string; person_category_cd: string; category_cd: string } }[] = [];
      const lunaItems: { idx: number; url: string }[] = [];
      let idx = kgcNotifs.length;
      for (const n of lunaNotifs) {
        if (n.url && lunaItems.length < 10) lunaItems.push({ idx, url: n.url });
        idx++;
      }
      if (kwicHome) {
        for (const sec of kwicHome.sections) {
          if (sec.title === "メインリンク" || sec.title === "注目コンテンツ") continue;
          for (const item of sec.items) {
            if (item.id && kwicItems.length < 10) {
              kwicItems.push({ idx, item: { id: item.id, information_type: item.information_type, person_category_cd: item.person_category_cd, category_cd: item.category_cd } });
            }
            idx++;
          }
        }
      }

      // Parallel fetch of notification body content
      const detailPromises: Promise<void>[] = [];
      for (const { idx: i, item } of kwicItems) {
        detailPromises.push(
          kwicFetchDetail(item as KwicPortalNotification)
            .then(d => {
              const body = truncate(stripHtml(d.body_html), 300);
              if (body) allNotifs[i].body = body;
            })
            .catch(e => console.warn(`[AI] KWIC detail fetch failed for idx=${i}:`, e))
        );
      }
      for (const { idx: i, url } of lunaItems) {
        detailPromises.push(
          lunaInvoke<{ title: string; course_name: string; sections: { heading: string; html: string }[]; attachments: { name: string; url: string }[]; meta: Record<string, string> }>("luna_fetch_detail", { path: url })
            .then(d => {
              const text = d.sections.map(s => (s.heading ? s.heading + ": " : "") + stripHtml(s.html)).join(" ");
              const body = truncate(text, 300);
              if (body) allNotifs[i].body = body;
            })
            .catch(e => console.warn(`[AI] Luna detail fetch failed for idx=${i}:`, e))
        );
      }
      if (detailPromises.length > 0) {
        await Promise.allSettled(detailPromises);
        const fetched = allNotifs.filter(n => n.body).length;
        console.log(`[AI] Detail content fetched for ${fetched}/${kwicItems.length + lunaItems.length} notifications`);
      }

      if (allNotifs.length === 0) {
        aiNotifResult = { summary: "現在通知はありません。", important: [], suggestions: [] };
        aiNotifSources = [];
        return;
      }

      // Build unified notif lookup for clickable tags
      const unifiedLookup: UnifiedNotif[] = [];
      for (const n of kgcNotifs) {
        unifiedLookup.push({ source: "kgc", title: n.title, category: n.category, date: n.date });
      }
      for (const n of lunaNotifs) {
        unifiedLookup.push({ source: "luna", title: n.content, category: n.module || n.course_info, date: n.date, url: n.url });
      }
      if (kwicHome) {
        for (const sec of kwicHome.sections) {
          if (sec.title === "メインリンク" || sec.title === "注目コンテンツ") continue;
          for (const item of sec.items) {
            unifiedLookup.push({
              source: "kwic", title: item.title, category: item.category || sec.title, date: item.date,
              kwicId: item.id, informationType: item.information_type,
              personCategoryCd: item.person_category_cd, categoryCd: item.category_cd,
            });
          }
        }
      }
      aiNotifSources = unifiedLookup;

      const notifText = allNotifs
        .map((n, i) => {
          let line = `${i + 1}. [${n.source}] ${n.date} | ${n.category} | ${n.title}`;
          if (n.extra) line += ` (${n.extra})`;
          if (n.body) line += `\n   内容: ${n.body}`;
          return line;
        })
        .join("\n");

      // Build student context with campus mapping
      const studentInfo = timetableData?.student;
      let studentCtx = `学生ID: ${$authState.studentId}`;
      if (studentInfo) {
        studentCtx += `\n氏名: ${studentInfo.name}`;
        if (studentInfo.faculty) studentCtx += `\n学部: ${studentInfo.faculty}`;
        if (studentInfo.department) studentCtx += `\n学科: ${studentInfo.department}`;
        if (studentInfo.major) studentCtx += `\n専攻: ${studentInfo.major}`;
        if (studentInfo.student_type) studentCtx += `\n学生種別: ${studentInfo.student_type}`;
        // Campus mapping based on faculty
        const faculty = studentInfo.faculty;
        const kscFaculties = ["総合政策学部", "理学部", "工学部", "生命環境学部", "建築学部"];
        const campus = kscFaculties.some(f => faculty.includes(f))
          ? "神戸三田キャンパス（KSC）"
          : "西宮上ケ原キャンパス（NUC）";
        studentCtx += `\n所属キャンパス: ${campus}`;
      }

      // Build timetable context
      let timetableCtx = "";
      if (timetableData?.entries?.length) {
        const courses = timetableData.entries
          .filter(e => !e.is_cancelled)
          .map(e => `${e.day}${e.period}限: ${e.course_name}${e.room ? ` (${e.room})` : ""}`)
          .join("\n");
        timetableCtx = `\n\n履修科目:\n${courses}`;
      }
      if (lunaTimetable?.courses?.length) {
        const lunaCourses = lunaTimetable.courses
          .map(c => `${DAY_LABELS[c.day]}${c.period}限: ${c.name} (${c.teacher})`)
          .join("\n");
        if (!timetableCtx) timetableCtx = `\n\n履修科目 (Luna):\n${lunaCourses}`;
      }

      // Build todo context
      let todoCtx = "";
      if (todoItems.length > 0) {
        const todos = todoItems
          .filter(t => !t.status.includes("提出済"))
          .slice(0, 10)
          .map(t => `${t.course_name} | ${t.content_type}: ${t.content_name} | 〆切: ${t.deadline} | ${t.status}`)
          .join("\n");
        if (todos) todoCtx = `\n\n未提出課題:\n${todos}`;
      }

      const today = new Date();
      const dayNames = ["日", "月", "火", "水", "木", "金", "土"];
      const dateStr = `${today.getFullYear()}年${today.getMonth() + 1}月${today.getDate()}日（${dayNames[today.getDay()]}）`;
      const timeStr = `${today.getHours()}:${String(today.getMinutes()).padStart(2, "0")}`;
      const nowStr = `${dateStr} ${timeStr}`;

      const systemPrompt = `あなたは関西学院大学の学生向けパーソナル通知アシスタントです。この学生の学部・キャンパス・履修科目・課題状況を踏まえ、今日時点で本当に行動が必要な情報だけを伝えてください。

★★★ 現在の日時: ${nowStr} ★★★
この日時が絶対的な基準。この日時より前に実施されたイベント・説明会・オリエンテーション等は「終了済み」。

# キャンパス情報
- 西宮上ケ原（NUC）：神学部、文学部、社会学部、法学部、経済学部、商学部、人間福祉学部、国際学部、教育学部
- 神戸三田（KSC）：総合政策学部、理学部、工学部、生命環境学部、建築学部
- 西宮聖和：教育学部（一部）
NUCとKSCは約40km離れており通学圏が完全に異なる。

# 出力形式（厳守：JSON以外の出力禁止）
{
  "_check": [
    "通知N: (タイトル抜粋) → 学生キャンパスでの日程: YYYY/MM/DD → 現在より前/後 → 採用/除外",
    ...
  ],
  "summary": "80〜150字",
  "important": [{"title": "20字以内", "reason": "15字以内", "index": N}],
  "suggestions": ["10〜20字の行動提案"]
}

★ _checkフィールドは必須。importantの候補となる全通知について、以下を1行ずつ書くこと：
  1. 通知の番号とタイトル抜粋
  2. この学生のキャンパスでの実施日（本文から特定。日程が書かれていない場合は「日程なし」）
  3. 現在（${nowStr}）と比較して「前（終了）」か「後（有効）」か
  4. 結論：「採用」か「除外」か
→ _checkで「除外」と判定した通知は、importantとsuggestionsに絶対に含めないこと。

# 関連性の優先度
1. 履修中の科目に直接関係（教室変更・休講・課題・試験）
2. 学部・学科に名指しで関係（学部事務・履修登録）
3. 所属キャンパスの施設・窓口・イベント（時間的に有効なもののみ）
4. 全学共通の重要事項（学費・奨学金・健康診断等）
5. 他キャンパス・他学部限定 → summaryで軽く触れる程度。importantには入れない

# 時間判定の詳細ルール
- 本文に複数キャンパスの日程がある → 学生の所属キャンパスの日程のみ使う
  例：国際学部（NUC）の学生。NUC開催4/4、KSC開催4/8 → 4/4で判定。現在が4/6なら「終了」
- 掲載期間がまだ有効でも、学生が参加すべき日程が過ぎていれば「終了」
- 締切系（課題提出・申請等）は残り日数を計算

# summary
- 80〜150字。今日以降に行動が必要な事項のみ書く
- 終了済みイベントには一切言及しない
- 本文付き通知はその内容を正確に反映

# important
- 最大5件。_checkで「採用」したものだけ
- ★重要マーク付きでも、時間切れなら含めない

# suggestions
- 最大3件、各10〜20字
- 「通知の要約」ではなく、通知＋学生状況を掛け合わせた一歩踏み込んだ提案
- カジュアルな口調（丁寧語・命令形は禁止）
- 良い例：「レポートは先に構成書いとくといいよ」「教室変わったから場所確認しとこ」
- 悪い例（禁止）：「〇〇のクイズ、あと3日」（事実の繰り返し）「〇〇に行こう」（根拠なし）
- 終了済みイベントに言及しない${aiReplyLanguage ? `\n\n# 言語指定\nすべての出力テキスト（summary, title, reason, suggestionsの中身）は必ず${aiReplyLanguage}で書くこと。_checkは日本語のままでよい` : ""}`;

      const userMsg = `現在日時: ${nowStr}\n${studentCtx}${timetableCtx}${todoCtx}\n\n通知一覧（${allNotifs.length}件）:\n${notifText}`;
      console.log("[AI] User message length:", userMsg.length, "Body-enriched notifs:", allNotifs.filter(n => n.body).length);

      const messages: AiChatMessage[] = [
        { role: "system", content: systemPrompt },
        { role: "user", content: userMsg },
      ];

      const raw = await aiChat(messages);
      // Extract JSON from response
      const jsonMatch = raw.match(/\{[\s\S]*\}/);
      if (!jsonMatch) throw new Error("AI応答の解析に失敗しました");
      const parsed = JSON.parse(jsonMatch[0]);
      // Strip _check (reasoning trace) before storing
      delete parsed._check;
      const result: AiNotifResult = parsed;

      aiNotifResult = result;
      localStorage.setItem(AI_CACHE_KEY, JSON.stringify({ timestamp: Date.now(), result, sources: aiNotifSources }));
      startSuggestionCycle();
    } catch (e: any) {
      aiNotifError = e?.message || String(e);
    } finally {
      aiNotifLoading = false;
    }
  }

  async function refreshAiNotifs() {
    // Re-read config in case settings changed
    try {
      const cfg = await getAiConfig();
      aiEnabled = !!(cfg.api_key && cfg.api_key.trim());
      aiReplyLanguage = cfg.reply_language || "";
    } catch { /* keep existing */ }
    await loadAiNotifs(true);
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
            kgcPath: entry.detail_path || null,
          });
          return;
        } catch (e) {
          console.error("Failed to open Luna detail:", e);
        }
      }
    }
    // Fallback to KG-Course
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
    } else if (n.source === "kwic" && n.kwicId) {
      kwicOpenDetail({
        id: n.kwicId,
        title: n.title,
        information_type: n.informationType || "",
        person_category_cd: n.personCategoryCd || "",
        category_cd: n.categoryCd || "",
      });
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
  <!-- ===== Header: date + greeting + weather ===== -->
  <div class="header">
    <div class="header-line1">
      <span class="header-date">{dateLabel}</span>
      {#if weather}
        <span class="weather-cycle">
          <span class="weather-layer" class:weather-visible={!weatherShowTomorrow} class:weather-hidden={weatherShowTomorrow}>
            <span class="weather-icon">{getWeatherInfo(weather.weatherCode).icon}</span>
            <span class="weather-temp">{weather.temperature}°</span>
            <span class="weather-label">{getWeatherInfo(weather.weatherCode).label}</span>
          </span>
          {#if tomorrowWeather}
            <span class="weather-layer" class:weather-visible={weatherShowTomorrow} class:weather-hidden={!weatherShowTomorrow}>
              <span class="weather-prefix">明日</span>
              <span class="weather-icon">{getWeatherInfo(tomorrowWeather.weatherCode).icon}</span>
              <span class="weather-temp">{tomorrowWeather.tempMin}°/{tomorrowWeather.tempMax}°</span>
            </span>
          {/if}
        </span>
      {/if}
      <span class="header-id">{$authState.studentId}</span>
    </div>
    <div class="header-line2">
      {#if isAiSuggestion}
        <span class="header-greeting header-ai-suggestion" class:fade-in={aiSuggestionFade} class:fade-out={!aiSuggestionFade}>{displayText}</span>
      {:else}
        <span class="header-greeting">{greeting}</span>
      {/if}
    </div>
  </div>

  <!-- ===== NOW / NEXT — hero row ===== -->
  {#if heroClasses.length > 0}
    <section class="section hero-section">
      <button class="section-head" onclick={() => navigate("timetable")}>
        <span>{heroClasses[0].live ? "いま" : "つぎの授業"}</span>
        <span class="arrow">›</span>
      </button>
      {#each heroClasses as nc}
        <button class="hero-card" class:hero-live={nc.live} onclick={() => openDetail(nc.entry)}>
          <span class="hero-tag">{nc.live ? "NOW" : "NEXT"}</span>
          <span class="hero-course">{nc.entry.course_name}</span>
          <span class="hero-meta">{nc.entry.room ? `${nc.entry.room} · ` : ""}{nc.time.start}–{nc.time.end}</span>
        </button>
      {/each}
    </section>
  {/if}

  <!-- ===== Recent Notifications ===== -->
  <section class="section">
    <button class="section-head" onclick={() => navigate("notifications")}>
      <span>お知らせ</span>
      <span class="arrow">›</span>
    </button>
    {#if loading && !aiNotifLoading && recentNotifs.length === 0}
      <div class="notif-cards">
        <div class="notif-skel"><div class="skel-text" style="width:36px;height:12px"></div><div class="skel-text" style="width:80%;height:14px;margin-top:8px"></div></div>
        <div class="notif-skel"><div class="skel-text" style="width:36px;height:12px"></div><div class="skel-text" style="width:65%;height:14px;margin-top:8px"></div></div>
        <div class="notif-skel"><div class="skel-text" style="width:36px;height:12px"></div><div class="skel-text" style="width:72%;height:14px;margin-top:8px"></div></div>
      </div>
    {:else if aiEnabled}
      <!-- AI Smart Notifications -->
      {#if aiNotifLoading}
        <div class="ai-loading-box">
          <div class="ai-loading-header">
            <span class="ai-badge">AI</span>
            <span class="ai-loading-text">分析中</span>
            <span class="ai-loading-dots"><span></span><span></span><span></span></span>
          </div>
          <div class="ai-loading-lines">
            <div class="ai-skel-line" style="width: 85%"></div>
            <div class="ai-skel-line" style="width: 60%"></div>
          </div>
          <div class="ai-skel-tags">
            <div class="ai-skel-tag"></div>
            <div class="ai-skel-tag" style="width: 90px"></div>
            <div class="ai-skel-tag" style="width: 70px"></div>
          </div>
        </div>
      {:else if aiNotifError}
        <div class="ai-error">
          <span>AI分析に失敗しました</span>
          <button class="ai-retry-btn" onclick={refreshAiNotifs}>再試行</button>
        </div>
        <!-- Fallback to normal notifs -->
        <div class="notif-cards">
          {#each recentNotifs as n}
            <button class="notif-card" onclick={() => openNotif(n)}>
              <div class="notif-card-top">
                <span class="notif-source" class:luna={n.source === 'luna'} class:kwic={n.source === 'kwic'}>{n.source === 'kgc' ? 'KGC' : n.source === 'luna' ? 'Luna' : 'KWIC'}</span>
                <span class="notif-cat">{n.category}</span>
              </div>
              <span class="notif-title">{n.title}</span>
            </button>
          {/each}
        </div>
      {:else if aiNotifResult}
        <div class="ai-notif-box">
          <div class="ai-notif-meta">
            <span class="ai-badge">AI</span>
            {#if aiNotifResult.suggestions.length > 0}
              <span class="ai-suggestions-row">
                {#each aiNotifResult.suggestions as s, i}
                  {#if i > 0}<span class="ai-sep">·</span>{/if}
                  <span class="ai-suggestion">{s}</span>
                {/each}
              </span>
            {/if}
            <button class="ai-refresh-btn" onclick={refreshAiNotifs} title="更新">↻</button>
          </div>
          <p class="ai-summary">{aiNotifResult.summary}</p>
          {#if aiNotifResult.important.length > 0}
            <div class="ai-tags">
              {#each aiNotifResult.important as item}
                <button class="ai-tag" onclick={() => {
                  const n = aiNotifSources[item.index - 1];
                  if (n) openNotif(n); else navigate("notifications");
                }}>
                  <span class="ai-tag-title">{item.title}</span>
                  <span class="ai-tag-reason">{item.reason}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {:else if recentNotifs.length > 0}
      <div class="notif-cards">
        {#each recentNotifs as n}
          <button class="notif-card" onclick={() => openNotif(n)}>
            <div class="notif-card-top">
              <span class="notif-source" class:luna={n.source === 'luna'} class:kwic={n.source === 'kwic'}>{n.source === 'kgc' ? 'KGC' : n.source === 'luna' ? 'Luna' : 'KWIC'}</span>
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

  <!-- ===== KWIC Portal Main Links ===== -->
  {#if loading && !kwicHome}
    <section class="section">
      <div class="section-head-static"><span>ポータルリンク</span></div>
      <div class="kwic-link-grid">
        {#each Array(8) as _}
          <div class="kwic-link-skel"><div class="skel-text" style="width:60%;height:13px"></div></div>
        {/each}
      </div>
    </section>
  {:else if $kwicAuthState.authenticated && kwicHome}
    {#if subportalData || subportalLoading || subportalError}
      <!-- Subportal Detail View -->
      <section class="section">
        <button class="section-head" onclick={closeSubportal}>
          <span class="back-arrow">‹</span>
          <span>{subportalData?.title || "読み込み中…"}</span>
        </button>
        {#if subportalLoading}
          <div class="subportal-loading">読み込み中…</div>
        {:else if subportalError}
          <div class="subportal-error">{subportalError}</div>
        {:else if subportalData}
          {#if subportalData.links.length > 0}
            <div class="kwic-link-list">
              {#each subportalData.links as link}
                <button class="kwic-sub-link" onclick={() => kwicOpenLink(link.url, link.title)}>
                  <span class="kwic-sub-link-title">{link.title}</span>
                </button>
              {/each}
            </div>
          {:else}
            <p class="empty-text">コンテンツはありません</p>
          {/if}
        {/if}
      </section>
    {:else if showIctFacility}
      <!-- ICT・施設利用 choice -->
      <section class="section">
        <button class="section-head" onclick={closeSubportal}>
          <span class="back-arrow">‹</span>
          <span>ICT・施設利用</span>
        </button>
        <div class="kwic-link-list">
          <button class="kwic-sub-link" onclick={() => kwicOpenLink("https://kwic.kwansei.ac.jp/portal/subportal?tagCd=6", "ICT活用・サポート")}>
            <span class="kwic-sub-link-title">ICT活用・サポート</span>
          </button>
          <button class="kwic-sub-link" onclick={() => kwicOpenLink("https://kwic.kwansei.ac.jp/portal/subportal?tagCd=9", "各種施設利用・イベント")}>
            <span class="kwic-sub-link-title">各種施設利用・イベント</span>
          </button>
        </div>
      </section>
    {:else}
      {@const mainLinks = kwicHome.sections.find(s => s.title === "メインリンク")}
      {#if mainLinks && mainLinks.items.length > 0}
        {@const ICT_TAG = "tagCd=6"}
        {@const FACILITY_TAG = "tagCd=9"}
        {@const filteredItems = mainLinks.items.filter(i => !i.url.includes(ICT_TAG) && !i.url.includes(FACILITY_TAG))}
        {@const hasIct = mainLinks.items.some(i => i.url.includes(ICT_TAG))}
        {@const hasFacility = mainLinks.items.some(i => i.url.includes(FACILITY_TAG))}
        <section class="section">
          <div class="section-head-static">
            <span>ポータルリンク</span>
          </div>
          <div class="kwic-link-grid">
            {#each filteredItems as item}
              <button class="kwic-link-card" onclick={() => openSubportal(item)}>
                <span class="kwic-link-title">{item.title}</span>
              </button>
            {/each}
            {#if hasIct || hasFacility}
              <button class="kwic-link-card" onclick={() => showIctFacility = true}>
                <span class="kwic-link-title">ICT・施設利用</span>
              </button>
            {/if}
          </div>
        </section>
      {/if}
    {/if}
  {/if}

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
    align-items: center;
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
    transition: opacity 0.4s ease-in-out, transform 0.4s ease-in-out;
  }

  .header-ai-suggestion {
    font-size: 20px;
  }

  .fade-in { opacity: 1; transform: translateY(0); }
  .fade-out { opacity: 0; transform: translateY(4px); }

  .header-line2 {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  .weather-cycle {
    position: relative;
    display: inline-flex;
    align-items: center;
    min-width: 100px;
    height: 20px;
  }

  .weather-layer {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 14px;
    color: var(--text-secondary);
    font-weight: 500;
    white-space: nowrap;
    position: absolute;
    left: 0;
    top: 50%;
    transform: translateY(-50%) translateZ(0);
    will-change: opacity;
    transition: opacity 0.6s ease;
  }

  .weather-visible { opacity: 1; }
  .weather-hidden { opacity: 0; pointer-events: none; }

  .weather-prefix {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .weather-icon {
    font-size: 16px;
    line-height: 1;
  }

  .weather-temp {
    font-weight: 600;
    color: var(--text-primary);
  }

  .weather-label {
    font-size: 12px;
    color: var(--text-tertiary);
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
  .notif-source.kwic {
    background: var(--green, #38a169);
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

  /* ===== AI Notifications ===== */

  .ai-loading-box {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 16px;
    border-radius: 14px;
    border: 0.5px solid rgba(175, 82, 222, 0.15);
    background: linear-gradient(160deg, var(--bg-card) 0%, color-mix(in srgb, var(--bg-card) 96%, rgba(175, 82, 222, 0.08)) 100%);
  }

  .ai-loading-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ai-loading-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .ai-loading-dots {
    display: inline-flex;
    gap: 3px;
    align-items: center;
  }

  .ai-loading-dots span {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: rgba(175, 82, 222, 0.6);
    animation: ai-dot-bounce 1.2s ease-in-out infinite;
  }

  .ai-loading-dots span:nth-child(2) { animation-delay: 0.15s; }
  .ai-loading-dots span:nth-child(3) { animation-delay: 0.3s; }

  @keyframes ai-dot-bounce {
    0%, 60%, 100% { opacity: 0.3; transform: translateY(0); }
    30% { opacity: 1; transform: translateY(-3px); }
  }

  .ai-loading-lines {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ai-skel-line {
    height: 10px;
    border-radius: 5px;
    background: linear-gradient(90deg, var(--glass-border) 25%, color-mix(in srgb, var(--glass-border) 60%, transparent) 50%, var(--glass-border) 75%);
    background-size: 200% 100%;
    animation: ai-shimmer 1.5s ease-in-out infinite;
  }

  .ai-skel-tags {
    display: flex;
    gap: 6px;
  }

  .ai-skel-tag {
    width: 80px;
    height: 28px;
    border-radius: 10px;
    background: linear-gradient(90deg, var(--glass-border) 25%, color-mix(in srgb, var(--glass-border) 60%, transparent) 50%, var(--glass-border) 75%);
    background-size: 200% 100%;
    animation: ai-shimmer 1.5s ease-in-out infinite;
  }

  @keyframes ai-shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  .ai-error {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--red, #e53e3e);
    margin-bottom: 8px;
  }

  .ai-retry-btn {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 6px;
    border: 1px solid var(--glass-border);
    background: var(--bg-card);
    color: var(--text-secondary);
    cursor: pointer;
    font-family: inherit;
  }

  .ai-notif-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
    border-radius: 14px;
    border: 0.5px solid rgba(175, 82, 222, 0.15);
    background: linear-gradient(160deg, var(--bg-card) 0%, color-mix(in srgb, var(--bg-card) 96%, rgba(175, 82, 222, 0.08)) 100%);
  }

  .ai-notif-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .ai-badge {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 6px;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.85), rgba(0, 122, 255, 0.85));
    color: rgb(255, 255, 255);
    line-height: 1.5;
    letter-spacing: 0px;
  }

  .ai-suggestions-row {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }

  .ai-sep {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .ai-suggestion {
    font-size: 11px;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.85), rgba(0, 122, 255, 0.85));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ai-summary {
    margin: 0;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    line-height: 1.5;
  }

  .ai-refresh-btn {
    flex-shrink: 0;
    margin-left: auto;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    border: none;
    background: none;
    color: var(--text-tertiary);
    font-size: 14px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.12s, background 0.12s;
  }

  .ai-refresh-btn:hover {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .ai-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }

  .ai-tag {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 5px 12px;
    border-radius: 10px;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.08), rgba(0, 122, 255, 0.08));
    border: 0.5px solid rgba(175, 82, 222, 0.18);
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    transition: all 0.15s;
  }

  .ai-tag:hover {
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.18), rgba(0, 122, 255, 0.18));
    border-color: rgba(175, 82, 222, 0.35);
    transform: translateY(-1px);
  }

  .ai-tag-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ai-tag-reason {
    font-size: 10px;
    color: var(--text-secondary);
    white-space: nowrap;
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
  .hero-section {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .hero-card {
    flex: 0 1 auto;
    min-width: 0;
    padding: 6px 12px;
    border-radius: 14px;
    border: none;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    transition: transform 0.15s ease;
  }

  /* Light mode */
  .hero-card {
    color: var(--text-primary);
    background: var(--bg-card);
    border: 1px solid var(--glass-border);
  }

  @media (prefers-color-scheme: dark) {
    .hero-card {
      color: #fff;
      background: color-mix(in srgb, var(--blue) 12%, var(--bg-card));
      border-color: color-mix(in srgb, var(--blue) 20%, var(--glass-border));
    }
    .hero-card.hero-live {
      background: color-mix(in srgb, var(--green) 12%, var(--bg-card));
      border-color: color-mix(in srgb, var(--green) 20%, var(--glass-border));
    }
  }

  :global([data-theme="dark"]) .hero-card {
    color: #fff;
    background: color-mix(in srgb, var(--blue) 12%, var(--bg-card));
    border-color: color-mix(in srgb, var(--blue) 20%, var(--glass-border));
  }

  :global([data-theme="dark"]) .hero-card.hero-live {
    background: color-mix(in srgb, var(--green) 12%, var(--bg-card));
    border-color: color-mix(in srgb, var(--green) 20%, var(--glass-border));
  }

  .hero-card:hover { transform: scale(1.01); }

  .hero-card.hero-live {
    background: color-mix(in srgb, var(--green) 8%, var(--bg-card));
    border-color: color-mix(in srgb, var(--green) 15%, var(--glass-border));
  }

  .hero-tag {
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
    color: #fff;
    background: var(--blue);
    padding: 1px 6px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .hero-live .hero-tag {
    background: var(--green);
  }

  .hero-course {
    font-size: 13px;
    font-weight: 600;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hero-meta {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    white-space: nowrap;
    flex-shrink: 0;
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
  .skel-text {
    border-radius: 6px;
    background: var(--bg-card);
    animation: shimmer 1.5s ease-in-out infinite;
  }

  .notif-skel {
    padding: 12px 14px;
    border-radius: 12px;
    background: var(--bg-card);
    animation: shimmer 1.5s ease-in-out infinite;
  }

  .kwic-link-skel {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 10px 8px;
    border-radius: 10px;
    background: var(--bg-card);
    min-height: 40px;
    animation: shimmer 1.5s ease-in-out infinite;
  }

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

  /* ===== KWIC Portal ===== */
  .section-head-static {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 16px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .kwic-badge {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 4px;
    background: #7c3aed;
    color: #fff;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .kwic-link-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
  }

  .kwic-link-card {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 10px 8px;
    border: 1px solid var(--glass-border);
    border-radius: 10px;
    background: var(--bg-card);
    cursor: pointer;
    text-decoration: none;
    color: inherit;
    transition: transform 0.12s, box-shadow 0.12s;
    text-align: center;
  }

  .kwic-link-card:hover {
    transform: scale(1.02);
    box-shadow: 0 2px 8px rgba(0,0,0,0.06);
  }

  .kwic-link-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.3;
  }

  /* Subportal view */
  .back-arrow {
    font-size: 18px;
    font-weight: 400;
    color: var(--accent);
  }

  .subportal-loading, .subportal-error {
    text-align: center;
    padding: 30px 0;
    font-size: 13px;
    color: var(--text-tertiary);
  }
  .subportal-error { color: var(--red, #ef4444); }

  .kwic-link-list {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 6px;
  }

  .kwic-sub-link {
    display: flex;
    align-items: center;
    gap: 10px;
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
  .kwic-sub-link:hover {
    transform: scale(1.01);
    box-shadow: 0 2px 8px rgba(0,0,0,0.06);
  }
  .kwic-sub-link-icon {
    width: 32px;
    height: 32px;
    border-radius: 6px;
    flex-shrink: 0;
    object-fit: contain;
  }
  .kwic-sub-link-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--accent);
  }

  .kwic-sub-notifs {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .kwic-sub-notifs-label {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-secondary);
  }
  .kwic-sub-notif {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid var(--glass-border);
  }
  .kwic-sub-notif-date {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .kwic-sub-notif-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
