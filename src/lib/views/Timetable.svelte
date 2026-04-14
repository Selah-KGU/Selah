<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getScheduleSnapshot, syncScheduleData, aiGenerateSchedule, openSettingsWindow, gcalCheckSession, gcalSyncTimetable, gcalOpenLogin, gcalDisconnect, syncCalendar, getDataCache, saveDataCache, fetchSyllabusFavorites, fetchExamTimetable, openSyllabusDetail, refreshLunaCounts } from "../api";
  import { lunaAuthState, gcalAuthState, registerTask, updateTask, onCacheUpdate } from "../stores";
  import type { ExamEntry, ExamTimetableData, SyllabusEntry, SyllabusSearchResult } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import type { ScheduleResponse, AiScheduleItem, AiScheduleResult, KgcCourseRow, LunaCourseRow } from "../types";

  // ── State ──
  let loading = $state(true);
  let error = $state("");
  let scheduleData = $state<ScheduleResponse | null>(null);
  let aiResult = $state<AiScheduleResult | null>(null);
  let aiGenerating = $state(false);
  let aiError = $state("");
  let syncing = $state(false);
  let activeWeek = $state<"current" | "next">("current");
  let gcalSyncing = $state(false);
  let gcalError = $state("");
  let legendHover = $state(false);
  let examEntries = $state<ExamEntry[]>([]);
  let favoriteEntries = $state<SyllabusEntry[]>([]);
  let showFavInTimetable = $state(localStorage.getItem("selah-fav-in-timetable") === "1");
  const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
  let syscalEnabled = $state(isMac && localStorage.getItem("selah-syscal-enabled") === "true");
  let toast = $state<{ message: string; type: "success" | "error" | "info" } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let cacheReloadTimer: ReturnType<typeof setInterval> | undefined;
  let calSyncTimer: ReturnType<typeof setInterval> | undefined;
  let calSyncInitTimeout: ReturnType<typeof setTimeout> | undefined;
  let lunaCountsInitTimeout: ReturnType<typeof setTimeout> | undefined;
  let lunaCountsTimer: ReturnType<typeof setInterval> | undefined;

  // ── Constants ──
  const days: [number, string][] = [[1,"月"],[2,"火"],[3,"水"],[4,"木"],[5,"金"],[6,"土"]];
  const periods = [1, 2, 3, 4, 5];
  const periodTimes: Record<number, { start: string; end: string }> = {
    1: { start: "9:00", end: "10:30" },
    2: { start: "11:00", end: "12:30" },
    3: { start: "13:30", end: "15:00" },
    4: { start: "15:10", end: "16:40" },
    5: { start: "16:50", end: "18:20" },
  };

  // ── Derived ──
  let hasAi = $derived(!!aiResult && (aiResult.current_week.length > 0 || aiResult.next_week.length > 0));

  let kgcEntries = $derived.by(() => {
    if (!scheduleData) return [];
    return activeWeek === "current"
      ? scheduleData.raw.kgc_entries_current
      : scheduleData.raw.kgc_entries_next;
  });

  let weekLabelRaw = $derived(
    activeWeek === "current"
      ? (aiResult?.current_week_label || scheduleData?.raw.current_week_label || "")
      : (aiResult?.next_week_label || scheduleData?.raw.next_week_label || ""),
  );

  // "2026年04月07日～2026年04月11日" -> "26年 4/7-11"
  // Also handles "2026/04/07～2026/04/11", "20260407～20260411" etc.
  function shortWeekLabel(raw: string): string {
    if (!raw) return "";
    // Pattern 1: YYYY年MM月DD日～YYYY年MM月DD日
    let m = raw.match(/(\d{4})年0?(\d{1,2})月0?(\d{1,2})日[～〜~\-―]+(\d{4})年0?(\d{1,2})月0?(\d{1,2})日/);
    // Pattern 2: YYYY/MM/DD～YYYY/MM/DD
    if (!m) m = raw.match(/(\d{4})[\/\-]0?(\d{1,2})[\/\-]0?(\d{1,2})[～〜~\-―\s]+(\d{4})[\/\-]0?(\d{1,2})[\/\-]0?(\d{1,2})/);
    // Pattern 3: just extract any 8-digit date pairs
    if (!m) {
      const digits = raw.match(/(\d{4})(\d{2})(\d{2})[^\d]+(\d{4})(\d{2})(\d{2})/);
      if (digits) m = digits;
    }
    if (!m) {
      console.log("[Timetable] weekLabel no match:", JSON.stringify(raw));
      return raw;
    }
    const [, y1, sm1, sd1, , sm2, sd2] = m;
    const yShort = y1.slice(2);
    const startMonth = parseInt(sm1, 10);
    const startDay = parseInt(sd1, 10);
    const endMonth = parseInt(sm2, 10);
    const endDay = parseInt(sd2, 10);
    if (startMonth === endMonth) {
      return `${yShort} 年 ${startMonth} 月 ${startDay} - ${endDay} 日`;
    }
    return `${yShort} 年 ${startMonth} 月 ${startDay} 日 - ${endMonth} 月 ${endDay} 日`;
  }

  let weekLabel = $derived(shortWeekLabel(weekLabelRaw));

  // ── Day labels for i32↔String conversion ──
  const dayLabels = ["月", "火", "水", "木", "金", "土"];

  // ── Parse syllabus day_period like "月1", "月 1", "月1/木3", "月１" into [{day,period}] ──
  function parseDayPeriod(dp: string): { day: number; period: number }[] {
    const results: { day: number; period: number }[] = [];
    const parts = dp.split(/[\/,・]+/);
    for (const part of parts) {
      for (let i = 0; i < dayLabels.length; i++) {
        if (part.includes(dayLabels[i])) {
          const afterDay = part.slice(part.indexOf(dayLabels[i]) + 1).trim();
          const periodMatch = afterDay.match(/[1-6１-６]/);
          if (periodMatch) {
            const p = parseInt(periodMatch[0].replace(/[１２３４５６]/, c =>
              String("１２３４５６".indexOf(c) + 1)
            ), 10);
            if (p >= 1 && p <= 6) results.push({ day: i + 1, period: p });
          }
          break;
        }
      }
    }
    return results;
  }

  // Parsed favorite slots (reactive, rebuilt when favoriteEntries changes)
  let favSlots = $derived(
    showFavInTimetable
      ? favoriteEntries.flatMap(f => parseDayPeriod(f.day_period).map(slot => ({ ...slot, entry: f })))
      : []
  );

  let hasEntries = $derived(kgcEntries.length > 0 || (scheduleData?.raw.luna_courses.length ?? 0) > 0 || favSlots.length > 0);

  // ── Cell data ──
  interface CellData {
    kgc?: KgcCourseRow;
    luna?: LunaCourseRow;
    ai?: AiScheduleItem;
    exam?: ExamEntry;
    favorite?: SyllabusEntry;
    empty: boolean;
  }

  function getCell(day: number, period: number): CellData {
    const kgc = kgcEntries.find(e => e.day === day && e.period === period);
    const luna = scheduleData?.raw.luna_courses.find(c => c.day === day && c.period === period);
    const aiItems = aiResult
      ? (activeWeek === "current" ? aiResult.current_week : aiResult.next_week)
      : [];
    const ai = aiItems.find(i => i.day === day && i.period === period);
    const dayStr = dayLabels[day - 1];
    const exam = examEntries.find(e => e.day === dayStr && e.period === period);
    const fav = favSlots.find(f => f.day === day && f.period === period);
    return { kgc, luna, ai, exam, favorite: fav?.entry, empty: !kgc && !luna && !ai && !fav };
  }

  function cellName(c: CellData): string {
    return c.ai?.course_name || c.luna?.name || c.kgc?.name || c.favorite?.course_title || "";
  }

  function cellDotColor(c: CellData): string {
    if (c.kgc?.is_cancelled || c.ai?.is_cancelled) return "#ff3b30";
    if (c.kgc?.is_makeup) return "#34c759";
    if (c.kgc?.is_room_changed) return "#ff9500";
    if (c.favorite && !c.kgc && !c.luna && !c.ai) return "#ffcc00";
    return "var(--accent)";
  }

  // ── Actions ──
  async function handleCellClick(c: CellData) {
    const name = cellName(c);
    if (!name) return;
    if (c.luna && $lunaAuthState.authenticated) {
      try {
        await invoke("luna_open_detail_window", {
          path: "", title: name, mode: "course", idnumber: c.luna.luna_id,
          kgcPath: c.kgc?.detail_path || null,
        });
      } catch (e: any) { console.error("Failed to open Luna course:", e); }
    } else if (c.kgc?.detail_path) {
      try {
        await invoke("open_detail_window", { path: c.kgc.detail_path, courseName: name });
      } catch (e: any) { console.error("Failed to open detail:", e); }
    }
  }

  function openLunaCourse(idnumber: string, name: string) {
    invoke("luna_open_detail_window", {
      path: "", title: name, mode: "course", idnumber,
      kgcPath: null,
    }).catch(console.error);
  }

  // ── Toast helper ──
  function showToast(message: string, type: "success" | "error" | "info" = "success") {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 3000);
  }

  // ── Load exam + favorites (DB cache first, then network fallback) ──
  async function loadCachedExtras() {
    console.log("[Timetable] loadCachedExtras: start");

    // Exam
    try {
      const examJson = await getDataCache("exam_timetable");
      console.log("[Timetable] exam cache:", examJson ? `${examJson.length} chars` : "null");
      if (examJson) {
        const data: ExamTimetableData = JSON.parse(examJson);
        examEntries = data.entries || [];
      } else {
        const data = await fetchExamTimetable();
        examEntries = data.entries || [];
      }
      console.log("[Timetable] exam entries:", examEntries.length);
    } catch (e) {
      console.warn("[Timetable] exam load failed:", e);
    }

    // Favorites
    try {
      const favJson = await getDataCache("syllabus_favorites");
      console.log("[Timetable] favorites cache:", favJson ? `${favJson.length} chars` : "null");
      if (favJson) {
        const data: SyllabusSearchResult = JSON.parse(favJson);
        favoriteEntries = data.entries || [];
      } else {
        console.log("[Timetable] no favorites cache, fetching...");
        const data = await fetchSyllabusFavorites();
        favoriteEntries = data.entries || [];
      }
      console.log("[Timetable] favorites loaded:", favoriteEntries.length, favoriteEntries.map(f => ({ title: f.course_title, dp: f.day_period })));
    } catch (e) {
      console.warn("[Timetable] favorites load failed:", e);
    }

    console.log("[Timetable] favSlots parsed:", favSlots.length, favSlots.map(s => `${dayLabels[s.day-1]}${s.period}:${s.entry.course_title}`));
  }

  // ── Data loading (DB snapshot only — no network) ──
  async function loadData() {
    try {
      const data = await getScheduleSnapshot();
      console.log("[Timetable] snapshot loaded:", {
        kgc_current: data.raw.kgc_entries_current.length,
        kgc_next: data.raw.kgc_entries_next.length,
        luna: data.raw.luna_courses.length,
        week_label: data.raw.current_week_label,
        has_ai: !!data.ai_result,
        snapshot_updated_at: data.snapshot_updated_at,
      });
      scheduleData = data;
      aiResult = data.ai_result;

      // Auto-refresh AI when cache is stale
      if (data.ai_stale && data.ai_result && data.raw.current_week_label) {
        console.log("[Timetable] AI cache stale, triggering background refresh");
        triggerAiGenerate(true);
      }
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
    // Load exam + favorites from DB cache (non-blocking)
    loadCachedExtras();
  }

  // ── Simple hash for change detection ──
  function computeScheduleHash(entries: KgcCourseRow[]): string {
    const key = entries.map(e => `${e.kgc_code}:${e.day}:${e.period}:${e.is_cancelled}:${e.is_makeup}:${e.room}`).sort().join("|");
    let h = 0;
    for (let i = 0; i < key.length; i++) {
      h = ((h << 5) - h + key.charCodeAt(i)) | 0;
    }
    return String(h);
  }

  // ── Build CalendarSyncEntry[] from kgc entries ──
  function buildCalendarEntries(entries: KgcCourseRow[]): { day: string; period: number; course_name: string; room: string; is_cancelled: boolean }[] {
    return entries.map(e => ({
      day: dayLabels[e.day - 1] || String(e.day),
      period: e.period,
      course_name: e.name,
      room: e.room || "",
      is_cancelled: e.is_cancelled,
    }));
  }

  // ── Auto-sync to calendars if data changed ──
  async function autoSyncCalendars(entries: KgcCourseRow[], weekLabel: string) {
    const newHash = computeScheduleHash(entries);
    // Derive stable hash key from weekLabel (e.g. "2026/04/13") instead of activeWeek tab
    const weekId = weekLabel.slice(0, 10).replace(/\//g, "");
    const hashKey = `schedule_hash_${weekId}`;
    let changed = true;
    try {
      const oldHash = await getDataCache(hashKey);
      if (oldHash === newHash) changed = false;
    } catch {}

    // Always persist latest hash
    try { await saveDataCache(hashKey, newHash); } catch {}

    if (!changed) {
      console.log("[Timetable] schedule unchanged, skipping auto-sync");
      return;
    }

    const calEntries = buildCalendarEntries(entries);

    // System Calendar.app auto-sync (respect enabled setting)
    try {
      const syscalEnabled = localStorage.getItem("selah-syscal-enabled") === "true";
      const autoSync = localStorage.getItem("selah-auto-sync");
      if (syscalEnabled && autoSync === "true" && calEntries.length > 0) {
        await syncCalendar(calEntries, weekLabel);
        showToast("カレンダーに同期しました");
      }
    } catch (e: any) {
      console.error("[Timetable] Calendar.app auto-sync failed:", e);
    }

    // Google Calendar auto-sync
    try {
      const gcalAutoSync = localStorage.getItem("selah-gcal-auto-sync");
      if (gcalAutoSync === "true" && calEntries.length > 0) {
        const status = await gcalCheckSession();
        if (status.authenticated) {
          await gcalSyncTimetable(calEntries, weekLabel);
          showToast("Google カレンダーに同期しました");
        }
      }
    } catch (e: any) {
      console.error("[Timetable] Google Calendar auto-sync failed:", e);
    }
  }

  // ── Sync: serial fetch KGC → Luna → enrichment → persist ──
  async function handleSync() {
    if (syncing) return;
    syncing = true;
    error = "";
    try {
      const data = await syncScheduleData();
      console.log("[Timetable] sync done:", {
        kgc_current: data.raw.kgc_entries_current.length,
        kgc_next: data.raw.kgc_entries_next.length,
        luna: data.raw.luna_courses.length,
      });
      scheduleData = data;
      if (data.ai_result) aiResult = data.ai_result;
      showToast("時間割を更新しました");

      // Reload cached extras (exam might have updated)
      loadCachedExtras();

      // Auto-sync calendars (hash-based, non-blocking) - sync both weeks
      for (const week of ["current", "next"] as const) {
        const entries = week === "current" ? data.raw.kgc_entries_current : data.raw.kgc_entries_next;
        const label = week === "current" ? (data.raw.current_week_label || "") : (data.raw.next_week_label || "");
        if (entries.length > 0) {
          await autoSyncCalendars(entries, label);
        }
      }
      localStorage.setItem("selah-cal-last-sync", String(Date.now()));
    } catch (e: any) {
      error = e?.message || String(e);
      showToast(error, "error");
    } finally {
      syncing = false;
    }
  }

  async function triggerAiGenerate(force: boolean) {
    if (aiGenerating || !scheduleData) return;
    aiGenerating = true;
    aiError = "";
    console.log("[Timetable] triggerAiGenerate:", {
      force,
      currentWeekLabel: scheduleData.raw.current_week_label,
      nextWeekLabel: scheduleData.raw.next_week_label,
    });
    try {
      const result = await aiGenerateSchedule(
        scheduleData.raw.current_week_label,
        scheduleData.raw.next_week_label,
        force,
      );
      console.log("[Timetable] AI result:", {
        current_week: result.current_week.length,
        next_week: result.next_week.length,
        summary: result.weekly_summary?.substring(0, 80),
      });
      aiResult = result;
      tipIndex = 0;
      tipFade = true;
      startTipCycle();
    } catch (e: any) {
      const msg = e?.message || String(e);
      console.error("[Timetable] AI generation failed:", msg);
      aiError = msg.includes("APIキーが設定されていません") ? "api_key_missing" : msg;
    } finally {
      aiGenerating = false;
    }
  }

  // ── Google Calendar Sync ──
  async function handleGcalSync() {
    if (gcalSyncing || !scheduleData) return;
    gcalSyncing = true;
    gcalError = "";
    try {
      const weeks = [
        { entries: scheduleData.raw.kgc_entries_current, label: scheduleData.raw.current_week_label || "" },
        { entries: scheduleData.raw.kgc_entries_next, label: scheduleData.raw.next_week_label || "" },
      ];

      // Apple Calendar.app sync (respect enabled setting)
      const syscalEnabled = localStorage.getItem("selah-syscal-enabled") === "true";
      if (syscalEnabled) {
        for (const { entries: raw, label } of weeks) {
          const calEntries = buildCalendarEntries(raw);
          if (calEntries.length > 0) {
            try {
              await syncCalendar(calEntries, label);
            } catch (e: any) {
              console.error("[Timetable] Calendar.app sync failed:", e);
            }
          }
        }
      }

      // Google Calendar sync
      try {
        const status = await gcalCheckSession();
        if (!status.authenticated) {
          await gcalOpenLogin();
          gcalSyncing = false;
          return;
        }
        for (const { entries: raw, label } of weeks) {
          const calEntries = buildCalendarEntries(raw);
          if (calEntries.length > 0) {
            await gcalSyncTimetable(calEntries, label);
          }
        }
        const freshStatus = await gcalCheckSession();
        gcalAuthState.update(s => ({
          ...s,
          authenticated: freshStatus.authenticated,
          calendarExists: freshStatus.calendar_exists,
          syncedEvents: freshStatus.synced_events,
        }));
      } catch (e: any) {
        console.error("[Timetable] Google Calendar sync failed:", e);
      }

      showToast("カレンダーに同期しました");
    } catch (e: any) {
      gcalError = e?.message || String(e);
      showToast("カレンダー同期に失敗", "error");
    } finally {
      gcalSyncing = false;
    }
  }

  // ── Cell dynamic info cycling (3-screen design) ──
  interface CellFrame {
    index: number;
    lines: string[];
    deliveryMode?: string;
    hasNotify: boolean;
    hasExam: boolean;
    hasAssign: boolean;
    notifyCount: number;
    examCount: number;
    assignCount: number;
  }

  function cellFrames(c: CellData): CellFrame[] {
    const frames: CellFrame[] = [];

    // Screen 1: Room + Teacher + alert pills
    {
      const lines: string[] = [];
      const room = c.ai?.room || c.kgc?.room;
      const teacher = c.ai?.teacher || c.luna?.teacher;
      if (room) lines.push(room);
      if (teacher) lines.push(teacher);
      const hasNotify = !!(c.ai?.notifications?.length);
      const hasExam = !!(c.ai?.exams?.length);
      const hasAssign = !!(c.ai?.assignments?.length);
      const notifyCount = c.ai?.notifications?.length || 0;
      const examCount = c.ai?.exams?.length || 0;
      const assignCount = c.ai?.assignments?.length || 0;
      frames.push({ index: 0, lines, hasNotify, hasExam, hasAssign, notifyCount, examCount, assignCount });
    }

    // Screen 2: Delivery mode + Session topic
    {
      const dm = c.ai?.delivery_mode || "";
      const topic = c.ai?.session_topic || "";
      if (dm || topic) {
        frames.push({ index: 1, lines: topic ? [topic] : [], deliveryMode: dm, hasNotify: false, hasExam: false, hasAssign: false, notifyCount: 0, examCount: 0, assignCount: 0 });
      }
    }

    // Screen 3: Pending items — notifications + exams + assignments (always shown)
    {
      const lines: string[] = [];
      if (c.ai?.notifications?.length) lines.push(...c.ai.notifications);
      if (c.ai?.exams?.length) lines.push(...c.ai.exams);
      if (c.ai?.assignments?.length) lines.push(...c.ai.assignments);
      frames.push({ index: 2, lines, hasNotify: false, hasExam: false, hasAssign: false, notifyCount: 0, examCount: 0, assignCount: 0 });
    }

    return frames;
  }

  // Global cell info screen (manual switching)
  let cellInfoTick = $state(0);
  let cellInfoFade = $state(true);
  const screenLabels = ["概要", "授業", "未了"];
  let screenIdx = $derived(cellInfoTick % screenLabels.length);
  let screenLabel = $derived(screenLabels[screenIdx]);

  let cellInfoTimeout: ReturnType<typeof setTimeout> | undefined;

  function stepCellInfo() {
    cellInfoFade = false;
    clearTimeout(cellInfoTimeout);
    cellInfoTimeout = setTimeout(() => {
      cellInfoTick++;
      cellInfoFade = true;
    }, 200);
  }

  // ── Suggestion cycling (like HomePage) ──
  const defaultTips = [
    "授業形態(対面/オンライン)が正しいか確認しましょう",
    "休講・補講・教室変更がないかチェック",
    "未提出の課題やテストの締切を見逃さないように",
  ];

  let tipIndex = $state(0);
  let tipFade = $state(true);
  let tipInterval: ReturnType<typeof setInterval> | undefined;
  let tipTimeout: ReturnType<typeof setTimeout> | undefined;

  let tipSources = $derived.by(() => {
    if (hasAi && aiResult) {
      // Combine both summary fields for richer tips
      const parts = [aiResult.weekly_summary, aiResult.cross_week_insights].filter(Boolean).join("。");
      if (parts) {
        const sentences = parts.split(/[。！？\n]+/).map(s => s.trim()).filter(s => s.length > 2);
        if (sentences.length > 0) return sentences;
      }
    }
    return defaultTips;
  });

  let tipText = $derived(tipSources[tipIndex % tipSources.length] || defaultTips[0]);
  let isAiTip = $derived(hasAi && aiResult != null);

  function startTipCycle() {
    stopTipCycle();
    tipInterval = setInterval(() => {
      tipFade = false;
      tipTimeout = setTimeout(() => {
        tipIndex = (tipIndex + 1) % tipSources.length;
        tipFade = true;
      }, 400);
    }, 8000);
  }

  function stopTipCycle() {
    if (tipTimeout) {
      clearTimeout(tipTimeout);
      tipTimeout = undefined;
    }
    if (tipInterval) {
      clearInterval(tipInterval);
      tipInterval = undefined;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Tab" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      stepCellInfo();
    }
  }

  // ── Sunday auto-refresh ──
  // KGC server likely switches "current week" on Sunday, so cached Saturday
  // data becomes stale. Force one raw data refresh per Sunday automatically.
  async function checkSundayRefresh() {
    if (syncing) return;
    const now = new Date();
    if (now.getDay() !== 0) return; // not Sunday

    const todayKey = `${now.getFullYear()}-${now.getMonth()}-${now.getDate()}`;
    if (localStorage.getItem("selah-sunday-refreshed") === todayKey) return;

    console.log("[Timetable] Sunday auto-refresh triggered");
    syncing = true;
    try {
      const data = await syncScheduleData();
      scheduleData = data;
      if (data.ai_result) aiResult = data.ai_result;
      localStorage.setItem("selah-sunday-refreshed", todayKey);

      // Sync calendars with refreshed data (both weeks)
      for (const week of ["current", "next"] as const) {
        const entries = week === "current" ? data.raw.kgc_entries_current : data.raw.kgc_entries_next;
        const label = week === "current" ? (data.raw.current_week_label || "") : (data.raw.next_week_label || "");
        if (entries.length > 0) {
          await autoSyncCalendars(entries, label);
        }
      }
      localStorage.setItem("selah-cal-last-sync", String(Date.now()));
      console.log("[Timetable] Sunday auto-refresh completed");
    } catch (e) {
      console.error("[Timetable] Sunday auto-refresh failed:", e);
    } finally {
      syncing = false;
    }
  }

  // ── Luna activity counts auto-refresh (every 3 hours) ──
  const LUNA_COUNTS_INTERVAL = 3 * 3600 * 1000; // 3 hours

  async function autoRefreshLunaCounts() {
    updateTask("luna_counts", { running: true });
    try {
      const updated = await refreshLunaCounts();
      if (updated > 0) {
        console.log(`[Timetable] Luna counts refreshed: ${updated} courses`);
        // Reload snapshot to pick up new counts
        const data = await getScheduleSnapshot();
        scheduleData = data;
        if (data.ai_result) aiResult = data.ai_result;
      }
      updateTask("luna_counts", { running: false, lastRunTs: Date.now(), lastOk: true });
    } catch (e) {
      console.error("[Timetable] Luna counts auto-refresh failed:", e);
      updateTask("luna_counts", { running: false, lastRunTs: Date.now(), lastOk: false });
    }
  }

  // ── Timer-based calendar auto-sync ──
  const CAL_SYNC_CHECK_INTERVAL = 10 * 60 * 1000; // check every 10 minutes
  const CAL_SYNC_DEFAULT_HOURS = 12;
  const CAL_SYNC_MIN_HOURS = 6;
  const CAL_SYNC_MAX_HOURS = 72;

  function getCalSyncIntervalMs(): number {
    const raw = parseInt(localStorage.getItem("selah-cal-sync-interval") || "", 10);
    const hours = (Number.isFinite(raw) && raw >= CAL_SYNC_MIN_HOURS && raw <= CAL_SYNC_MAX_HOURS)
      ? raw : CAL_SYNC_DEFAULT_HOURS;
    return hours * 3600 * 1000;
  }

  async function timerCalendarSync() {
    // Check if any auto-sync is enabled
    const sysEnabled = localStorage.getItem("selah-syscal-enabled") !== "false";
    const sysAuto = localStorage.getItem("selah-auto-sync") === "true";
    const gcalAuto = localStorage.getItem("selah-gcal-auto-sync") === "true";
    if (!(sysEnabled && sysAuto) && !gcalAuto) return;

    // Check interval
    const lastSync = parseInt(localStorage.getItem("selah-cal-last-sync") || "0", 10);
    const now = Date.now();
    if (now - lastSync < getCalSyncIntervalMs()) return;

    console.log("[Timetable] timer-based calendar sync triggered");

    // Fetch fresh schedule data
    try {
      const data = await syncScheduleData();
      scheduleData = data;
      if (data.ai_result) aiResult = data.ai_result;

      // Sync both weeks
      for (const week of ["current", "next"] as const) {
        const entries = week === "current" ? data.raw.kgc_entries_current : data.raw.kgc_entries_next;
        const label = week === "current" ? (data.raw.current_week_label || "") : (data.raw.next_week_label || "");
        if (entries.length > 0) {
          await autoSyncCalendars(entries, label);
        }
      }

      localStorage.setItem("selah-cal-last-sync", String(now));
      console.log("[Timetable] timer-based calendar sync completed");
      updateTask("cal_sync", { running: false, lastRunTs: Date.now(), lastOk: true });
    } catch (e) {
      console.error("[Timetable] timer-based calendar sync failed:", e);
      updateTask("cal_sync", { running: false, lastRunTs: Date.now(), lastOk: false });
    }
  }

  function startCalSyncTimer() {
    registerTask("cal_sync", "カレンダー自動同期", "system", CAL_SYNC_CHECK_INTERVAL);
    registerTask("luna_counts", "Luna 活動カウント更新", "system", LUNA_COUNTS_INTERVAL);
    // Run once on mount after a short delay, then periodically
    calSyncInitTimeout = setTimeout(() => { updateTask("cal_sync", { running: true }); checkSundayRefresh(); timerCalendarSync(); }, 5000);
    calSyncTimer = setInterval(() => { updateTask("cal_sync", { running: true }); checkSundayRefresh(); timerCalendarSync(); }, CAL_SYNC_CHECK_INTERVAL);
  }

  function stopCalSyncTimer() {
    if (calSyncInitTimeout) { clearTimeout(calSyncInitTimeout); calSyncInitTimeout = undefined; }
    if (calSyncTimer) { clearInterval(calSyncTimer); calSyncTimer = undefined; }
  }

  const CACHE_RELOAD_FG = 30 * 60 * 1000; // 30 minutes (foreground)
  const CACHE_RELOAD_BG = 60 * 60 * 1000; // 1 hour (background)
  let lastCacheReload = 0;

  async function reloadFromCache() {
    try {
      const data = await getScheduleSnapshot();
      scheduleData = data;
      if (data.ai_result) aiResult = data.ai_result;
      loadCachedExtras();
      lastCacheReload = Date.now();
      console.log("[Timetable] cache reload done");
    } catch (e) {
      console.warn("[Timetable] cache reload failed:", e);
    }
  }

  function scheduleCacheReload() {
    if (cacheReloadTimer) clearInterval(cacheReloadTimer);
    const interval = document.visibilityState === "visible" ? CACHE_RELOAD_FG : CACHE_RELOAD_BG;
    cacheReloadTimer = setInterval(reloadFromCache, interval);
  }

  onMount(() => {
    loadData();
    lastCacheReload = Date.now();
    startTipCycle();
    startCalSyncTimer();
    // Periodic cache reload (no network, just re-read DB snapshot)
    scheduleCacheReload();
    // Luna activity counts: run once after 10s, then every 3 hours
    lunaCountsInitTimeout = setTimeout(() => autoRefreshLunaCounts(), 10_000);
    lunaCountsTimer = setInterval(() => autoRefreshLunaCounts(), LUNA_COUNTS_INTERVAL);
    window.addEventListener("keydown", handleKeydown);
    window.addEventListener("selah-fav-toggle", handleFavToggle as EventListener);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    // Check gcal session in background
    gcalCheckSession().then(status => {
      gcalAuthState.update(s => ({
        ...s,
        authenticated: status.authenticated,
        calendarExists: status.calendar_exists,
        syncedEvents: status.synced_events,
      }));
    }).catch(() => {});
  });

  function handleFavToggle(e: CustomEvent<boolean>) {
    showFavInTimetable = e.detail;
  }

  function handleVisibilityChange() {
    if (document.visibilityState === "visible") {
      syscalEnabled = isMac && localStorage.getItem("selah-syscal-enabled") !== "false";
      // Reload cache if stale when coming back to foreground
      if (Date.now() - lastCacheReload >= CACHE_RELOAD_FG) reloadFromCache();
    }
    scheduleCacheReload();
  }

  // SWR: pick up background poll refreshes
  const unsubExams = onCacheUpdate<ExamTimetableData>("exams", (fresh) => {
    examEntries = fresh?.entries || [];
  });

  onDestroy(() => {
    unsubExams();
    stopTipCycle();
    stopCalSyncTimer();
    if (cacheReloadTimer) clearInterval(cacheReloadTimer);
    if (lunaCountsTimer) clearInterval(lunaCountsTimer);
    if (lunaCountsInitTimeout) clearTimeout(lunaCountsInitTimeout);
    if (toastTimer) clearTimeout(toastTimer);
    clearTimeout(cellInfoTimeout);
    window.removeEventListener("keydown", handleKeydown);
    window.removeEventListener("selah-fav-toggle", handleFavToggle as EventListener);
    document.removeEventListener("visibilitychange", handleVisibilityChange);
  });
</script>

<div class="view">
  <!-- Header -->
  <div class="header">
    <div class="header-line1">
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <span
        class="header-date"
        onclick={() => activeWeek = activeWeek === "current" ? "next" : "current"}
      >
        <span class="header-date-content">
          <svg class="header-date-icon" width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="2" y="3" width="12" height="11" rx="2" stroke="currentColor" stroke-width="1.3"/>
            <path d="M2 6.5h12" stroke="currentColor" stroke-width="1.3"/>
            <path d="M5 1.5v3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
            <path d="M11 1.5v3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
          </svg>
          {weekLabel || (activeWeek === "current" ? "今週" : "来週")}
        </span>
        <span class="header-date-hint">{activeWeek === "current" ? "来週へ" : "今週へ"}</span>
      </span>
      <div class="toolbar-actions">
        {#if scheduleData}
          <button class="action-btn screen-toggle" onclick={() => stepCellInfo()}>
            {#if screenIdx === 0}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <rect x="2" y="2" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.2"/>
                <rect x="9" y="2" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.2"/>
                <rect x="2" y="9" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.2"/>
                <rect x="9" y="9" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.2"/>
              </svg>
            {:else if screenIdx === 1}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path d="M3 4h10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
                <path d="M3 8h7" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
                <path d="M3 12h5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
              </svg>
            {:else}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.2"/>
                <path d="M8 5v3.5l2.5 1.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            {/if}
            <span class="action-label">{screenLabel}</span>
            <kbd class="kbd-hint">Tab</kbd>
          </button>
          <button class="action-btn" onclick={handleSync} disabled={syncing}>
            {#if syncing}
              <span class="mini-spinner"></span>
            {:else}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path d="M13.5 2.5v4h-4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
                <path d="M2.5 13.5v-4h4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
                <path d="M4.3 6A5 5 0 0 1 13 4.5L13.5 6.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
                <path d="M11.7 10A5 5 0 0 1 3 11.5L2.5 9.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            {/if}
            <span class="action-label">更新</span>
          </button>
          <button class="action-btn" onclick={handleGcalSync} disabled={gcalSyncing} title={syscalEnabled && $gcalAuthState.authenticated ? "Apple / Google カレンダーに同期" : syscalEnabled ? "Apple カレンダーに同期" : "Google カレンダーに同期"}>
            {#if gcalSyncing}
              <span class="mini-spinner"></span>
            {:else}
              <span class="cal-logos">
                {#if syscalEnabled}
                <svg width="13" height="13" viewBox="0 0 814 1000" fill="currentColor">
                  <path d="M788.1 340.9c-5.8 4.5-108.2 62.2-108.2 190.5 0 148.4 130.3 200.9 134.2 202.2-.6 3.2-20.7 71.9-68.7 141.9-42.8 61.6-87.5 123.1-155.5 123.1s-85.5-39.5-164-39.5c-76.5 0-103.7 40.8-165.9 40.8s-105.6-57.8-155.5-127.4c-58.6-81.6-106.3-207.3-106.3-327.1 0-192.8 125.3-295.1 248.8-295.1 65.6 0 120.2 43.1 161.4 43.1 39.3 0 100.6-45.7 174.5-45.7 28.2 0 129.6 2.6 196.2 99.2z"/>
                  <path d="M554.1 159.4c31.5-37.7 53.8-90.1 53.8-142.6 0-7.3-.7-14.4-1.9-20.5-51.2 1.9-111.7 34.1-148.4 76.5-28.2 31.5-55.8 83.8-55.8 137.1 0 8 1.3 16 1.9 18.5 3.2.6 8.6 1.3 14 1.3 46.3-.1 103.7-30.7 136.4-70.3z"/>
                </svg>
                {/if}
                {#if $gcalAuthState.authenticated}
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
                  <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285F4"/>
                  <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853"/>
                  <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18A10.96 10.96 0 0 0 1 12c0 1.77.42 3.45 1.18 4.93l3.66-2.84z" fill="#FBBC05"/>
                  <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335"/>
                </svg>
                {/if}
              </span>
            {/if}
            <span class="action-label">同期</span>
          </button>
          <button class="action-btn action-btn-ai" onclick={() => triggerAiGenerate(true)} disabled={aiGenerating}>
            {#if aiGenerating}
              <span class="mini-spinner"></span>
            {:else}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path d="M8 1l1.5 3.5L13 6l-3.5 1.5L8 11 6.5 7.5 3 6l3.5-1.5z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>
                <path d="M12 10l.75 1.75L14.5 12.5l-1.75.75L12 15l-.75-1.75-1.75-.75 1.75-.75z" stroke="currentColor" stroke-width="0.9" stroke-linejoin="round" fill="none"/>
              </svg>
            {/if}
            <span class="action-label-ai">AI 日程</span>
          </button>
        {/if}
      </div>
    </div>
    <div class="header-line2">
      {#if isAiTip}
        <span class="header-greeting header-ai-tip" class:fade-in={tipFade} class:fade-out={!tipFade}>{tipText}</span>
      {:else}
        <span class="header-greeting" class:fade-in={tipFade} class:fade-out={!tipFade}>{tipText}</span>
      {/if}
    </div>
  </div>

  <!-- Error banners -->
  {#if aiError}
    <div class="error-banner">
      {#if aiError === "api_key_missing"}
        <span>AI 機能を利用するには API キーの設定が必要です</span>
        <button class="link-btn" onclick={() => openSettingsWindow()}>設定</button>
      {:else}
        <span>AI 分析に失敗: {aiError}</span>
      {/if}
    </div>
  {/if}
  {#if gcalError}
    <div class="error-banner">
      <span>カレンダー同期に失敗: {gcalError}</span>
      <button class="link-btn" onclick={() => gcalError = ""}>閉じる</button>
    </div>
  {/if}

  <ViewLoader {loading} {error} empty={!loading && !error && !hasEntries} emptyMessage="登録されている授業はありません">

    <!-- Grid timetable -->
    <div class="grid-outer">
      <div class="grid-wrap">
        <div class="timetable">
          <!-- Header row -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="grid-header corner"
            onmouseenter={() => legendHover = true}
            onmouseleave={() => legendHover = false}
          >
            <svg class="legend-info-icon" width="14" height="14" viewBox="0 0 16 16" fill="none">
              <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.2"/>
              <path d="M8 7v4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
              <circle cx="8" cy="5" r="0.8" fill="currentColor"/>
            </svg>
          </div>
        {#each days as [, label]}
          <div class="grid-header">{label}</div>
        {/each}

        <!-- Period rows -->
        {#each periods as period}
          <div class="grid-header period-num">
            <span class="period-label">{period}</span>
            <span class="period-time">{periodTimes[period].start}</span>
            <span class="period-time">{periodTimes[period].end}</span>
          </div>

          {#each days as [dayNum]}
            {@const cell = getCell(dayNum, period)}
            {#if cell.empty}
              <div class="cell"></div>
            {:else if cell.favorite && !cell.kgc && !cell.luna && !cell.ai}
              <!-- Favorite-only cell -->
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="cell course-cell entry-favorite"
                onclick={() => openSyllabusDetail(cell.favorite!.class_code, cell.favorite!.course_title)}
              >
                <span class="cell-dot" style="background:#ffcc00"></span>
                <div class="cell-frame cell-info-in">
                  <span class="cell-name"><span class="fav-star">★</span>{cell.favorite.course_title}</span>
                  <div class="cell-detail">
                    {#if cell.favorite.instructor}<div class="cell-info-line">{cell.favorite.instructor}</div>{/if}
                    {#if cell.favorite.campus}<div class="cell-info-line">{cell.favorite.campus}</div>{/if}
                  </div>
                  {#if cell.exam}<div class="cell-tags"><span class="cell-tag cell-tag-exam">試験</span></div>{/if}
                </div>
              </div>
            {:else}
              {@const frames = cellFrames(cell)}
              {@const isCancelled = !!(cell.kgc?.is_cancelled || cell.ai?.is_cancelled)}
              {@const isMakeup = !!cell.kgc?.is_makeup}
              {@const isChanged = !!cell.kgc?.is_room_changed}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="cell course-cell"
                class:entry-normal={!isCancelled && !isMakeup && !isChanged}
                class:entry-cancelled={isCancelled}
                class:entry-makeup={isMakeup}
                class:entry-changed={isChanged}
                onclick={() => handleCellClick(cell)}
              >
                <!-- Status indicator dot -->
                <span class="cell-dot" style="background:{cellDotColor(cell)}"></span>

                {#if frames.length > 0}
                {@const frameIdx = cellInfoTick % frames.length}
                {@const frame = frames[frameIdx]}
                <div class="cell-frame" class:cell-info-in={cellInfoFade} class:cell-info-out={!cellInfoFade}>
                  {#if frameIdx === 0}
                    <!-- Screen 1: Title + pills + info -->
                    <span class="cell-name" class:struck={isCancelled}>
                      {#if cell.favorite}<span class="fav-star">★</span>{/if}{cellName(cell)}
                    </span>
                    {#if isCancelled || isMakeup || isChanged || cell.exam || frame.hasNotify || frame.hasExam || frame.hasAssign}
                      <div class="cell-tags">
                        {#if isCancelled}<span class="cell-tag cell-tag-cancel">休講</span>{/if}
                        {#if isMakeup}<span class="cell-tag cell-tag-makeup">補講</span>{/if}
                        {#if isChanged}<span class="cell-tag cell-tag-change">変更</span>{/if}
                        {#if cell.exam}<span class="cell-tag cell-tag-exam">試験</span>{/if}
                        {#if frame.hasNotify}<span class="alert-pill alert-pill-notify">通知 {frame.notifyCount}</span>{/if}
                        {#if frame.hasExam}<span class="alert-pill alert-pill-exam">試験 {frame.examCount}</span>{/if}
                        {#if frame.hasAssign}<span class="alert-pill alert-pill-assign">課題 {frame.assignCount}</span>{/if}
                      </div>
                    {/if}
                    {#if frame.lines.length > 0}
                      <div class="cell-detail">
                        {#each frame.lines as line}
                          <div class="cell-info-line">{line}</div>
                        {/each}
                      </div>
                    {/if}
                  {:else if frame.index === 1}
                    <!-- Screen 2: Delivery + Topic -->
                    {#if frame.deliveryMode}
                      <span class="delivery-pill" class:delivery-face={frame.deliveryMode === '対面'} class:delivery-online={frame.deliveryMode === 'オンライン' || frame.deliveryMode === '同時双方向'} class:delivery-demand={frame.deliveryMode === 'オンデマンド'}>
                        {frame.deliveryMode}
                      </span>
                    {/if}
                    {#each frame.lines as line}
                      <span class="screen-topic">{line}</span>
                    {/each}
                  {:else}
                    <!-- Screen 3: Pending items -->
                    {#if frame.lines.length > 0}
                      <span class="screen-label screen-label-warn">未完了</span>
                      {#each frame.lines as line}
                        <span class="screen-content screen-content-warn">{line}</span>
                      {/each}
                    {:else}
                      <span class="screen-label screen-label-done">全完了</span>
                      <span class="screen-done-msg">未完了の課題・通知はありません</span>
                    {/if}
                  {/if}
                </div>
                {/if}
              </div>
            {/if}
          {/each}
        {/each}
      </div>
    </div>
    {#if legendHover}
      <div class="legend-tooltip" onmouseenter={() => legendHover = true} onmouseleave={() => legendHover = false}>
        <div class="legend-row"><span class="dot" style="background:var(--accent)"></span><span>通常授業</span></div>
        <div class="legend-row"><span class="dot" style="background:#ff3b30"></span><span>休講（この週は授業なし）</span></div>
        <div class="legend-row"><span class="dot" style="background:#34c759"></span><span>補講（追加の授業）</span></div>
        <div class="legend-row"><span class="dot" style="background:#ff9500"></span><span>教室変更あり</span></div>
      </div>
    {/if}
    </div>

    <!-- Communities -->
    {#if scheduleData?.luna_communities && scheduleData.luna_communities.length > 0}
      <div class="comm-section">
        <h3>コミュニティ</h3>
        <div class="comm-chips">
          {#each scheduleData.luna_communities as comm}
            <button class="comm-chip" onclick={() => openLunaCourse(comm.idnumber, comm.name)}>
              {comm.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </ViewLoader>
</div>

<!-- Toast notification (outside .view to avoid overflow clipping) -->
{#if toast}
  <div class="toast toast-{toast.type}">
    <span>{toast.message}</span>
    <button class="toast-close" onclick={() => toast = null}>&times;</button>
  </div>
{/if}

<style>
  /* ── Header (two-line layout like HomePage) ── */
  .header {
    display: flex;
    flex-direction: column;
    gap: 0;
    margin-bottom: 10px;
  }
  .header-line1 {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .header-date {
    position: relative;
    display: flex;
    align-items: center;
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
    cursor: pointer;
    user-select: none;
    border-radius: 6px;
    padding: 2px 6px 2px 2px;
    transition: background 0.2s ease;
    overflow: hidden;
  }
  .header-date:hover {
    background: var(--bg-hover);
  }
  .header-date:active {
    transform: scale(0.97);
    transition: transform 0.08s ease;
  }
  .header-date-content {
    display: flex;
    align-items: center;
    gap: 6px;
    transform: translateY(0);
    opacity: 1;
    transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.2s ease;
  }
  .header-date:hover .header-date-content {
    transform: translateY(-8px);
    opacity: 0;
  }
  .header-date-icon {
    color: var(--text-primary);
    flex-shrink: 0;
    display: block;
    position: relative;
    top: -0.5px;
  }
  .header-date-hint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--accent);
    transform: translateY(8px);
    opacity: 0;
    transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.2s ease;
    pointer-events: none;
  }
  .header-date:hover .header-date-hint {
    transform: translateY(0);
    opacity: 1;
  }
  .header-line2 {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .header-greeting {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
    transition: opacity 0.4s ease-in-out, transform 0.4s ease-in-out;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .header-ai-tip {
    color: var(--text-secondary);
  }
  .fade-in { opacity: 1; transform: translateY(0); }
  .fade-out { opacity: 0; transform: translateY(4px); }

  /* ── Toolbar actions ── */
  .toolbar-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
  }
  .screen-toggle {
    border: 0.5px solid var(--border);
    border-radius: 6px;
    background: var(--bg-card);
    min-width: 56px;
  }
  .screen-toggle:hover {
    background: var(--bg-hover);
  }
  .kbd-hint {
    font-size: 9px;
    font-weight: 500;
    font-family: inherit;
    padding: 1px 4px;
    border-radius: 3px;
    background: var(--bg-tertiary);
    color: var(--text-tertiary);
    border: 0.5px solid var(--border);
    line-height: 1.3;
    opacity: 0.7;
  }
  .action-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    border-radius: 7px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.12s ease;
    padding: 0 8px;
    position: relative;
    white-space: nowrap;
  }
  .action-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
  }
  .action-btn:active:not(:disabled) {
    transform: scale(0.95);
    background: var(--bg-active);
  }
  .action-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .action-label {
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.01em;
  }
  .action-btn-ai {
    color: rgba(175, 82, 222, 0.85);
  }
  .cal-logos {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  .action-btn-ai:hover:not(:disabled) {
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.08), rgba(0, 122, 255, 0.08));
    color: rgba(175, 82, 222, 0.95);
  }
  .action-label-ai {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.01em;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.85), rgba(0, 122, 255, 0.85));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  /* ── Error banner ── */
  .error-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    margin-bottom: 10px;
    border-radius: 10px;
    background: rgba(255, 149, 0, 0.08);
    border: 0.5px solid rgba(255, 149, 0, 0.2);
    font-size: 12px;
    color: var(--text-secondary);
  }
  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
  }

  /* ── Legend (icon in corner cell, tooltip outside grid-wrap) ── */
  .grid-outer {
    position: relative;
  }
  .corner {
    background: var(--bg-tertiary);
    cursor: help;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .corner:hover .legend-info-icon {
    color: var(--text-secondary);
  }
  .legend-info-icon {
    color: var(--text-tertiary);
    transition: color 0.15s ease;
  }
  .legend-tooltip {
    position: absolute;
    top: 38px;
    left: 4px;
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    box-shadow: var(--shadow-md);
    white-space: nowrap;
    font-size: 11px;
    color: var(--text-secondary);
    z-index: 30;
  }
  .legend-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .mini-spinner {
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  /* ── Grid ── */
  .grid-wrap {
    border-radius: 14px;
    overflow-x: auto;
    overflow-y: hidden;
    box-shadow: var(--shadow-md);
    animation: fade-in 0.3s ease both;
    -webkit-overflow-scrolling: touch;
  }
  @keyframes fade-in { from { opacity: 0; transform: translateY(6px); } }

  .timetable {
    min-width: 720px;
    display: grid;
    grid-template-columns: 68px repeat(6, 1fr);
    gap: 1px;
    background: var(--border);
  }

  /* ── Headers ── */
  .grid-header {
    background: var(--bg-secondary);
    padding: 9px 4px;
    text-align: center;
    font-weight: 600;
    font-size: 11.5px;
    color: var(--text-secondary);
    letter-spacing: 0.02em;
  }
  .period-num {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0;
    line-height: 1;
    padding: 4px 2px;
  }
  .period-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: 2px;
  }
  .period-time {
    font-size: 9px;
    font-weight: 400;
    color: var(--text-tertiary);
    opacity: 0.6;
    white-space: nowrap;
    line-height: 1.3;
  }

  /* ── Cell (magnet / Apple widget style) ── */
  .cell {
    background: var(--bg-card);
    display: flex;
    flex-direction: column;
    height: 140px;
    overflow: hidden;
    position: relative;
  }

  .course-cell {
    cursor: pointer;
    padding: 8px 8px 6px;
    justify-content: space-between;
    transition: filter 0.12s ease;
  }
  .course-cell:hover { filter: brightness(0.96); }
  .course-cell:active { filter: brightness(0.92); }

  .entry-normal   { background: color-mix(in srgb, var(--accent) 8%, var(--bg-card)); }
  .entry-cancelled { background: rgba(255, 59, 48, 0.06); }
  .entry-makeup   { background: rgba(52, 199, 89, 0.08); }
  .entry-changed  { background: rgba(255, 149, 0, 0.07); }

  /* Status dot — top right */
  .cell-dot {
    position: absolute;
    top: 7px;
    right: 7px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  /* Hero course name — large, dominant */
  .cell-name {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.3;
    letter-spacing: -0.01em;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    padding-right: 10px;
  }
  .struck { text-decoration: line-through; opacity: 0.45; }

  /* Status tags */
  .cell-tags {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .cell-tag {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 4px;
    line-height: 1.5;
  }
  .cell-tag-cancel { background: rgba(255, 59, 48, 0.12); color: #ff3b30; }
  .cell-tag-makeup { background: rgba(52, 199, 89, 0.12); color: #28a745; }
  .cell-tag-change { background: rgba(255, 149, 0, 0.12); color: #cc7700; }
  .cell-tag-exam { background: rgba(175, 82, 222, 0.12); color: #af52de; }
  .entry-favorite {
    background: #fffbeb;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .entry-favorite {
      background: rgba(255, 204, 0, 0.12);
    }
  }
  :global([data-theme="dark"]) .entry-favorite {
    background: rgba(255, 204, 0, 0.12);
  }

  /* Full-cell cycling frame */
  .cell-frame {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-height: 0;
    transition: opacity 0.3s ease, transform 0.3s ease;
  }
  .cell-info-in {
    opacity: 1;
    transform: translateY(0);
  }
  .cell-info-out {
    opacity: 0;
    transform: translateY(3px);
  }
  .cell-detail {
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin-top: auto;
  }
  .cell-info-line {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    line-height: 1.35;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Screen 2: Delivery mode pill + topic */
  .delivery-pill {
    display: inline-block;
    font-size: 10px;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 8px;
    line-height: 1.4;
    letter-spacing: 0.02em;
    align-self: flex-start;
  }
  .delivery-face {
    background: rgba(52, 199, 89, 0.12);
    color: #28a745;
  }
  .delivery-online {
    background: rgba(0, 122, 255, 0.12);
    color: #007aff;
  }
  .delivery-demand {
    background: rgba(175, 82, 222, 0.12);
    color: #af52de;
  }
  .screen-topic {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
    margin-top: 2px;
  }

  /* Screen 3: pending items */
  .screen-label {
    font-size: 9px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
    line-height: 1;
    margin-bottom: 2px;
  }
  .screen-label-warn {
    color: #ff9500;
  }
  .screen-content {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .screen-content-sub {
    font-weight: 500;
    font-size: 11px;
    color: var(--text-secondary);
    -webkit-line-clamp: 3;
  }
  .screen-content-warn {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    display: block;
    -webkit-line-clamp: unset;
    overflow: visible;
  }
  .screen-label-done {
    color: #34c759;
  }
  .screen-done-msg {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    margin-top: 2px;
  }
  .cell-alert-pills {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    margin-top: 2px;
  }
  .alert-pill {
    font-size: 9px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 8px;
    line-height: 1.5;
    white-space: nowrap;
  }
  .alert-pill-notify {
    background: rgba(255, 149, 0, 0.12);
    color: #ff9500;
  }
  .alert-pill-exam {
    background: rgba(255, 59, 48, 0.12);
    color: #ff3b30;
  }
  .alert-pill-assign {
    background: rgba(0, 122, 255, 0.12);
    color: #007aff;
  }

  /* ── Communities ── */
  .comm-section { margin-top: 16px; }
  .comm-section h3 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0 0 8px 0;
  }
  .comm-chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .comm-chip {
    padding: 6px 14px;
    border-radius: 20px;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s ease;
    box-shadow: var(--shadow-sm);
  }
  .comm-chip:hover { background: var(--bg-hover); box-shadow: var(--shadow-md); }

  /* ── Favorite star ── */
  .fav-star {
    color: #ffcc00;
    font-size: 10px;
    margin-right: 2px;
  }

  /* ── Toast notification ── */
  .toast {
    position: fixed;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    padding: 8px 16px;
    border-radius: 10px;
    font-size: 13px;
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 8px;
    z-index: 9999;
    animation: toast-slide-up 0.3s ease;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(12px);
  }
  .toast-success {
    background: rgba(52, 199, 89, 0.9);
    color: #fff;
  }
  .toast-error {
    background: rgba(255, 59, 48, 0.9);
    color: #fff;
  }
  .toast-info {
    background: rgba(0, 122, 255, 0.9);
    color: #fff;
  }
  .toast-close {
    background: none;
    border: none;
    color: inherit;
    font-size: 16px;
    cursor: pointer;
    padding: 0;
    line-height: 1;
    opacity: 0.7;
  }
  .toast-close:hover { opacity: 1; }
  @keyframes toast-slide-up {
    from { transform: translateX(-50%) translateY(20px); opacity: 0; }
    to { transform: translateX(-50%) translateY(0); opacity: 1; }
  }
</style>
