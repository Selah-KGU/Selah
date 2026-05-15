<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { authState, lunaAuthState, kwicAuthState, activeTab, cachedBackendFetch, onCacheUpdate, getCached, aiNotifStore, sessionExpired } from "../stores";
  import type { NotificationsData, NotificationEntry } from "../stores";
  import { lunaInvoke, kwicFetchSubportal, kwicOpenLink, kwicOpenDetail, kwicFetchDetail, getAiConfig, isAiReady, isLocalStandard2b, resetAiReady, aiChat, isDemoActive, openLunaTodoItem } from "../api";
  import type { KwicPortalHome, KwicPortalNotification, KwicSubportalData, WeatherData } from "../api";
  import type { LunaTodoItem, LunaNotification, ScheduleResponse } from "../types";
  import { PERIOD_TIMES, DAY_LABELS, DAY_NUM_LABELS } from "../types";
  import { invoke } from "@tauri-apps/api/core";
  import { openExternalUrl } from "../system";
  import { buildCourseSlots, getHeroCourses, type CourseSlot } from "../schedule";
  import {
    AI_CACHE_KEY,
    AI_REFRESH_MS,
    buildLocalSystemPrompt,
    daysUntil,
    getGreetingSlot,
    getRecentNotifications,
    getWeatherInfo,
    parseAiNotifResponse,
    pickStableGreeting,
    type AiNotifCache,
    type AiNotifResult,
    type UnifiedNotif,
  } from "./home/homeData";

  // ============ State ============

  let timetableData = $state<ScheduleResponse | null>(null);
  let todoItems = $state<LunaTodoItem[]>([]);

  let homeEntries = $derived.by((): CourseSlot[] => buildCourseSlots(timetableData));
  let kgcNotifs = $state<NotificationEntry[]>([]);
  let lunaNotifs = $state<LunaNotification[]>([]);
  let kwicHome = $state<KwicPortalHome | null>(null);
  let now = $state(new Date());
  // Day-level date: only reassigned when the calendar date or greeting-slot changes
  let todayDate = $state(new Date());
  let loading = $state(true);
  let loadInProgress = false;

  // KWIC subportal state
  let subportalData = $state<KwicSubportalData | null>(null);
  let subportalLoading = $state(false);
  let subportalError = $state("");

  // AI smart notification state
  let aiConfigEnabled = $state(false);
  let aiEnabled = $state(false);
  let aiNotifBlocked2b = $state(false);
  let aiNotifResult = $state<AiNotifResult | null>(null);
  let aiNotifLoading = $state(false);
  let aiNotifError = $state("");
  let aiNotifSources = $state<UnifiedNotif[]>([]);
  let aiReplyLanguage = $state("");
  let aiProvider = $state("");
  /** AI notifs are usable: enabled, ready, and not blocked by 2B */
  let aiNotifUsable = $derived(aiConfigEnabled && aiEnabled && !aiNotifBlocked2b);

  async function openSubportal(item: { url: string; title: string }) {
    // Extract tagCd from URL like /portal/subportal?tagCd=1
    const match = item.url.match(/tagCd=(\d+)/);
    if (!match) {
      // Fallback: open in browser for non-subportal links
      if (isDemoActive()) return;
      await openExternalUrl(item.url).catch(e => console.error("open_external_url failed:", e));
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

  function closeSubportal() {
    subportalData = null;
    subportalError = "";
  }

  // ============ Derived ============

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
    stopWeatherCycle();
    if (tomorrowWeather) startWeatherCycle();
  }

  let greeting = $derived.by(() => pickStableGreeting(todayDate));

  let dateLabel = $derived.by(() => {
    const m = todayDate.getMonth() + 1;
    const d = todayDate.getDate();
    const dayStr = DAY_LABELS[todayDate.getDay()];
    return `${m} 月 ${d} 日（${dayStr}）`;
  });

  let todaySummary = $derived.by(() => {
    if (!homeEntries.length) return null;
    const jsDow = now.getDay();
    const todayDay = jsDow === 0 ? 7 : jsDow;
    const classes = homeEntries.filter(e => e.day === todayDay && !e.is_cancelled);
    if (!classes.length) return "今日は授業がありません";
    const nowMin = now.getHours() * 60 + now.getMinutes();
    const remaining = classes.filter(e => {
      const pt = PERIOD_TIMES[e.period];
      return pt && nowMin < pt.endH * 60 + pt.endM;
    });
    if (!remaining.length) return "今日の授業はすべて終了";
    return `今日はあと${remaining.length}コマ`;
  });

  let heroClasses = $derived.by(() => getHeroCourses(homeEntries, now));

  let upcomingDays = $derived.by(() => {
    if (!homeEntries.length) {
      return [];
    }
    const todayDow = todayDate.getDay(); // 0=Sun..6=Sat
    const nowMin = now.getHours() * 60 + now.getMinutes();

    // Build map: unified day number (1=Mon..6=Sat) -> non-cancelled entries
    const dayMap = new Map<number, CourseSlot[]>();
    for (const e of homeEntries) {
      if (e.is_cancelled) continue;
      const arr = dayMap.get(e.day) ?? [];
      arr.push(e);
      dayMap.set(e.day, arr);
    }

    const result: { label: string; relLabel: string; entries: CourseSlot[] }[] = [];

    // Scan up to 14 days ahead, find first 2 days that have classes
    for (let offset = 0; offset < 14 && result.length < 2; offset++) {
      const jsDow = (todayDow + offset) % 7;
      const unifiedDay = jsDow === 0 ? 7 : jsDow; // 1=Mon..7=Sun
      const dayEntries = dayMap.get(unifiedDay);
      if (!dayEntries?.length) continue;

      // If today: skip if all classes already ended
      if (offset === 0) {
        const lastEnd = Math.max(...dayEntries.map(e => {
          const pt = PERIOD_TIMES[e.period];
          return pt ? pt.endH * 60 + pt.endM : 0;
        }));
        if (nowMin >= lastEnd) continue;
      }

      const dayStr = DAY_LABELS[jsDow];
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
    const startOfToday = new Date(todayDate.getFullYear(), todayDate.getMonth(), todayDate.getDate());
    const limit = new Date(startOfToday);
    limit.setDate(limit.getDate() + 5);
    return todoItems
      .filter(t => {
        if (t.status.includes("提出済")) return false;
        if (!t.deadline) return false;
        const d = new Date(t.deadline.replace(/\//g, "-"));
        return d >= startOfToday && d <= limit;
      })
      .sort((a, b) => {
        const da = new Date(a.deadline.replace(/\//g, "-")).getTime();
        const db = new Date(b.deadline.replace(/\//g, "-")).getTime();
        return da - db;
      });
  });

  let recentNotifs = $derived.by(() => getRecentNotifications(kgcNotifs, lunaNotifs, kwicHome));

  let totalUpcoming = $derived(urgentTodos.length);

  // ============ AI Suggestion Cycling ============

  let aiSuggestionIndex = $state(0);
  let aiSuggestionFade = $state(true);
  let suggestionInterval: ReturnType<typeof setInterval> | undefined;
  let suggestionTimeout: ReturnType<typeof setTimeout> | undefined;

  function startSuggestionCycle() {
    stopSuggestionCycle();
    if (!aiNotifResult?.suggestions?.length) return;
    suggestionInterval = setInterval(() => {
      aiSuggestionFade = false;
      suggestionTimeout = setTimeout(() => {
        aiSuggestionIndex = (aiSuggestionIndex + 1) % (aiNotifResult?.suggestions?.length || 1);
        aiSuggestionFade = true;
      }, 400);
    }, 8000);
  }

  function stopSuggestionCycle() {
    if (suggestionTimeout) {
      clearTimeout(suggestionTimeout);
      suggestionTimeout = undefined;
    }
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

  let clockInterval: ReturnType<typeof setInterval> | undefined;
  let serverDataLoaded = false;

  function tickClock() {
    const prev = now;
    now = new Date();
    if (now.getDate() !== prev.getDate() || getGreetingSlot(now) !== getGreetingSlot(prev)) {
      todayDate = now;
    }
  }

  function startClockTick() {
    if (clockInterval) return;
    clockInterval = setInterval(tickClock, 15_000);
  }

  function stopClockTick() {
    if (clockInterval) {
      clearInterval(clockInterval);
      clockInterval = undefined;
    }
  }

  function handleHomeVisibility() {
    if (document.visibilityState !== "visible") {
      // Pause short-interval timers when hidden to save CPU/battery
      stopWeatherCycle();
      stopSuggestionCycle();
      stopClockTick();
      return;
    }
    // Re-check AI config in case user just configured it in settings
    if (!aiEnabled) {
      resetAiReady();
      checkAiConfig();
    }
    // Immediately refresh clock so now/next updates on tab focus
    tickClock();
    startClockTick();
    // Resume visual cycling
    startWeatherCycle();
    startSuggestionCycle();
    // Re-fetch timetable if cache is stale
    if (serverDataLoaded) {
      cachedBackendFetch<ScheduleResponse>("schedule_data")
        .then(tt => { if (tt) timetableData = tt; })
        .catch(() => {});
    }
  }

  onMount(async () => {
    startClockTick();
    document.addEventListener("visibilitychange", handleHomeVisibility);
    // Restore cached data immediately so UI is never blank
    const cachedTT = getCached<ScheduleResponse>("schedule_data");
    const cachedTodo = getCached<LunaTodoItem[]>("luna_todo");
    const cachedKwic = getCached<KwicPortalHome>("kwic_home");
    const cachedNotifs = getCached<NotificationsData>("notifications");
    const cachedLunaNotifs = getCached<LunaNotification[]>("luna_updates");
    if (cachedTT) timetableData = cachedTT;
    if (cachedTodo) todoItems = cachedTodo;
    if (cachedKwic) kwicHome = cachedKwic;
    if (cachedNotifs) kgcNotifs = cachedNotifs.entries ?? [];
    if (cachedLunaNotifs) lunaNotifs = cachedLunaNotifs;
    if (cachedTT || cachedNotifs || cachedKwic) loading = false;
    cachedBackendFetch<WeatherData>("weather").then(applyWeather).catch(() => {});
    checkAiConfig();
    if ($authState.authenticated) {
      await loadData();
    } else {
      // Not yet authenticated (e.g. session restoring) — clear loading so
      // the auth subscriber can trigger loadData() later without being blocked.
      loading = false;
    }
  });
  onDestroy(() => {
    stopClockTick();
    document.removeEventListener("visibilitychange", handleHomeVisibility);
    stopSuggestionCycle();
    stopWeatherCycle();
    unsubTimetable();
    unsubTodo();
    unsubKgcNotifs();
    unsubLunaNotifs();
    unsubKwicHome();
    unsubWeather();
    unsubAiNotif();
    unsubAuth();
    unsubLunaAuth();
    unsubKwicAuth();
  });

  const unsubTimetable = onCacheUpdate<ScheduleResponse>("schedule_data", (fresh) => { timetableData = fresh; });
  const unsubTodo = onCacheUpdate<LunaTodoItem[]>("luna_todo", (fresh) => { todoItems = fresh; });
  const unsubKgcNotifs = onCacheUpdate<NotificationsData>("notifications", (fresh) => { kgcNotifs = fresh?.entries ?? []; });
  const unsubLunaNotifs = onCacheUpdate<LunaNotification[]>("luna_updates", (fresh) => { lunaNotifs = fresh ?? []; });
  const unsubKwicHome = onCacheUpdate<KwicPortalHome>("kwic_home", (fresh) => { kwicHome = fresh ?? null; });
  const unsubWeather = onCacheUpdate<WeatherData>("weather", (fresh) => { if (fresh) applyWeather(fresh); });

  // When AI scheduler signals a refresh (store set to null), re-run AI notif analysis
  const unsubAiNotif = aiNotifStore.subscribe((val) => {
    if (val === null && aiEnabled && !aiNotifLoading && !get(sessionExpired)) {
      fetchAiNotifs();
    }
  });

  // Re-fetch when auth state changes (e.g. after re-login, or initial session restore)
  const unsubAuth = authState.subscribe((state) => {
    if (state.authenticated && !serverDataLoaded) {
      loadData();
    } else if (!state.authenticated) {
      // Session lost — next auth will trigger a fresh load
      serverDataLoaded = false;
    }
  });

  // Re-fetch Luna data when Luna authenticates
  const unsubLunaAuth = lunaAuthState.subscribe((state) => {
    if (state.authenticated && !todoItems.length && !lunaNotifs.length) {
      Promise.allSettled([
        cachedBackendFetch<LunaTodoItem[]>("luna_todo"),
        cachedBackendFetch<LunaNotification[]>("luna_updates"),
      ]).then(([td, ln]) => {
        if (td.status === "fulfilled" && td.value) todoItems = td.value;
        if (ln.status === "fulfilled" && ln.value) lunaNotifs = ln.value as LunaNotification[];
      }).catch(() => {});
    }
  });

  // Re-fetch KWIC data when KWIC authenticates
  const unsubKwicAuth = kwicAuthState.subscribe((state) => {
    if (state.authenticated && !kwicHome) {
      cachedBackendFetch<KwicPortalHome>("kwic_home").then(kh => {
        if (kh) kwicHome = kh;
      }).catch(() => {});
    }
  });

  async function loadData() {
    if (loadInProgress) return; // prevent concurrent loads
    loadInProgress = true;
    loading = true;
    try {
      const [tt, td, nt, ln, kh] = await Promise.allSettled([
        cachedBackendFetch<ScheduleResponse>("schedule_data"),
        $lunaAuthState.authenticated
          ? cachedBackendFetch<LunaTodoItem[]>("luna_todo")
          : Promise.resolve([]),
        cachedBackendFetch<NotificationsData>("notifications"),
        $lunaAuthState.authenticated
          ? cachedBackendFetch<LunaNotification[]>("luna_updates")
          : Promise.resolve([]),
        $kwicAuthState.authenticated
          ? cachedBackendFetch<KwicPortalHome>("kwic_home")
          : Promise.resolve(null),
      ]);
      if (tt.status === "fulfilled" && tt.value) {
        timetableData = tt.value;
      }
      if (td.status === "fulfilled" && td.value) todoItems = td.value;
      if (nt.status === "fulfilled" && nt.value) {
        kgcNotifs = nt.value.entries ?? [];
      }
      if (ln.status === "fulfilled" && ln.value) {
        lunaNotifs = (ln.value as LunaNotification[]) ?? [];
      }
      if (kh.status === "fulfilled" && kh.value) {
        kwicHome = kh.value as KwicPortalHome;
      }
      // At least one fetch succeeded — mark server data as loaded
      if (tt.status === "fulfilled" || nt.status === "fulfilled") {
        serverDataLoaded = true;
      }
    } catch (err) { console.error("[HomePage] loadData error:", err); }
    loading = false;
    loadInProgress = false;
  }

  async function checkAiConfig() {
    try {
      if (get(sessionExpired)) return;
      const cfg = await getAiConfig();
      aiConfigEnabled = cfg.ai_enabled !== false;
      const ready = await isAiReady();
      aiEnabled = ready;
      aiNotifBlocked2b = await isLocalStandard2b();
      aiReplyLanguage = cfg.reply_language || "";
      aiProvider = cfg.provider || "";
      if (aiEnabled && !aiNotifBlocked2b) await loadAiNotifs(false);
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
    if (get(sessionExpired)) return;
    aiNotifLoading = true;
    aiNotifError = "";
    try {
      // Collect all notifications with rich context
      const stripHtml = (html: string) => html.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/\s+/g, " ").trim();
      const truncate = (s: string, max: number) => s.length > max ? s.slice(0, max) + "…" : s;
      const isLunaCourseTopUrl = (raw: string) => {
        if (!raw) return false;
        try {
          const url = new URL(raw, "https://luna.kwansei.ac.jp");
          return url.pathname === "/lms/course" || url.pathname === "/lms/contents";
        } catch {
          return /^\/lms\/(?:course|contents)(?:[?#]|$)/.test(raw);
        }
      };
      const pushExtra = (parts: string[], label: string, value: string | null | undefined) => {
        const text = (value || "").trim();
        if (!text) return;
        parts.push(`${label}: ${text}`);
      };

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
            pushExtra(flags, "区分", sec.title);
            allNotifs.push({
              source: "KWIC",
              title: item.title,
              category: item.category || sec.title,
              date: item.date,
              extra: flags.join(", "),
              body: "",
            });
          }
        }
      }

      // Fetch detail content for KWIC and Luna notifications
      // Local: limit to 5 each to reduce prompt size; Cloud: up to 10 each
      const detailLimit = aiProvider === "local" ? 5 : 10;
      const kwicItems: { idx: number; item: { id: string; information_type: string; person_category_cd: string; category_cd: string } }[] = [];
      const lunaItems: { idx: number; url: string; title: string }[] = [];
      let idx = kgcNotifs.length;
      for (const n of lunaNotifs) {
        if (n.url && !isLunaCourseTopUrl(n.url) && lunaItems.length < detailLimit) {
          lunaItems.push({ idx, url: n.url, title: n.content || "" });
        }
        idx++;
      }
      if (kwicHome) {
        for (const sec of kwicHome.sections) {
          if (sec.title === "メインリンク" || sec.title === "注目コンテンツ") continue;
          for (const item of sec.items) {
            if (item.id && kwicItems.length < detailLimit) {
              kwicItems.push({ idx, item: { id: item.id, information_type: item.information_type, person_category_cd: item.person_category_cd, category_cd: item.category_cd } });
            }
            idx++;
          }
        }
      }

      // Parallel fetch of notification body content
      const bodyMaxLen = aiProvider === "local" ? 150 : 300;
      const detailPromises: Promise<void>[] = [];
      for (const { idx: i, item } of kwicItems) {
        detailPromises.push(
          kwicFetchDetail(item as KwicPortalNotification)
            .then(d => {
              const body = truncate(stripHtml(d.body_html), bodyMaxLen);
              const extraParts = allNotifs[i].extra
                ? allNotifs[i].extra.split(/\s*,\s*/).filter(Boolean)
                : [];
              pushExtra(extraParts, "送信者", d.sender);
              allNotifs[i].extra = extraParts.join(", ");
              if (body) allNotifs[i].body = body;
            })
            .catch(e => console.warn(`[AI] KWIC detail fetch failed for idx=${i}:`, e))
        );
      }
      for (const { idx: i, url, title } of lunaItems) {
        detailPromises.push(
          lunaInvoke<{ title: string; course_name: string; sections: { heading: string; body: string }[]; attachments: { name: string; url: string }[]; meta: Record<string, string> }>("luna_fetch_detail", { path: url, expectedTitle: title })
            .then(d => {
              const text = d.sections.map(s => (s.heading ? s.heading + ": " : "") + stripHtml(s.body || "")).join(" ");
              const body = truncate(text, bodyMaxLen);
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
              section: sec.title,
              kwicId: item.id, informationType: item.information_type,
              personCategoryCd: item.person_category_cd, categoryCd: item.category_cd,
            });
          }
        }
      }
      aiNotifSources = unifiedLookup;

      // Helper: format a slice of notifications into numbered text
      const fmtNotifs = (notifs: typeof allNotifs, offset: number) =>
        notifs.map((n, i) => {
          let line = `${offset + i + 1}. [${n.source}] ${n.date} | ${n.category} | ${n.title}`;
          if (n.extra) line += ` (${n.extra})`;
          if (n.body) line += `\n   内容: ${n.body}`;
          return line;
        }).join("\n");

      // Build student context with campus mapping
      let studentCtx = `学生ID: ${$authState.studentId}`;
      if ($authState.displayName) studentCtx += `\n氏名: ${$authState.displayName}`;
      if ($authState.faculty) {
        studentCtx += `\n学部: ${$authState.faculty}`;
        const kscFaculties = ["総合政策学部", "理学部", "工学部", "生命環境学部", "建築学部"];
        const campus = kscFaculties.some(f => $authState.faculty.includes(f))
          ? "神戸三田キャンパス（KSC）"
          : "西宮上ケ原キャンパス（NUC）";
        studentCtx += `\n所属キャンパス: ${campus}`;
      }
      if ($authState.department) studentCtx += `\n学科: ${$authState.department}`;

      // Build timetable context
      let timetableCtx = "";
      if (homeEntries.length) {
        const courses = homeEntries
          .filter(e => !e.is_cancelled)
          .map(e => `${DAY_NUM_LABELS[e.day] ?? ""}${e.period}限: ${e.name}${e.teacher ? ` (${e.teacher})` : ""}${e.room ? ` [${e.room}]` : ""}`)
          .join("\n");
        timetableCtx = `\n\n履修科目:\n${courses}`;
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
      const baseCtx = `現在日時: ${nowStr}\n${studentCtx}${timetableCtx}${todoCtx}`;

      const isLocal = aiProvider === "local";

      const systemPrompt = isLocal
        ? buildLocalSystemPrompt(nowStr, aiReplyLanguage)
        : `あなたは関西学院大学の学生向けパーソナル通知アシスタントです。
学生のプロフィール（学部・キャンパス・履修科目・課題状況）と通知一覧を受け取り、今この学生にとって重要な情報を分析します。

━━━━━━━━━━━━━━━━━━━━━━━━━
現在の日時: ${nowStr}
━━━━━━━━━━━━━━━━━━━━━━━━━
この日時がすべての判断の基準です。

# 関西学院大学のキャンパスと学部の対応
- 西宮上ケ原キャンパス（NUC）：神学部、文学部、社会学部、法学部、経済学部、商学部、人間福祉学部、国際学部、教育学部
- 神戸三田キャンパス（KSC）：総合政策学部、理学部、工学部、生命環境学部、建築学部
- 西宮聖和キャンパス：教育学部（一部の課程）

NUCとKSCは約40km離れています。あるキャンパスのイベント・教室変更・窓口案内は、別キャンパスの学生にはほぼ無関係です。

# あなたの作業手順

## ステップ1: 各通知を個別に検証する（_checkフィールド）

通知一覧のすべての通知について、1件ずつ以下のチェックを行い、_checkに記録してください。

チェック項目：
a) この通知は何についてか？（1行で要約）
b) この通知にはイベント・説明会・オリエンテーション・ガイダンス等の日程が含まれるか？
   - 含まれる場合：学生の所属キャンパスでの開催日はいつか？
   - 複数キャンパスで別日程の場合：学生のキャンパスの日程のみ抽出する
c) その日程は現在（${nowStr}）より前か後か？
   - 前 → この通知のイベントは「終了済み」
d) この学生の学部・キャンパス・履修科目に関係するか？
e) 結論：「採用」「低優先」「除外」のいずれか

★★★ 非常に重要な注意 ★★★
- 各通知は完全に独立した別の通知です。番号が違えば別の通知です。
- 通知3と通知7のタイトルが似ていても、それぞれの本文（「内容:」）に書かれた日程は異なる可能性があります。
- 通知Aの本文にある日程を、通知Bの判定に使ってはいけません。
- 必ず各通知の「内容:」フィールドのみからその通知の日程を読み取ってください。
- 「内容:」がない通知は、タイトルと日付のみで判断してください。

_checkの書き方の例：
"通知1: 春学期履修登録について → 日程: 4/10まで → 現在4/6より後（有効） → 履修関連で全学生対象 → 採用"
"通知2: NUC留学生オリエンテーション → NUC日程: 4/4 → 現在4/6より前（終了） → 除外"
"通知3: KSC留学生オリエンテーション → KSC日程: 4/8 → 現在4/6より後 → ただし学生はNUC所属 → 低優先"
"通知4: 日本語Iの小テスト → 日程なし（締切:4/9） → 残り3日 → 履修科目と一致 → 採用"

## ステップ2: summaryを書く

_checkで「採用」と判定した通知だけを使ってsummaryを書きます。

summaryのルール：
- 80〜150字
- 今日以降にこの学生が行動すべき内容を具体的に書く
- 本文（「内容:」）がある通知は、そこに書かれた具体的な情報（教室名・時間・場所など）を正確に引用する
- 課題の締切は「あとN日」の形で残り日数を明記する
- _checkで「除外」した通知（終了済み・無関係）には一切言及しない
- _checkで「低優先」の通知は、スペースがあれば簡潔に触れてよいが、なくてもよい

## ステップ3: importantを選ぶ

importantのルール：
- _checkで「採用」と判定した通知の中から最大5件選ぶ
- 「除外」と判定した通知は、★重要マーク付きであっても絶対にimportantに入れない
- indexは通知一覧の番号（1始まり）と完全に一致させること

優先順位：
1. 履修中の科目に直接関係する通知（教室変更・休講・課題・試験）
2. 学部・学科に名指しで関係する通知（学部事務・履修登録・ゼミ関連）
3. 所属キャンパスの施設・窓口・イベントで、日程が未来のもの
4. 全学生共通の重要事項（学費・奨学金・健康診断・システムメンテナンス等）
5. 他キャンパス・他学部限定の通知はimportantに含めない

## ステップ4: suggestionsを書く

suggestionsのルール：
- 最大3件、各10〜20字
- suggestionsは通知の要約やリマインダーではありません
- 通知の情報とこの学生の状況を組み合わせて初めて出てくる、一歩踏み込んだアドバイスを書いてください
- 学生が通知を読んだだけでは気づかないような視点や行動を提案する

語調：
- 友達に軽く教えるようなカジュアルな口調で書く
- 丁寧語（〜ましょう、〜してください）や命令形は使わない

良い例（通知にない付加価値がある）：
- 「レポートは構成だけ先に書いとくといいよ」（通知は締切だけ → 早期着手の提案）
- 「教室変わったから初回は早めに出よう」（通知は変更告知 → 迷わないための提案）
- 「奨学金の書類、教務課で先にもらっておこう」（通知は募集開始 → 準備行動の提案）

悪い例（禁止。通知の繰り返しでしかない）：
- ×「〇〇のクイズ、あと3日」← summaryで書くべき事実
- ×「Web問診に回答しよう」← 通知の指示そのまま
- ×「〇〇に参加しよう」← 通知を読めばわかること
- ×「確認しましょう」「注意してください」← 中身がない

終了済みイベントに関するsuggestionsは絶対に書かない。

# 出力形式

以下のJSON形式のみ出力してください。JSON以外のテキストは一切不要です。

{
  "_check": [
    "通知N: (要約) → (キャンパスでの日程) → (現在との比較) → (学生との関連) → 採用/低優先/除外",
    ...全通知分
  ],
  "summary": "80〜150字のサマリー",
  "important": [
    {"title": "20字以内の短縮タイトル", "reason": "15字以内の理由", "index": 通知番号}
  ],
  "suggestions": ["10〜20字の行動提案"]
}${aiReplyLanguage ? `\n\n# 言語指定\nsummary, important内のtitle/reason, suggestionsの中身は必ず${aiReplyLanguage}で書くこと。_checkは日本語のままでよい。` : ""}`;

      const fullNotifText = fmtNotifs(allNotifs, 0);
      const fullUserMsg = `${baseCtx}\n\n通知一覧（${allNotifs.length}件）:\n${fullNotifText}`;

      let result: AiNotifResult;
      console.log("[AI] Single request: provider=%s, user msg %d chars, body-enriched %d", aiProvider, fullUserMsg.length, allNotifs.filter(n => n.body).length);
      const raw = await aiChat([{ role: "system", content: systemPrompt }, { role: "user", content: fullUserMsg }]);
      result = parseAiNotifResponse(raw);

      aiNotifResult = result;
      localStorage.setItem(AI_CACHE_KEY, JSON.stringify({ timestamp: Date.now(), result, sources: aiNotifSources }));
      aiNotifStore.set({ result, sources: aiNotifSources, timestamp: Date.now() });
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
      aiConfigEnabled = cfg.ai_enabled !== false;
      const ready = await isAiReady();
      aiEnabled = ready;
      aiNotifBlocked2b = await isLocalStandard2b();
      aiReplyLanguage = cfg.reply_language || "";
      aiProvider = cfg.provider || "";
    } catch { /* keep existing */ }
    if (aiNotifBlocked2b) return;
    await loadAiNotifs(true);
  }

  function navigate(tab: string) {
    activeTab.set(tab);
  }

  async function openDetail(entry: CourseSlot) {
    if (isDemoActive()) {
      navigate("timetable");
      return;
    }
    // Prefer Luna if authenticated and course has luna_id
    if ($lunaAuthState.authenticated && entry.luna_id) {
      try {
        await invoke("university_open_detail_window", {
          path: "", title: entry.name, mode: "course", idnumber: entry.luna_id,
          kgcPath: entry.detail_path || null, courseName: entry.name,
        });
        return;
      } catch (e) {
        console.error("Failed to open Luna detail:", e);
      }
    }
    // Fallback to KG-Course
    if (entry.detail_path) {
      try {
        await invoke("open_detail_window", { path: entry.detail_path, courseName: entry.name });
      } catch (e) {
        console.error("Failed to open detail:", e);
      }
    }
  }

  async function openLunaDetail(path: string, title: string, courseName?: string | null) {
    if (!path) return;
    if (isDemoActive()) {
      navigate("todo");
      return;
    }
    try {
      await invoke("university_open_detail_window", { path, title, courseName: courseName || null });
    } catch (e) {
      console.error("Failed to open Luna detail:", e);
    }
  }

  function openNotif(n: UnifiedNotif) {
    if (n.source === "luna" && n.url) {
      openLunaDetail(n.url, n.title, n.courseInfo);
    } else if (n.source === "kwic" && n.kwicId) {
      if (isDemoActive()) {
        navigate("notifications");
        return;
      }
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
      if (isDemoActive()) {
        navigate("todo");
        return;
      }
      openLunaTodoItem(item).catch((e) => console.error("Failed to open TODO item:", e));
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
          <span class="hero-course">{nc.entry.name}</span>
          <span class="hero-meta">{nc.entry.room ? `${nc.entry.room} · ` : ""}{nc.time.start}–{nc.time.end}</span>
        </button>
      {/each}
    </section>
  {/if}

  <!-- ===== Recent Notifications ===== -->
  <section class="section">
    <div class="section-head-row">
      <button class="section-head" onclick={() => navigate("notifications")}>
        <span>お知らせ</span>
        <span class="arrow">›</span>
      </button>
      {#if aiNotifUsable && aiNotifError && !aiNotifLoading}
        <button class="ai-fail-pill" onclick={refreshAiNotifs} title={aiNotifError}>
          <span class="ai-fail-dot"></span>
          <span>AI要約失敗: {aiNotifError.length > 20 ? aiNotifError.slice(0, 20) + '...' : aiNotifError}</span>
          <span class="ai-fail-retry">再試行</span>
        </button>
      {/if}
    </div>
    {#if loading && !aiNotifLoading && recentNotifs.length === 0}
      <div class="notif-cards">
        <div class="notif-skel"><div class="skel-text" style="width:36px;height:12px"></div><div class="skel-text" style="width:80%;height:14px;margin-top:8px"></div></div>
        <div class="notif-skel"><div class="skel-text" style="width:36px;height:12px"></div><div class="skel-text" style="width:65%;height:14px;margin-top:8px"></div></div>
        <div class="notif-skel"><div class="skel-text" style="width:36px;height:12px"></div><div class="skel-text" style="width:72%;height:14px;margin-top:8px"></div></div>
      </div>
    {:else if aiNotifUsable}
      <!-- AI Smart Notifications -->
      {#if aiNotifLoading}
        <div class="ai-loading-box">
          <div class="ai-loading-header">
            <span class="ai-badge"><svg width="12" height="12" viewBox="0 0 20 20" fill="none" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="#fff" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="#fff" stroke-linejoin="round" stroke-width="1"/></svg><span class="ai-badge-text">AI 要約</span></span>
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
            <span class="ai-badge"><svg width="12" height="12" viewBox="0 0 20 20" fill="none" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="#fff" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="#fff" stroke-linejoin="round" stroke-width="1"/></svg><span class="ai-badge-text">AI 要約</span></span>
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
      {:else}
        <button class="ai-trigger-box" onclick={refreshAiNotifs}>
          <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke-linejoin="round" stroke-width="1"/></svg>
          <span>AI 分析を実行</span>
        </button>
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
    {:else}
      {@const mainLinks = kwicHome.sections.find(s => s.title === "メインリンク")}
      {#if mainLinks && mainLinks.items.length > 0}
        {@const ICT_TAG = "tagCd=6"}
        {@const filteredItems = mainLinks.items.filter(i => !i.url.includes(ICT_TAG))}
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
    {#if loading && !timetableData}
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
                    <span class="tile-main">{entry.name}</span>
                    {#if entry.room}<span class="tile-sub">{entry.room}</span>{/if}
                  </div>
                  <span class="tile-chevron">›</span>
                </button>
              {/each}
            </div>
          </div>
        {/each}
        {#each urgentTodos as item}
          {@const d = daysUntil(item.deadline, now)}
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

  .section-head-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ai-fail-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    font-family: inherit;
    padding: 2px 8px 2px 6px;
    border-radius: 20px;
    border: none;
    background: color-mix(in srgb, var(--red, #e53e3e) 12%, transparent);
    color: var(--red, #e53e3e);
    cursor: pointer;
    transition: background 0.15s;
    white-space: nowrap;
  }
  .ai-fail-pill:hover {
    background: color-mix(in srgb, var(--red, #e53e3e) 20%, transparent);
  }
  .ai-fail-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--red, #e53e3e);
    flex-shrink: 0;
  }
  .ai-fail-retry {
    margin-left: 2px;
    padding-left: 4px;
    border-left: 1px solid color-mix(in srgb, var(--red, #e53e3e) 30%, transparent);
    opacity: 0.8;
  }

  .ai-trigger-box {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    width: 100%;
    padding: 14px;
    border-radius: 10px;
    border: 1px dashed var(--border-strong, rgba(0,0,0,0.12));
    background: none;
    color: var(--text-tertiary);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    transition: color 0.15s, border-color 0.15s, background 0.15s;
  }
  .ai-trigger-box:hover {
    color: var(--accent);
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
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
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    background: linear-gradient(135deg, #c480e8, #6bacf0);
    border-radius: 50px;
    padding: 3px 7px 3px 5px;
  }
  .ai-badge-text {
    font-size: 10px;
    font-weight: 700;
    color: #fff;
    letter-spacing: 0.5px;
    line-height: 12px;
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
    scrollbar-width: none;
    cursor: grab;
  }

  .scroll-row:active { cursor: grabbing; }

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
    line-clamp: 3;
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
  .kwic-sub-link-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--accent);
  }
</style>
