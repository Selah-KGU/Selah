<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import { get } from "svelte/store";
  import { authState, lunaAuthState, kwicAuthState, activeTab, cachedBackendFetch, onCacheUpdate, getCached, aiNotifStore, sessionExpired } from "../stores";
  import type { NotificationsData, NotificationEntry } from "../stores";
  import { kwicFetchSubportal, kwicOpenLink, kwicOpenDetail, kwicOpenCabinetReference, getAiConfig, isAiReady, resetAiReady, isDemoActive, openLunaTodoItem, backendAiRefreshNow } from "../api";
  import type { KwicPortalHome, KwicSubportalData, WeatherData } from "../api";
  import type { LunaTodoItem, LunaNotification, ScheduleResponse } from "../types";
  import { PERIOD_TIMES, DAY_LABELS } from "../types";
  import { invoke } from "@tauri-apps/api/core";
  import { openExternalUrl } from "../system";
  import { buildCourseSlots, getHeroCourses, type CourseSlot } from "../schedule";
  import { showHomeOnboardingCard, reopenOnboarding } from "../onboarding/onboardingState";
  import selahLogoUrl from "../../assets/logo.png";
  import {
    daysUntil,
    getGreetingSlot,
    getRecentNotifications,
    getWeatherInfo,
    pickStableGreeting,
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
  let showOnboardingCard = $derived($showHomeOnboardingCard);

  function handleStartOnboarding() {
    reopenOnboarding();
  }

  // KWIC subportal state
  let subportalData = $state<KwicSubportalData | null>(null);
  let subportalLoading = $state(false);
  let subportalError = $state("");
  let portalExpansionItems = $state<Array<{ title: string; url: string }> | null>(null);

  // AI smart notification state
  let aiConfigEnabled = $state(false);
  let aiEnabled = $state(false);
  let aiNotifResult = $state<AiNotifResult | null>(null);
  let aiNotifLoading = $state(false);
  let aiNotifError = $state("");
  let aiNotifSources = $state<UnifiedNotif[]>([]);
  /** AI notifs are usable when AI is enabled and the selected provider is ready. */
  let aiNotifUsable = $derived(aiConfigEnabled && aiEnabled);

  function isTeacherQualificationPortal(item: { title: string }) {
    return item.title.includes("教職") || item.title.includes("資格取得");
  }

  function isCertificatePortal(item: { title: string }) {
    return item.title.includes("証明書発行") || item.title.includes("各種手続");
  }

  function isCabinetPortal(item: { url: string }) {
    return /\/cabinet\/reference(?:\?|$)/.test(item.url);
  }

  function openMergedPortal(title: string, items: Array<{ title: string; url: string } | undefined>) {
    const links = items.filter((item): item is { title: string; url: string } => !!item);
    if (!links.length) return;
    subportalLoading = false;
    subportalError = "";
    portalExpansionItems = links;
    subportalData = {
      title,
      links: links.map(item => ({ title: item.title, url: item.url, icon_url: "", description: "" })),
      notifications: [],
    };
  }

  async function openSubportal(item: { url: string; title: string }) {
    portalExpansionItems = null;
    if (isCabinetPortal(item)) {
      await kwicOpenCabinetReference(item.title || "学生キャビネット").catch(e => console.error("kwic_open_cabinet_window failed:", e));
      return;
    }
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
    portalExpansionItems = null;
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
    if (tomorrowWeather && homeTimersShouldRun()) startWeatherCycle();
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

  function homeTimersShouldRun() {
    return document.visibilityState === "visible" && get(activeTab) === "home";
  }

  function stopHomeUiTimers() {
    stopWeatherCycle();
    stopSuggestionCycle();
    stopClockTick();
  }

  function syncHomeUiTimers() {
    if (!homeTimersShouldRun()) {
      stopHomeUiTimers();
      return;
    }
    tickClock();
    startClockTick();
    startWeatherCycle();
    startSuggestionCycle();
  }

  function handleHomeVisibility() {
    if (!homeTimersShouldRun()) {
      // Pause short-interval timers when hidden to save CPU/battery
      stopHomeUiTimers();
      return;
    }
    // Re-check AI config in case user just configured it in settings
    if (!aiEnabled) {
      resetAiReady();
      checkAiConfig();
    }
    // Immediately refresh clock so now/next updates on tab focus, then resume visual cycling.
    syncHomeUiTimers();
    // Re-fetch timetable if cache is stale
    if (serverDataLoaded) {
      cachedBackendFetch<ScheduleResponse>("schedule_data")
        .then(tt => { if (tt) timetableData = tt; })
        .catch(() => {});
    }
  }

  $effect(() => {
    $activeTab;
    // syncHomeUiTimers → tickClock reads and writes `now` ($state).
    // Without untrack, writing `now` would re-trigger this effect → infinite loop.
    untrack(() => syncHomeUiTimers());
  });

  onMount(async () => {
    syncHomeUiTimers();
    document.addEventListener("visibilitychange", handleHomeVisibility);
    // Restore cached data immediately so UI is never blank
    const cachedTodo = getCached<LunaTodoItem[]>("luna_todo");
    const cachedKwic = getCached<KwicPortalHome>("kwic_home");
    const cachedNotifs = getCached<NotificationsData>("notifications");
    const cachedLunaNotifs = getCached<LunaNotification[]>("luna_updates");
    if (cachedTodo) todoItems = cachedTodo;
    if (cachedKwic) kwicHome = cachedKwic;
    if (cachedNotifs) kgcNotifs = cachedNotifs.entries ?? [];
    if (cachedLunaNotifs) lunaNotifs = cachedLunaNotifs;
    if (cachedNotifs || cachedKwic || cachedTodo) loading = false;
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

  // Backend AI refresh writes analysis into the shared store; Home only displays it.
  const unsubAiNotif = aiNotifStore.subscribe((val) => {
    if (val?.result) {
      aiNotifResult = val.result;
      aiNotifSources = val.sources || [];
      aiNotifError = "";
      aiNotifLoading = false;
      if (homeTimersShouldRun()) startSuggestionCycle();
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
      aiConfigEnabled = (await getAiConfig()).ai_enabled !== false;
      const ready = await isAiReady();
      aiEnabled = ready;
    } catch { aiEnabled = false; }
  }

  async function refreshAiNotifs() {
    // Re-read config in case settings changed
    try {
      aiConfigEnabled = (await getAiConfig()).ai_enabled !== false;
      const ready = await isAiReady();
      aiEnabled = ready;
    } catch { /* keep existing */ }
    if (!aiNotifUsable) return;
    if (get(sessionExpired)) return;
    aiNotifLoading = true;
    aiNotifError = "";
    try {
      await backendAiRefreshNow(true, ["ai_notif"]);
    } catch (e: any) {
      aiNotifError = e?.message || String(e);
    } finally {
      aiNotifLoading = false;
    }
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
        return;
      }
    }
    // Use KG-Course only when no Luna detail is available.
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
    </div>
    <div class="header-line2">
      {#if isAiSuggestion}
        <span class="header-greeting header-ai-suggestion" class:fade-in={aiSuggestionFade} class:fade-out={!aiSuggestionFade}>{displayText}</span>
      {:else}
        <span class="header-greeting">{greeting}</span>
      {/if}
    </div>
  </div>

  {#if showOnboardingCard}
    <div class="ob-banner">
      <img class="ob-banner-logo" src={selahLogoUrl} alt="" draggable="false" />
      <div class="ob-banner-body">
        <div class="ob-banner-title">初期設定を完了する</div>
        <div class="ob-banner-sub">AI・メール・通知を 2 分でセットアップ</div>
      </div>
      <button class="ob-banner-start" onclick={handleStartOnboarding}>始める</button>
    </div>
  {/if}

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
          {#if portalExpansionItems && portalExpansionItems.length > 0}
            <div class="kwic-link-list">
              {#each portalExpansionItems as item}
                <button class="kwic-sub-link" onclick={() => openSubportal(item)}>
                  <span class="kwic-sub-link-title">{item.title}</span>
                </button>
              {/each}
            </div>
          {:else if subportalData.links.length > 0}
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
        {@const portalItems = mainLinks.items.filter(i => !i.url.includes(ICT_TAG) && !isCabinetPortal(i))}
        {@const teacherItem = portalItems.find(isTeacherQualificationPortal)}
        {@const certificateItem = portalItems.find(isCertificatePortal)}
        {@const hasCertificateSlot = portalItems.some(isCertificatePortal)}
        {@const cabinetItem = mainLinks.items.find(isCabinetPortal) ?? { title: "キャビネット", url: "/cabinet/reference" }}
        <section class="section">
          <div class="section-head-static">
            <span>ポータルリンク</span>
          </div>
          <div class="kwic-link-grid">
            {#each portalItems as item}
              {#if isTeacherQualificationPortal(item)}
                <button class="kwic-link-card" onclick={() => openMergedPortal("資格・手続き", [item, certificateItem])}>
                  <span class="kwic-link-title">資格・手続き</span>
                </button>
              {:else if isCertificatePortal(item)}
                <button class="kwic-link-card" onclick={() => openSubportal(cabinetItem)}>
                  <span class="kwic-link-title">キャビネット</span>
                </button>
              {:else}
                <button class="kwic-link-card" onclick={() => openSubportal(item)}>
                  <span class="kwic-link-title">{item.title}</span>
                </button>
              {/if}
            {/each}
            {#if !teacherItem && certificateItem}
              <button class="kwic-link-card" onclick={() => openMergedPortal("資格・手続き", [certificateItem])}>
                <span class="kwic-link-title">資格・手続き</span>
              </button>
            {/if}
            {#if !hasCertificateSlot}
              <button class="kwic-link-card" onclick={() => openSubportal(cabinetItem)}>
                <span class="kwic-link-title">キャビネット</span>
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
    position: relative;
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
    animation: dot-bounce 1.2s ease-in-out infinite;
  }

  .ai-loading-dots span:nth-child(2) { animation-delay: 0.15s; }
  .ai-loading-dots span:nth-child(3) { animation-delay: 0.3s; }

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

  .ob-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 10px 10px 14px;
    background: var(--bg-card);
    border: 1px solid var(--glass-border);
    border-radius: 14px;
    width: 50%;
    max-width: 480px;
  }
  .ob-banner-logo {
    width: 28px;
    height: 28px;
    object-fit: contain;
    flex-shrink: 0;
    user-select: none;
    -webkit-user-drag: none;
  }
  .ob-banner-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ob-banner-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.25;
  }
  .ob-banner-sub {
    font-size: 11px;
    color: var(--text-tertiary);
    line-height: 1.35;
  }
  .ob-banner-start {
    flex-shrink: 0;
    padding: 5px 12px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 7px;
    background: var(--accent);
    color: #fff;
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .ob-banner-start:hover { opacity: 0.9; }

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
