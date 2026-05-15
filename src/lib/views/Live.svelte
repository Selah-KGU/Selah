<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import { onCacheUpdate } from "../stores";
  import {
    getScheduleSnapshot,
    getAiConfig,
    isAiReady,
    liveAppendTranscript,
    liveCancelSession,
    liveClearDayCache,
    liveFinishSession,
    liveFlushSummary,
    liveGetSession,
    livePeekDayCache,
    liveStartSession,
    isDemoActive,
    openSettingsWindow,
    openSubtitleOverlay,
    closeSubtitleOverlay,
    saveLiveGeneratedTodos,
    type LiveCourseInfo,
    type LiveSaveResult,
    type LiveSessionSnapshot,
    type LiveTodoSuggestion,
  } from "../api";
  import type { ScheduleResponse } from "../types";
  import { DAY_NUM_LABELS, PERIOD_TIMES } from "../types";
  import { buildCourseSlots, getHeroCourses, type CourseSlot } from "../schedule";

  type NoticeKind = "error" | "success" | "warning";
  type NoticeSource = "general" | "readiness" | "stt";
  type NoticeAction = "open-ai-settings";
  type SttPhase = "idle" | "checking" | "starting" | "initializing" | "listening";
  type NoticeState = {
    kind: NoticeKind;
    text: string;
    source: NoticeSource;
    action?: NoticeAction;
  } | null;
  type LiveTodoDraft = LiveTodoSuggestion & { selected: boolean };

  let scheduleData = $state<ScheduleResponse | null>(null);
  let allCourseOptions = $state<CourseSlot[]>([]);
  let courseOptions = $state<CourseSlot[]>([]);
  let selectedKey = $state("");
  let snapshot = $state<LiveSessionSnapshot>({
    active: false,
    course: null,
    started_at: null,
    transcript_lines: [],
    pending_lines: [],
    summaries: [],
  });
  let partialText = $state("");
  let sttListening = $state(false);
  let sttPhase = $state<SttPhase>("idle");
  let busy = $state(false);
  let pageLoading = $state(true);
  let notice = $state<NoticeState>(null);
  let liveReady = $state(false);
  let lastSaved = $state<LiveSaveResult | null>(null);
  let showSaveNotif = $state(false);
  let saveProgress = $state("");
  let todoDrafts = $state<LiveTodoDraft[]>([]);
  let todoDraftSourcePath = $state("");
  let todoDraftSaving = $state(false);
  let summaryViewIndex = $state(-1); // -1 = auto (latest)
  let summaryExpanded = $state(false);
  let flushTimer: ReturnType<typeof setInterval> | null = null;
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;
  let scheduleFocusTimer: ReturnType<typeof setInterval> | null = null;
  let liveSummaryIntervalMinutes = $state(5);
  let timeTimer: ReturnType<typeof setInterval> | null = null;
  let now = $state(new Date());
  let scrollEl: HTMLElement | null = null;
  const FREE_NOTE_NAME = "自由ノート";
  const MIN_AI_SUMMARIZATION_MS = 2 * 60 * 1000;
  const NO_EFFECTIVE_SPEECH_AUTO_PAUSE_MS = 10 * 60 * 1000;
  const PAUSED_AUTO_FINISH_MS = 20 * 60 * 1000;
  const LIVE_AUTO_GUARD_INTERVAL_MS = 60 * 1000;
  let pendingActivationMode: "start" | "resume" | null = null;
  let cancelSessionOnStartFailure = false;
  let lastEffectiveSpeechAtMs: number | null = null;
  let pausedSinceMs: number | null = null;
  let liveAutoGuardTimer: ReturnType<typeof setInterval> | null = null;
  let autoLifecycleBusy = false;

  marked.setOptions({ breaks: true, gfm: true });

  const renderMdCache = new Map<string, string>();
  const RENDER_MD_CACHE_MAX = 128;
  function renderMd(text: string): string {
    const cached = renderMdCache.get(text);
    if (cached !== undefined) return cached;
    const out = DOMPurify.sanitize(marked.parse(text) as string);
    if (renderMdCache.size >= RENDER_MD_CACHE_MAX) {
      const firstKey = renderMdCache.keys().next().value;
      if (firstKey !== undefined) renderMdCache.delete(firstKey);
    }
    renderMdCache.set(text, out);
    return out;
  }

  function extractOverallSummary(md: string): string {
    const start = md.indexOf("### 全体要約");
    if (start < 0) return "";
    const afterHeader = md.indexOf("\n", start);
    if (afterHeader < 0) return "";
    const nextSection = md.indexOf("\n###", afterHeader + 1);
    const end = nextSection >= 0 ? nextSection : md.indexOf("\n## ", afterHeader + 1);
    return (end >= 0 ? md.slice(afterHeader + 1, end) : md.slice(afterHeader + 1)).trim();
  }

  function snapshotStartedAtMs(value: string | null | undefined): number | null {
    if (!value) return null;
    const parsed = new Date(value.replace(" ", "T")).getTime();
    return Number.isFinite(parsed) ? parsed : null;
  }

  function shouldSkipAiSummarizationForSnapshot(current: LiveSessionSnapshot): boolean {
    const startedAtMs = snapshotStartedAtMs(current.started_at);
    if (startedAtMs == null) return false;
    return Date.now() - startedAtMs < MIN_AI_SUMMARIZATION_MS;
  }

  function expandSummary() {
    summaryExpanded = true;
  }

  function collapseSummary() {
    summaryExpanded = false;
  }

  function selectSummaryView(event: MouseEvent, idx: number) {
    event.stopPropagation();
    summaryViewIndex = idx;
  }

  function handleSummaryOverlayClick(event: MouseEvent) {
    const target = event.target;
    if (target instanceof Element && target.closest("button, a")) return;
    collapseSummary();
  }

  function bindSummaryOverlayDismiss(node: HTMLDivElement) {
    const onDismiss = (event: Event) => {
      if (event instanceof MouseEvent) {
        handleSummaryOverlayClick(event);
      }
    };
    node.addEventListener("click", onDismiss);
    return {
      destroy() {
        node.removeEventListener("click", onDismiss);
      }
    };
  }

  const activeSummaryIdx = $derived(
    summaryViewIndex < 0 || summaryViewIndex >= snapshot.summaries.length
      ? snapshot.summaries.length - 1
      : summaryViewIndex
  );

  let unlistenPartial: (() => void) | null = null;
  let unlistenFinal: (() => void) | null = null;
  let unlistenState: (() => void) | null = null;
  let unlistenError: (() => void) | null = null;
  let unlistenInfo: (() => void) | null = null;
  let unlistenLive: (() => void) | null = null;
  let unlistenSaved: (() => void) | null = null;
  let unlistenAiConfig: (() => void) | null = null;
  let unlistenScheduleCache: (() => void) | null = null;
  let unlistenWinFocus: (() => void) | null = null;
  let unlistenWinBlur: (() => void) | null = null;

  const hasContent = $derived(snapshot.transcript_lines.length > 0 || partialText.trim().length > 0);
  const sttBooting = $derived(
    sttPhase === "checking" || sttPhase === "starting" || sttPhase === "initializing"
  );
  const sttBootMessage = $derived.by(() => {
    switch (sttPhase) {
      case "checking":
        return "音声入力モデルを確認中…";
      case "starting":
        return "音声入力を起動中…";
      case "initializing":
        return "マイクと音声認識を初期化中…";
      default:
        return "";
    }
  });
  const liveBadgeLabel = $derived.by(() => {
    if (!snapshot.active) return "LIVE";
    if (sttBooting) return "準備中";
    if (sttPhase === "listening") return "REC";
    return saveProgress ? "処理中" : "一時停止";
  });

  const remainingLabel = $derived.by(() => {
    if (!snapshot.active || !snapshot.course) return "";
    if (snapshot.course.is_free_note) return "";
    const period = snapshot.course.period;
    const pt = PERIOD_TIMES[period];
    if (pt) {
      const endMs = new Date(now.getFullYear(), now.getMonth(), now.getDate(), pt.endH, pt.endM).getTime();
      const diff = endMs - now.getTime();
      if (diff > 0) {
        const totalMin = Math.ceil(diff / 60000);
        const h = Math.floor(totalMin / 60);
        const m = totalMin % 60;
        if (h > 0) return `残 ${h}:${String(m).padStart(2, '0')}`;
        return `残 ${m}分`;
      }
      return "終了";
    }
    return now.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
  });

  let autoFollow = $state(true);
  let showScrollBtn = $derived(sttListening && !autoFollow);
  let confirmClear = $state(false);
  let lastAppliedLen = $state(0);

  const VISIBLE_LINE_WINDOW = 120;
  const visibleLines = $derived.by(() => {
    const lines = snapshot.transcript_lines;
    if (lines.length <= VISIBLE_LINE_WINDOW) return lines;
    return lines.slice(lines.length - VISIBLE_LINE_WINDOW);
  });
  const hiddenLineCount = $derived(
    Math.max(0, snapshot.transcript_lines.length - visibleLines.length)
  );

  /** User deliberately scrolled — unlock auto-follow while streaming. */
  function handleUserScroll() {
    if (!scrollEl || !sttListening) return;
    autoFollow = false;
  }

  function bindManualScroll(node: HTMLDivElement) {
    const onUserScroll = () => handleUserScroll();
    node.addEventListener("wheel", onUserScroll);
    node.addEventListener("touchmove", onUserScroll);
    return {
      destroy() {
        node.removeEventListener("wheel", onUserScroll);
        node.removeEventListener("touchmove", onUserScroll);
      }
    };
  }

  function scrollToBottom() {
    if (!scrollEl) return;
    autoFollow = true;
    scrollEl.scrollTop = scrollEl.scrollHeight;
  }

  $effect(() => {
    // Only run the clock while a session is active. When idle the badge
    // doesn't display remaining time, so waking the event loop is wasted.
    // 30s tick: the badge is minute-resolution ("残 X 分"), so anything
    // tighter just burns power re-deriving the same string.
    if (snapshot.active) {
      if (!timeTimer) {
        now = new Date();
        timeTimer = setInterval(() => { now = new Date(); }, 30_000);
      }
      if (!liveAutoGuardTimer) {
        liveAutoGuardTimer = setInterval(() => {
          checkLiveAutoLifecycle().catch((e: any) => {
            console.warn("[Live] auto lifecycle check failed:", e);
          });
        }, LIVE_AUTO_GUARD_INTERVAL_MS);
      }
    } else {
      if (timeTimer) {
        clearInterval(timeTimer);
        timeTimer = null;
      }
      stopLiveAutoGuardTimer();
      clearLiveAutoLifecycle();
    }
  });

  let lastScrolledLen = $state(-1);
  $effect(() => {
    const len = snapshot.transcript_lines.length;
    if (!scrollEl || !autoFollow || !sttListening) return;
    // Only schedule a scroll when the line count actually changes; partial
    // text churn would otherwise trigger rAF on every 600ms decode.
    if (len === lastScrolledLen) return;
    lastScrolledLen = len;
    requestAnimationFrame(() => {
      if (!scrollEl || !autoFollow) return;
      scrollEl.scrollTop = scrollEl.scrollHeight;
    });
  });

  const selectedCourse = $derived.by(() => {
    if (!selectedKey) return null;
    return courseOptions.find((course) => courseKey(course) === selectedKey) ?? null;
  });

  const renderedCourseOptions = $derived.by(() => {
    const day = courseOptions[0]?.day;
    if (day == null) return courseOptions;
    return courseOptions.filter((course) => course.day === day);
  });

  const canStart = $derived(!snapshot.active && !!selectedCourse && liveReady && !busy);
  const canStartFreeNote = $derived(!snapshot.active && liveReady && !busy);
  const canStop = $derived(snapshot.active && !busy);

  // When the selected course changes (and session not active), load cached history
  $effect(() => {
    const course = selectedCourse;
    if (snapshot.active || !course || showSaveNotif) return;
    livePeekDayCache(toLiveCourse(course)).then((cached) => {
      if (snapshot.active || showSaveNotif) return;
      if (cached.transcript_lines.length > 0 || cached.summaries.length > 0) {
        snapshot = cached;
      } else if (snapshot.course) {
        snapshot = { active: false, course: null, started_at: null, transcript_lines: [], pending_lines: [], summaries: [] };
      }
    }).catch(() => {});
  });

  function courseKey(course: CourseSlot): string {
    return `${course.day}-${course.period}-${course.kgc_code || course.name}`;
  }

  function courseLabel(course: CourseSlot): string {
    const time = PERIOD_TIMES[course.period];
    const day = DAY_NUM_LABELS[course.day] ?? `${course.day}`;
    const timeLabel = time ? `${time.start}-${time.end}` : `${course.period}限`;
    const meta = [day, `${course.period}限`, timeLabel].filter(Boolean).join(" ");
    return `${course.name} (${meta})`;
  }

  function toLiveCourse(course: CourseSlot): LiveCourseInfo {
    const time = PERIOD_TIMES[course.period];
    return {
      course_name: course.name,
      course_code: course.kgc_code,
      room: course.room,
      teacher: course.teacher,
      day: course.day,
      period: course.period,
      time_label: time ? `${time.start}-${time.end}` : "",
      is_free_note: false,
    };
  }

  function createFreeNoteCourse(): LiveCourseInfo {
    return {
      course_name: FREE_NOTE_NAME,
      course_code: "",
      room: "",
      teacher: "",
      day: 0,
      period: 0,
      time_label: "",
      is_free_note: true,
    };
  }

  function todayDayNumber(date: Date): number {
    const jsDow = date.getDay();
    return jsDow === 0 ? 7 : jsDow;
  }

  function closestCourseForNow(courses: CourseSlot[], date: Date): CourseSlot | null {
    if (!courses.length) return null;
    const nowMin = date.getHours() * 60 + date.getMinutes();
    const ranked = courses
      .map((course) => {
        const time = PERIOD_TIMES[course.period];
        if (!time) return { course, distance: Number.MAX_SAFE_INTEGER, startMin: Number.MAX_SAFE_INTEGER };
        const startMin = time.startH * 60 + time.startM;
        const endMin = time.endH * 60 + time.endM;
        let distance = 0;
        if (nowMin < startMin) distance = startMin - nowMin;
        else if (nowMin > endMin) distance = nowMin - endMin;
        return { course, distance, startMin };
      })
      .sort((a, b) => a.distance - b.distance || a.startMin - b.startMin);
    return ranked[0]?.course ?? null;
  }

  function defaultCourseForVisibleOptions(courses: CourseSlot[], date: Date): CourseSlot | null {
    if (!courses.length) return null;
    const visibleDay = courses[0]?.day;
    if (visibleDay == null) return courses[0] ?? null;
    if (visibleDay === todayDayNumber(date)) {
      return closestCourseForNow(courses, date) ?? courses[0] ?? null;
    }
    return [...courses].sort((a, b) => a.period - b.period || a.name.localeCompare(b.name))[0] ?? null;
  }

  function chooseFocusedCourseOptions(courses: CourseSlot[], date: Date): CourseSlot[] {
    if (!courses.length) return [];
    const today = todayDayNumber(date);
    const nowMin = date.getHours() * 60 + date.getMinutes();

    const grouped = new Map<number, CourseSlot[]>();
    for (const course of courses) {
      if (!grouped.has(course.day)) grouped.set(course.day, []);
      grouped.get(course.day)!.push(course);
    }
    for (const list of grouped.values()) {
      list.sort((a, b) => a.period - b.period || a.name.localeCompare(b.name));
    }

    const todayCourses = grouped.get(today) ?? [];
    const hasRemainingToday = todayCourses.some((course) => {
      const time = PERIOD_TIMES[course.period];
      if (!time) return false;
      const endMin = time.endH * 60 + time.endM;
      return nowMin <= endMin;
    });
    if (todayCourses.length > 0 && hasRemainingToday) {
      return todayCourses;
    }

    for (let offset = 1; offset <= 7; offset++) {
      const day = ((today - 1 + offset) % 7) + 1;
      const nextCourses = grouped.get(day) ?? [];
      if (nextCourses.length > 0) return nextCourses;
    }

    return todayCourses.length > 0
      ? todayCourses
      : [...courses].sort((a, b) => a.day - b.day || a.period - b.period || a.name.localeCompare(b.name));
  }

  function clearNoticeTimer() {
    if (noticeTimer) {
      clearTimeout(noticeTimer);
      noticeTimer = null;
    }
  }

  function clearNotice() {
    clearNoticeTimer();
    notice = null;
  }

  function setNotice(
    kind: NoticeKind,
    text: string,
    options: {
      source?: NoticeSource;
      action?: NoticeAction;
      autoClearMs?: number;
    } = {},
  ) {
    clearNoticeTimer();
    const source = options.source ?? "general";
    notice = {
      kind,
      text,
      source,
      action: options.action,
    };
    if (options.autoClearMs && options.autoClearMs > 0) {
      const expected = { kind, text, source };
      noticeTimer = setTimeout(() => {
        if (
          notice &&
          notice.kind === expected.kind &&
          notice.text === expected.text &&
          notice.source === expected.source
        ) {
          notice = null;
        }
        noticeTimer = null;
      }, options.autoClearMs);
    }
  }

  function setMessage(kind: "error" | "success", message: string) {
    if (kind === "error") {
      setNotice("error", message);
      return;
    }
    setNotice("success", message, { autoClearMs: 4000 });
  }

  function setReadinessNotice(message: string) {
    if (notice && notice.source !== "readiness" && notice.kind === "error") return;
    setNotice("warning", message, {
      source: "readiness",
      action: "open-ai-settings",
    });
  }

  function clearReadinessNotice() {
    if (notice?.source === "readiness") {
      clearNotice();
    }
  }

  function setSttNotice(message: string) {
    if (notice?.source === "readiness" && notice.kind === "error") return;
    setNotice("warning", message, { source: "stt" });
  }

  function clearSttNotice() {
    if (notice?.source === "stt") {
      clearNotice();
    }
  }

  function buildReadinessMessage(
    cfg: { ai_enabled: boolean; provider: string; api_key?: string },
    ready: boolean,
  ): string {
    if (cfg.ai_enabled === false) {
      return "AIが無効です。LIVEを使うには設定でAIを有効にしてください。";
    }
    if (cfg.provider === "local" && !ready) {
      return "ローカルAIモデルの準備ができていません。AI設定でモデルを確認してください。";
    }
    if (!cfg.api_key?.trim()) {
      return "APIキーが未設定です。LIVEを使うにはAI設定を完了してください。";
    }
    return "LIVEにはAIの準備が必要です。AI設定を確認してください。";
  }

  function applyScheduleSnapshot(data: ScheduleResponse, date: Date = new Date(), preserveSelection = true) {
    scheduleData = data;
    const slots = buildCourseSlots(scheduleData).filter((course) => !course.is_cancelled);
    allCourseOptions = [...slots].sort((a, b) => a.day - b.day || a.period - b.period || a.name.localeCompare(b.name));
    const focused = chooseFocusedCourseOptions(allCourseOptions, date);
    const focusedDay = focused[0]?.day;
    courseOptions = focusedDay != null
      ? focused.filter((course) => course.day === focusedDay)
      : focused;
    console.log("[LIVE] allCourseOptions =", allCourseOptions.map((c) => ({ day: c.day, period: c.period, name: c.name })));
    console.log("[LIVE] focusedCourseOptions =", courseOptions.map((c) => ({ day: c.day, period: c.period, name: c.name })));
    if (snapshot.active && snapshot.course) {
      const match = courseOptions.find((course) =>
        course.name === snapshot.course?.course_name &&
        course.period === snapshot.course?.period &&
        course.day === snapshot.course?.day,
      );
      if (match) {
        selectedKey = courseKey(match);
        return;
      }
      const allMatch = allCourseOptions.find((course) =>
        course.name === snapshot.course?.course_name &&
        course.period === snapshot.course?.period &&
        course.day === snapshot.course?.day,
      );
      if (allMatch) {
        courseOptions = allCourseOptions.filter((course) => course.day === allMatch.day);
        selectedKey = courseKey(allMatch);
        return;
      }
    }
    if (preserveSelection && courseOptions.some((course) => courseKey(course) === selectedKey)) {
      return;
    }
    const nearest = defaultCourseForVisibleOptions(courseOptions, date);
    if (nearest) {
      selectedKey = courseKey(nearest);
      return;
    }
    const hero = getHeroCourses(courseOptions, date);
    selectedKey = hero[0] ? courseKey(hero[0].entry) : (courseOptions[0] ? courseKey(courseOptions[0]) : "");
  }

  async function refreshSchedule(preserveSelection = true) {
    applyScheduleSnapshot(await getScheduleSnapshot(), new Date(), preserveSelection);
  }

  function refreshFocusedCoursesFromClock() {
    const current = new Date();
    now = current;
    if (!scheduleData || snapshot.active) return;
    applyScheduleSnapshot(scheduleData, current, true);
  }

  async function refreshReadiness() {
    const cfg = await getAiConfig();
    liveSummaryIntervalMinutes = Math.min(30, Math.max(5, cfg.live_summary_interval_minutes ?? 5));
    const ready = await isAiReady();
    liveReady = ready;
    if (liveReady) {
      clearReadinessNotice();
      return;
    }
    setReadinessNotice(buildReadinessMessage(cfg, ready));
  }

  async function ensureReadyToStart() {
    await refreshReadiness();
    if (!liveReady) {
      throw new Error(notice?.source === "readiness" ? notice.text : "AIの準備ができていません");
    }
  }

  function markLiveListeningStarted() {
    lastEffectiveSpeechAtMs = Date.now();
    pausedSinceMs = null;
  }

  function markEffectiveSpeech() {
    lastEffectiveSpeechAtMs = Date.now();
    pausedSinceMs = null;
  }

  function markLivePaused() {
    if (!snapshot.active) return;
    if (!pausedSinceMs) pausedSinceMs = Date.now();
    lastEffectiveSpeechAtMs = null;
  }

  function clearLiveAutoLifecycle() {
    lastEffectiveSpeechAtMs = null;
    pausedSinceMs = null;
    autoLifecycleBusy = false;
  }

  function stopLiveAutoGuardTimer() {
    if (liveAutoGuardTimer) {
      clearInterval(liveAutoGuardTimer);
      liveAutoGuardTimer = null;
    }
  }

  async function checkLiveAutoLifecycle() {
    if (!snapshot.active || busy || autoLifecycleBusy) return;
    const nowMs = Date.now();
    if (sttListening && !sttBooting) {
      const lastEffectiveAt = lastEffectiveSpeechAtMs ?? nowMs;
      lastEffectiveSpeechAtMs = lastEffectiveAt;
      pausedSinceMs = null;
      if (nowMs - lastEffectiveAt >= NO_EFFECTIVE_SPEECH_AUTO_PAUSE_MS) {
        autoLifecycleBusy = true;
        try {
          await pauseLiveInternal(true);
        } finally {
          autoLifecycleBusy = false;
        }
      }
      return;
    }

    if (!sttBooting) {
      const pausedAt = pausedSinceMs ?? nowMs;
      pausedSinceMs = pausedAt;
      if (nowMs - pausedAt >= PAUSED_AUTO_FINISH_MS) {
        autoLifecycleBusy = true;
        try {
          await stopLiveInternal(true);
        } finally {
          autoLifecycleBusy = false;
        }
      }
    }
  }

  async function startSession(course: LiveCourseInfo) {
    busy = true;
    clearNotice();
    sttListening = false;
    sttPhase = "checking";
    setSttNotice("音声入力モデルを確認中…");
    pendingActivationMode = "start";
    cancelSessionOnStartFailure = true;
    try {
      await ensureReadyToStart();
      sttPhase = "starting";
      setSttNotice("音声入力を起動中…");
      snapshot = await liveStartSession(course);
      partialText = "";
      lastSaved = null;
      if (isDemoActive()) {
        sttListening = true;
        sttPhase = "listening";
        markLiveListeningStarted();
        clearSttNotice();
      } else {
        await invoke("stt_start_stream", { caller: "live" });
      }
      autoFollow = true;
      startFlushTimer();
    } catch (e: any) {
      pendingActivationMode = null;
      cancelSessionOnStartFailure = false;
      sttPhase = "idle";
      clearSttNotice();
      setMessage("error", e?.message || String(e));
      try {
        await liveCancelSession();
        snapshot = await liveGetSession();
      } catch {}
      stopFlushTimer();
      clearLiveAutoLifecycle();
    } finally {
      busy = false;
    }
  }

  async function startLive() {
    if (!selectedCourse) return;
    await startSession(toLiveCourse(selectedCourse));
  }

  async function startFreeNote() {
    await startSession(createFreeNoteCourse());
  }

  async function pauseLiveInternal(automated = false) {
    busy = true;
    clearNotice();
    clearSttNotice();
    pendingActivationMode = null;
    cancelSessionOnStartFailure = false;
    try {
      if (!isDemoActive()) {
        try {
          await invoke("stt_stop_stream");
        } catch {}
      }
      sttListening = false;
      sttPhase = "idle";
      partialText = "";
      stopFlushTimer();
      markLivePaused();
      if (automated) {
        setNotice("warning", "10分間有効な音声が認識されなかったため、LIVEを一時停止しました。");
      } else {
        setMessage("success", `LIVEを一時停止: ${snapshot.course?.course_name ?? "録音"}`);
      }
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  async function pauseLive() {
    await pauseLiveInternal(false);
  }

  async function resumeLive() {
    if (!snapshot.active) return;
    busy = true;
    clearNotice();
    sttListening = false;
    sttPhase = "checking";
    setSttNotice("音声入力モデルを確認中…");
    pendingActivationMode = "resume";
    cancelSessionOnStartFailure = false;
    try {
      await ensureReadyToStart();
      sttPhase = "starting";
      setSttNotice("音声入力を起動中…");
      if (isDemoActive()) {
        sttListening = true;
        sttPhase = "listening";
        markLiveListeningStarted();
        clearSttNotice();
      } else {
        await invoke("stt_start_stream", { caller: "live" });
      }
      autoFollow = true;
      startFlushTimer();
    } catch (e: any) {
      pendingActivationMode = null;
      cancelSessionOnStartFailure = false;
      sttPhase = "idle";
      markLivePaused();
      clearSttNotice();
      setMessage("error", e?.message || String(e));
      stopFlushTimer();
    } finally {
      busy = false;
    }
  }

  async function stopLiveInternal(automated = false) {
    busy = true;
    clearNotice();
    clearSttNotice();
    pendingActivationMode = null;
    cancelSessionOnStartFailure = false;
    sttPhase = "idle";
    saveProgress = automated ? "自動終了の準備中…" : "録音を停止中…";
    try {
      if (!isDemoActive()) {
        try {
          await invoke("stt_stop_stream");
        } catch {}
      }
      sttListening = false;
      partialText = "";
      snapshot = await liveGetSession();
      if (snapshot.transcript_lines.length === 0) {
        const ended = await liveFinishSession();
        lastSaved = ended.saved ? ended : null;
        snapshot = await liveGetSession();
        stopFlushTimer();
        clearLiveAutoLifecycle();
        saveProgress = "";
        if (!ended.saved) {
          setMessage("success", automated ? "20分間再開されなかったため、LIVEを自動終了しました" : "LIVEを終了しました");
        }
        return;
      }
      const skipAiSummarization = shouldSkipAiSummarizationForSnapshot(snapshot);
      if (!skipAiSummarization) {
        saveProgress = "AI要約を生成中…";
        const flushed = await liveFlushSummary(true);
        snapshot = flushed;
      }
      saveProgress = "ファイルに書き出し中…";
      const saved = await liveFinishSession();
      lastSaved = saved.saved ? saved : null;
      if (saved.saved && saved.suggested_todos?.length) {
        todoDrafts = saved.suggested_todos.map((item) => ({ ...item, selected: true }));
        todoDraftSourcePath = saved.path;
      }
      snapshot = await liveGetSession();
      stopFlushTimer();
      clearLiveAutoLifecycle();
      saveProgress = "";
      if (saved.saved) {
        showSaveNotif = true;
        setTimeout(() => { showSaveNotif = false; }, 6000);
        if (automated) {
          setMessage("success", "20分間再開されなかったため、LIVEを自動保存しました");
        }
      } else {
        setMessage("success", automated ? "20分間再開されなかったため、LIVEを自動終了しました" : "LIVEを終了しました");
      }
    } catch (e: any) {
      saveProgress = "";
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  function toggleTodoDraft(index: number) {
    todoDrafts = todoDrafts.map((item, i) => i === index ? { ...item, selected: !item.selected } : item);
  }

  function closeTodoDrafts() {
    if (todoDraftSaving) return;
    todoDrafts = [];
    todoDraftSourcePath = "";
  }

  async function confirmTodoDrafts() {
    const selected = todoDrafts.filter((item) => item.selected);
    if (selected.length === 0) {
      closeTodoDrafts();
      return;
    }
    todoDraftSaving = true;
    try {
      const added = await saveLiveGeneratedTodos(selected, todoDraftSourcePath);
      setMessage("success", added.length > 0 ? `${added.length}件のTODOを追加しました` : "既存のTODOと重複していたため追加はありません");
      todoDrafts = [];
      todoDraftSourcePath = "";
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      todoDraftSaving = false;
    }
  }

  async function stopLive() {
    await stopLiveInternal(false);
  }

  async function cancelLive() {
    busy = true;
    try {
      clearSttNotice();
      pendingActivationMode = null;
      cancelSessionOnStartFailure = false;
      sttPhase = "idle";
      if (!isDemoActive()) {
        try {
          await invoke("stt_stop_stream");
        } catch {}
      }
      await liveCancelSession();
      snapshot = await liveGetSession();
      partialText = "";
      sttListening = false;
      stopFlushTimer();
      clearLiveAutoLifecycle();
      setMessage("success", "LIVEセッションを破棄しました");
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  function clearCourseData() {
    if (!selectedCourse || busy) return;
    confirmClear = true;
  }

  async function executeClearCourseData() {
    if (!selectedCourse) return;
    const name = selectedCourse.name;
    busy = true;
    clearNotice();
    try {
      await liveClearDayCache(toLiveCourse(selectedCourse));
      snapshot = { active: false, course: null, started_at: null, transcript_lines: [], pending_lines: [], summaries: [] };
      setMessage("success", `${name} のキャッシュをクリアしました`);
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  function startFlushTimer() {
    stopFlushTimer();
    const intervalMs = Math.max(30_000, liveSummaryIntervalMinutes * 60 * 1000);
    console.log("[Live] flush timer started, interval =", intervalMs, "ms");
    flushTimer = setInterval(async () => {
      console.log("[Live] flush timer tick");
      try {
        snapshot = await liveFlushSummary(true);
        console.log("[Live] flush done, summaries =", snapshot.summaries.length);
      } catch (e: any) {
        console.warn("[Live] flush error:", e);
        setMessage("error", e?.message || String(e));
      }
    }, intervalMs);
  }

  function stopFlushTimer() {
    if (flushTimer) {
      clearInterval(flushTimer);
      flushTimer = null;
    }
  }

  async function refreshLiveSttState() {
    if (isDemoActive()) {
      sttListening = false;
      sttPhase = "idle";
      return;
    }
    try {
      const [running, caller] = await Promise.all([
        invoke<boolean>("stt_is_running"),
        invoke<string | null>("stt_get_active_caller"),
      ]);
      sttListening = running && caller === "live";
      sttPhase = sttListening ? "listening" : "idle";
      if (sttListening) {
        markLiveListeningStarted();
      } else if (snapshot.active) {
        markLivePaused();
      }
    } catch {
      sttListening = false;
      sttPhase = "idle";
      if (snapshot.active) markLivePaused();
    }
  }

  onMount(async () => {
    try {
      snapshot = await liveGetSession();
      await Promise.all([refreshSchedule(false), refreshReadiness()]);
      await refreshLiveSttState();
      if (snapshot.active && sttListening) startFlushTimer();

      unlistenScheduleCache = onCacheUpdate<ScheduleResponse>("schedule_data", (fresh) => {
        applyScheduleSnapshot(fresh, new Date(), true);
      });
      scheduleFocusTimer = setInterval(refreshFocusedCoursesFromClock, 60_000);

      unlistenPartial = await listen<{ text: string; caller: string }>("stt-partial", (event) => {
        if (event.payload.caller !== "live") return;
        partialText = event.payload.text || "";
      });
      unlistenFinal = await listen<{ text: string; caller: string }>("stt-final", async (event) => {
        if (event.payload.caller !== "live") return;
        if (!snapshot.active) return;
        partialText = "";
        try {
          // The backend also emits `live-session-updated`; we apply the
          // return value and let the listener be an idempotent no-op via
          // the line-length fingerprint check below.
          snapshot = await liveAppendTranscript(event.payload.text || "");
          lastAppliedLen = snapshot.transcript_lines.length;
          markEffectiveSpeech();
        } catch (e: any) {
          setMessage("error", e?.message || String(e));
        }
      });
      unlistenState = await listen<{ state: string; caller: string }>("stt-state", (event) => {
        if (event.payload.caller !== "live") return;
        const wasListening = sttListening;
        const previousPhase = sttPhase;
        sttListening = event.payload.state === "initializing" || event.payload.state === "listening";
        if (event.payload.state === "initializing") {
          sttPhase = "initializing";
          setSttNotice("マイクと音声認識を初期化中…");
        } else if (event.payload.state === "listening") {
          sttPhase = "listening";
          clearSttNotice();
          cancelSessionOnStartFailure = false;
          if (previousPhase !== "listening") {
            const verb = pendingActivationMode === "resume" ? "LIVEを再開" : "LIVEを開始";
            setMessage("success", `${verb}: ${snapshot.course?.course_name ?? selectedCourse?.name ?? "録音"}`);
          }
          pendingActivationMode = null;
          if (!wasListening) markLiveListeningStarted();
        } else {
          sttPhase = "idle";
          clearSttNotice();
          stopFlushTimer();
          if (snapshot.active) markLivePaused();
        }
        if (sttListening && !wasListening) autoFollow = true;
      });
      unlistenError = await listen<{ message: string; caller: string }>("stt-error", (event) => {
        if (event.payload.caller !== "live") return;
        const wasStarting = sttPhase === "starting" || sttPhase === "initializing";
        sttListening = false;
        sttPhase = "idle";
        pendingActivationMode = null;
        clearSttNotice();
        stopFlushTimer();
        if (snapshot.active) markLivePaused();
        setMessage("error", event.payload.message);
        if (wasStarting && cancelSessionOnStartFailure) {
          cancelSessionOnStartFailure = false;
          void (async () => {
            try {
              await liveCancelSession();
              snapshot = await liveGetSession();
              partialText = "";
            } catch {}
          })();
        }
      });
      unlistenInfo = await listen<{ message: string; caller: string }>("stt-info", (event) => {
        if (event.payload.caller !== "live") return;
        setMessage("success", event.payload.message);
      });
      unlistenLive = await listen<LiveSessionSnapshot>("live-session-updated", (event) => {
        const len = event.payload.transcript_lines.length;
        // Skip when this update is the same one we just applied via the
        // liveAppendTranscript return value — avoids re-rendering the
        // whole transcript block twice per final.
        if (
          len === lastAppliedLen &&
          event.payload.summaries.length === snapshot.summaries.length &&
          event.payload.active === snapshot.active
        ) {
          return;
        }
        snapshot = event.payload;
        lastAppliedLen = len;
      });
      unlistenSaved = await listen<LiveSaveResult>("live-session-saved", (event) => {
        lastSaved = event.payload;
      });
      unlistenAiConfig = await listen("ai-config-changed", () => {
        refreshReadiness().catch((e: any) => {
          liveReady = false;
          setReadinessNotice(e?.message || "LIVEにはAIの準備が必要です。AI設定を確認してください。");
        });
      });
      // Timer is only needed while a session is active (to drive the
      // remaining-time badge). Started lazily by the $effect below.
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      pageLoading = false;
    }
    // Live ページ表示中は字幕浮窗をブラックリスト
    closeSubtitleOverlay().catch(() => {});
    // アプリがバックグラウンドに回ったら浮窗を表示、フォアに戻ったら再ブラック
    const win = getCurrentWindow();
    unlistenWinBlur = await win.listen("tauri://blur", () => {
      openSubtitleOverlay().catch(() => {});
    });
    unlistenWinFocus = await win.listen("tauri://focus", () => {
      refreshSchedule(true).catch(() => {});
      closeSubtitleOverlay().catch(() => {});
    });
  });

  onDestroy(() => {
    stopFlushTimer();
    stopLiveAutoGuardTimer();
    if (timeTimer) clearInterval(timeTimer);
    clearNoticeTimer();
    unlistenPartial?.();
    unlistenFinal?.();
    unlistenState?.();
    unlistenError?.();
    unlistenInfo?.();
    unlistenLive?.();
    unlistenSaved?.();
    unlistenAiConfig?.();
    unlistenScheduleCache?.();
    unlistenWinFocus?.();
    unlistenWinBlur?.();
    if (scheduleFocusTimer) clearInterval(scheduleFocusTimer);
    // Live ページを離れたら浮窗を再表示
    openSubtitleOverlay().catch(() => {});
  });
</script>

<div class="live-root view">
  <!-- ─── Floating Top Capsule ─── -->
  <header class="top-capsule">
    <div class="capsule-inner">
      <span class="live-badge" class:recording={snapshot.active && sttPhase === "listening"}>
        <span class="live-dot"></span>
        {liveBadgeLabel}
      </span>

      {#if snapshot.active && snapshot.course}
        <span class="capsule-divider"></span>
        <span class="capsule-course">{snapshot.course.course_name}</span>
        {#if remainingLabel}
          <span class="capsule-clock">{remainingLabel}</span>
        {/if}
        {#if saveProgress}
          <span class="capsule-progress">{saveProgress}</span>
        {/if}
      {:else}
        <select class="capsule-select" bind:value={selectedKey} disabled={pageLoading}>
          {#if renderedCourseOptions.length === 0}
            <option value="">授業候補なし</option>
          {:else}
            {#each renderedCourseOptions as course}
              <option value={courseKey(course)}>{courseLabel(course)}</option>
            {/each}
          {/if}
        </select>
      {/if}

      <div class="capsule-actions">
        {#if !snapshot.active}
          <button class="capsule-act primary" onclick={startLive} disabled={!canStart}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
            開始
          </button>
          <button class="capsule-act ghost note" onclick={startFreeNote} disabled={!canStartFreeNote}>
            自由ノート
          </button>
          {#if hasContent && selectedCourse}
            <div class="clear-wrap">
              <button class="capsule-act ghost danger" onclick={clearCourseData} disabled={busy}>
                <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 011-1h4a1 1 0 011 1v2"/></svg>
                クリア
              </button>
              {#if confirmClear}
                <div class="clear-tooltip" role="tooltip">
                  <span class="clear-tooltip-msg">本当に削除？</span>
                  <button class="clear-tip-btn cancel" onclick={() => confirmClear = false}>いいえ</button>
                  <button class="clear-tip-btn danger" onclick={() => { confirmClear = false; executeClearCourseData(); }}>削除</button>
                </div>
              {/if}
            </div>
          {/if}
        {:else}
          <button class="capsule-act stop" onclick={stopLive} disabled={!canStop}>
            <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="4" width="16" height="16" rx="2"/></svg>
            保存
          </button>
          {#if sttListening || sttBooting}
            <button class="capsule-act ghost" onclick={pauseLive} disabled={busy}>一時停止</button>
          {:else}
            <button class="capsule-act ghost note" onclick={resumeLive} disabled={busy}>再開</button>
          {/if}
        {/if}
      </div>
    </div>

  </header>

  <!-- ─── Main scrollable area ─── -->
  <div class="main-scroll" bind:this={scrollEl} use:bindManualScroll role="region" aria-label="LIVE transcript">
    <div class="scroll-spacer-top"></div>

    <!-- Inline messages -->
    {#if notice}
      <div class="inline-msg {notice.kind}" class:has-action={!!notice.action}>
        <span>{notice.text}</span>
        {#if notice.action === "open-ai-settings"}
          <button class="inline-msg-action" onclick={() => openSettingsWindow("ai")}>AI設定</button>
        {/if}
      </div>
    {/if}

    <!-- ─── AI Summary Card ─── -->
    {#if snapshot.summaries.length > 0}
      {@const chunk = snapshot.summaries[activeSummaryIdx]}
      {@const total = snapshot.summaries.length}
      <div class="summary-card" class:expanded={summaryExpanded}>
        <div class="summary-card-header">
          <span class="toast-ai-badge"><svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke-width="1.3"><defs><linearGradient id="ai-g1" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#c480e8"/><stop offset="100%" stop-color="#6bacf0"/></linearGradient></defs><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="url(#ai-g1)" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="url(#ai-g1)" stroke-linejoin="round" stroke-width="1"/></svg><span class="toast-badge-text">AI 要点</span></span>
          {#if total > 1}
            <div class="summary-time-pills">
              {#each snapshot.summaries as s, idx}
                <button
                  class="time-pill"
                  class:active={idx === activeSummaryIdx}
                  onclick={(e) => selectSummaryView(e, idx)}
                >{s.range_label}</button>
              {/each}
            </div>
          {:else}
            <span class="toast-meta">{chunk.range_label}</span>
          {/if}
          <button class="toast-expand-btn" onclick={summaryExpanded ? collapseSummary : expandSummary}>{summaryExpanded ? '収める' : '展開'}</button>
        </div>
        <div class="summary-card-body md">{@html renderMd(chunk.body)}</div>
        {#if summaryExpanded}
          <div class="summary-card-overlay" use:bindSummaryOverlayDismiss>
            <div class="summary-card-header">
              <span class="toast-ai-badge"><svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke-width="1.3"><defs><linearGradient id="ai-g2" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#c480e8"/><stop offset="100%" stop-color="#6bacf0"/></linearGradient></defs><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="url(#ai-g2)" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="url(#ai-g2)" stroke-linejoin="round" stroke-width="1"/></svg><span class="toast-badge-text">AI 要点</span></span>
              {#if total > 1}
                <div class="summary-time-pills">
                  {#each snapshot.summaries as s, idx}
                    <button
                      class="time-pill"
                      class:active={idx === activeSummaryIdx}
                      onclick={(e) => selectSummaryView(e, idx)}
                    >{s.range_label}</button>
                  {/each}
                </div>
              {:else}
                <span class="toast-meta">{chunk.range_label}</span>
              {/if}
              <button class="toast-expand-btn" onclick={collapseSummary}>収める</button>
            </div>
            <div class="summary-card-full md">{@html renderMd(chunk.body)}</div>
          </div>
        {/if}
      </div>
    {/if}

    <!-- ─── Transcript: Lyrics-style scrolling ─── -->
    <section class="lyrics-stage">
      {#if pageLoading}
        <div class="lyrics-empty">読み込み中…</div>
      {:else if !hasContent}
        <div class="lyrics-empty">
          {#if snapshot.active && saveProgress}
            <div class="save-capsule saving">
              <span class="save-capsule-spinner"></span>
              <span class="save-capsule-text">{saveProgress}</span>
            </div>
          {:else if snapshot.active && sttBooting}
            <div class="save-capsule saving">
              <span class="save-capsule-spinner"></span>
              <span class="save-capsule-text">{sttBootMessage}</span>
            </div>
          {:else if snapshot.active}
            <div class="waiting-vis">
              <span class="vis-bar"></span>
              <span class="vis-bar"></span>
              <span class="vis-bar"></span>
              <span class="vis-bar"></span>
              <span class="vis-bar"></span>
            </div>
            <span>音声待機中…</span>
          {:else}
            <div class="empty-hero">
              {#if saveProgress}
                <div class="save-capsule saving">
                  <span class="save-capsule-spinner"></span>
                  <span class="save-capsule-text">{saveProgress}</span>
                </div>
              {:else if showSaveNotif && lastSaved}
                <div class="save-capsule done">
                  <svg class="save-capsule-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="url(#notif-grad)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <defs><linearGradient id="notif-grad" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#c480e8"/><stop offset="100%" stop-color="#6bacf0"/></linearGradient></defs>
                    <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/>
                  </svg>
                  <span class="save-capsule-text">保存完了</span>
                </div>
                <div class="save-summary md">{@html renderMd(extractOverallSummary(lastSaved.markdown))}</div>
              {:else}
                <svg width="52" height="52" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round" opacity="0.18">
                  <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/>
                  <path d="M19 10v2a7 7 0 0 1-14 0v-2"/>
                  <line x1="12" y1="19" x2="12" y2="23"/>
                  <line x1="8" y1="23" x2="16" y2="23"/>
                </svg>
                <p>授業または自由ノートを開始すると<br/>リアルタイム文字起こしがここに表示されます</p>
              {/if}
            </div>
          {/if}
        </div>
      {:else}
        <div class="lyrics-track">
          {#if hiddenLineCount > 0}
            <div class="lyrics-hidden-hint">前{hiddenLineCount}行は保存済み（表示省略）</div>
          {/if}
          {#each visibleLines as line, i (line.at + '-' + i)}
            {@const isLast = i === visibleLines.length - 1 && !partialText.trim()}
            <div class="lyric-line" class:past={!isLast} class:active={isLast}>
              <span class="lyric-time">{line.at}</span>
              <span class="lyric-text">{line.text}</span>
            </div>
          {/each}
          {#if partialText.trim()}
            <div class="lyric-line active partial">
              <span class="lyric-time">now</span>
              <span class="lyric-text">{partialText.trim()}<span class="typing-cursor"></span></span>
            </div>
          {/if}
        </div>
        <div class="lyrics-count">{snapshot.transcript_lines.length}行</div>
      {/if}
    </section>



    <div class="scroll-spacer-bottom"></div>
  </div>

  {#if showScrollBtn && hasContent}
    <button class="scroll-to-bottom" onclick={scrollToBottom}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="7 13 12 18 17 13"/><line x1="12" y1="18" x2="12" y2="6"/></svg>
      最新へ
    </button>
  {/if}

  {#if todoDrafts.length > 0}
    <div class="todo-confirm-shell" role="presentation">
      <div class="todo-confirm-card" role="dialog" aria-modal="true" aria-labelledby="live-todo-confirm-title">
        <div class="todo-confirm-head">
          <div>
            <div id="live-todo-confirm-title" class="todo-confirm-title">LiveからTODO候補を追加</div>
            <div class="todo-confirm-sub">講義中に指示された可能性がある課題だけを選んで追加できます</div>
          </div>
          <button class="todo-confirm-close" onclick={closeTodoDrafts} disabled={todoDraftSaving} aria-label="閉じる">×</button>
        </div>
        <div class="todo-draft-list">
          {#each todoDrafts as item, idx}
            <label class="todo-draft-row" class:selected={item.selected}>
              <input type="checkbox" checked={item.selected} onchange={() => toggleTodoDraft(idx)} disabled={todoDraftSaving} />
              <span class="todo-draft-main">
                <span class="todo-draft-title">{item.title}</span>
                <span class="todo-draft-meta">
                  {item.course_name}
                  {#if item.content_type} · {item.content_type}{/if}
                  {#if item.deadline} · 締切 {item.deadline}{/if}
                </span>
                {#if item.note}
                  <span class="todo-draft-note">{item.note}</span>
                {/if}
                {#if item.source_excerpt}
                  <span class="todo-draft-source">“{item.source_excerpt}”</span>
                {/if}
              </span>
            </label>
          {/each}
        </div>
        <div class="todo-confirm-actions">
          <button class="todo-confirm-btn secondary" onclick={closeTodoDrafts} disabled={todoDraftSaving}>追加しない</button>
          <button class="todo-confirm-btn primary" onclick={confirmTodoDrafts} disabled={todoDraftSaving}>
            {todoDraftSaving ? "追加中…" : "選択したTODOを追加"}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  /* ═══════════════════════════════════════════════
     Live — Capsule + Transcript-first Design
     ═══════════════════════════════════════════════ */

  .live-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    width: 100%;
    position: relative;
    overflow: hidden;
  }

  .todo-confirm-shell {
    position: absolute;
    inset: 0;
    z-index: 80;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 18px;
    background: rgba(0, 0, 0, 0.22);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
  }

  .todo-confirm-card {
    width: min(520px, 100%);
    max-height: min(680px, calc(100% - 28px));
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border-radius: 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    box-shadow: 0 18px 60px rgba(0, 0, 0, 0.22);
  }

  .todo-confirm-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .todo-confirm-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .todo-confirm-sub {
    margin-top: 3px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-secondary);
  }

  .todo-confirm-close {
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 50%;
    background: var(--bg-secondary);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
  }

  .todo-draft-list {
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-right: 2px;
  }

  .todo-draft-row {
    display: grid;
    grid-template-columns: 18px 1fr;
    gap: 10px;
    padding: 11px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg-secondary);
    cursor: pointer;
  }

  .todo-draft-row.selected {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-secondary));
  }

  .todo-draft-row input {
    margin-top: 2px;
    accent-color: var(--accent);
  }

  .todo-draft-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .todo-draft-title {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .todo-draft-meta,
  .todo-draft-note,
  .todo-draft-source {
    font-size: 11px;
    line-height: 1.45;
    color: var(--text-secondary);
  }

  .todo-draft-source {
    color: var(--text-tertiary);
    word-break: break-word;
  }

  .todo-confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .todo-confirm-btn {
    border: none;
    border-radius: 999px;
    padding: 8px 13px;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
  }

  .todo-confirm-btn.secondary {
    background: var(--bg-secondary);
    color: var(--text-secondary);
  }

  .todo-confirm-btn.primary {
    background: var(--accent);
    color: white;
  }

  .todo-confirm-btn:disabled,
  .todo-confirm-close:disabled {
    opacity: 0.55;
    cursor: default;
  }

  /* ── Floating Top Capsule ── */
  .top-capsule {
    position: absolute;
    top: 10px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 20;
    max-width: min(760px, calc(100% - 24px));
    width: auto;
  }

  .capsule-inner {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 6px 5px 10px;
    border-radius: 20px;
    background: var(--glass-bg, rgba(255, 255, 255, 0.55));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    box-shadow: var(--shadow-glass), 0 4px 20px rgba(0, 0, 0, 0.06);
  }

  .live-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    padding: 3px 8px 3px 6px;
    border-radius: 6px;
    background: var(--bg-tertiary);
    color: var(--text-secondary);
    flex-shrink: 0;
    white-space: nowrap;
  }
  .live-badge.recording {
    background: color-mix(in srgb, var(--red) 14%, transparent);
    color: var(--red);
  }
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-tertiary);
    flex-shrink: 0;
  }
  .live-badge.recording .live-dot {
    background: var(--red);
    animation: pulse-dot 1.2s ease-in-out infinite;
  }
  @keyframes pulse-dot {
    0%, 100% { opacity: 1; box-shadow: 0 0 0 0 rgba(255, 59, 48, 0.5); }
    50% { opacity: 0.7; box-shadow: 0 0 0 4px rgba(255, 59, 48, 0); }
  }

  .capsule-divider {
    width: 1px;
    height: 16px;
    background: var(--border);
    flex-shrink: 0;
  }

  .capsule-course {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    letter-spacing: -0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 200px;
    min-width: 0;
  }

  .capsule-clock {
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
    letter-spacing: -0.01em;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .capsule-progress {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border: 0.5px solid color-mix(in srgb, var(--accent) 18%, transparent);
    border-radius: 999px;
    padding: 4px 10px;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .capsule-select {
    padding: 4px 8px;
    font-size: 12.5px;
    font-family: inherit;
    font-weight: 500;
    color: var(--text-primary);
    background: transparent;
    border: 0.5px solid color-mix(in srgb, var(--text-primary) 10%, transparent);
    border-radius: 10px;
    outline: none;
    cursor: pointer;
    max-width: 240px;
    min-width: 0;
    transition: border-color 0.15s;
  }
  .capsule-select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 18%, transparent);
  }

  .capsule-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: 2px;
    flex-shrink: 0;
  }

  .capsule-act {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 5px 12px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: 600;
    font-family: inherit;
    border: none;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s, transform 0.1s, opacity 0.15s;
  }
  .capsule-act:active { transform: scale(0.96); }
  .capsule-act:disabled { opacity: 0.4; cursor: default; transform: none; }
  .capsule-act.primary {
    background: var(--blue);
    color: var(--text-on-accent);
  }
  .capsule-act.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--blue) 85%, #000);
  }
  .capsule-act.stop {
    background: color-mix(in srgb, var(--red) 14%, transparent);
    color: var(--red);
  }
  .capsule-act.stop:hover:not(:disabled) {
    background: color-mix(in srgb, var(--red) 22%, transparent);
  }
  .capsule-act.ghost {
    background: transparent;
    color: var(--text-secondary);
    padding: 5px 8px;
  }
  .capsule-act.ghost:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
  }
  .capsule-act.ghost.danger {
    color: color-mix(in srgb, var(--red, #e5484d) 72%, var(--text-secondary));
  }
  .capsule-act.ghost.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--red, #e5484d) 10%, transparent);
    color: var(--red, #e5484d);
  }
  .capsule-act.ghost.note {
    color: color-mix(in srgb, var(--blue) 72%, var(--text-secondary));
  }
  .capsule-act.ghost.note:hover:not(:disabled) {
    background: color-mix(in srgb, var(--blue) 10%, transparent);
    color: var(--blue);
  }

  /* ── Main Scroll Area ── */
  .main-scroll {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: 0 16px;
    scroll-behavior: smooth;
    scrollbar-width: none;
  }
  .main-scroll::-webkit-scrollbar { display: none; }

  .scroll-spacer-top { height: 56px; flex-shrink: 0; }
  .scroll-spacer-bottom { height: 32px; flex-shrink: 0; }

  /* ── Inline Messages ── */
  .inline-msg {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 9px 14px;
    border-radius: 10px;
    font-size: 12.5px;
    font-weight: 500;
    margin-bottom: 10px;
    animation: toast-enter 0.25s ease-out;
  }
  .inline-msg.has-action {
    flex-wrap: wrap;
  }
  .inline-msg.error {
    background: color-mix(in srgb, var(--red) 10%, transparent);
    color: var(--red);
    border: 0.5px solid color-mix(in srgb, var(--red) 15%, transparent);
  }
  .inline-msg.warning {
    background: color-mix(in srgb, var(--orange, #e67700) 8%, var(--bg-card));
    color: var(--orange, #e67700);
    border: 0.5px solid color-mix(in srgb, var(--orange, #e67700) 18%, transparent);
  }
  .inline-msg.success {
    background: color-mix(in srgb, var(--green) 10%, transparent);
    color: var(--green);
    border: 0.5px solid color-mix(in srgb, var(--green) 15%, transparent);
  }
  .inline-msg-action {
    border: none;
    background: color-mix(in srgb, currentColor 12%, transparent);
    color: inherit;
    border-radius: 999px;
    padding: 4px 10px;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.18s ease, transform 0.18s ease;
  }
  .inline-msg-action:hover {
    background: color-mix(in srgb, currentColor 18%, transparent);
    transform: translateY(-1px);
  }
  .inline-msg-action:focus-visible {
    outline: 2px solid color-mix(in srgb, currentColor 35%, transparent);
    outline-offset: 2px;
  }

  /* ── Summary Card (single floating card) ── */
  .summary-card {
    position: sticky;
    top: 56px;
    z-index: 10;
    margin-bottom: 14px;
    background: #f9f6fc;
    border: 0.5px solid rgba(175, 82, 222, 0.22);
    border-radius: 14px;
    padding: 10px 16px;
    box-shadow: 0 2px 16px rgba(175, 82, 222, 0.08), 0 1px 3px rgba(0, 0, 0, 0.04);
    animation: card-enter 0.4s cubic-bezier(0.22, 1, 0.36, 1) both;
    overflow: hidden;
    transition: box-shadow 0.3s ease;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .summary-card {
      background: #1c1c20;
      border-color: rgba(191, 90, 242, 0.24);
      box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28), 0 0 0 1px rgba(255, 255, 255, 0.04);
    }
    :global(:root:not([data-theme="light"])) .summary-card.expanded {
      box-shadow: 0 14px 36px rgba(0, 0, 0, 0.34), 0 0 0 1px rgba(255, 255, 255, 0.05);
    }
  }
  :global([data-theme="dark"]) .summary-card {
    background: #1c1c20;
    border-color: rgba(191, 90, 242, 0.24);
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28), 0 0 0 1px rgba(255, 255, 255, 0.04);
  }
  :global([data-theme="dark"]) .summary-card.expanded {
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.34), 0 0 0 1px rgba(255, 255, 255, 0.05);
  }
  .summary-card.expanded {
    overflow: visible;
    box-shadow: 0 4px 24px rgba(175, 82, 222, 0.12), 0 1px 3px rgba(0, 0, 0, 0.04);
  }

  .summary-card-header {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 8px;
    margin-bottom: 4px;
    min-width: 0;
  }
  .toast-ai-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .toast-badge-text {
    font-size: 12px;
    font-weight: 700;
    background: linear-gradient(135deg, #c480e8, #6bacf0);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    letter-spacing: 0.3px;
    line-height: 1;
  }

  /* Time-block pill navigation */
  .summary-time-pills {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    width: 100%;
    flex-wrap: nowrap;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }
  .summary-time-pills::-webkit-scrollbar { display: none; }
  .time-pill {
    all: unset;
    cursor: pointer;
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    padding: 2px 8px;
    border-radius: 20px;
    border: 0.5px solid color-mix(in srgb, var(--text-tertiary) 20%, transparent);
    transition: all 0.2s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .time-pill:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--text-secondary);
    transform: scale(1.04);
  }
  .time-pill.active {
    background: linear-gradient(135deg, rgba(196, 128, 232, 0.12), rgba(107, 172, 240, 0.12));
    border-color: rgba(175, 82, 222, 0.3);
    color: var(--text-primary);
    font-weight: 600;
    transform: scale(1.06);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .time-pill {
      color: rgba(245, 245, 247, 0.76);
      border-color: rgba(255, 255, 255, 0.1);
      background: rgba(255, 255, 255, 0.03);
    }
    :global(:root:not([data-theme="light"])) .time-pill:hover {
      background: rgba(74, 144, 217, 0.16);
      color: rgba(245, 245, 247, 0.92);
    }
    :global(:root:not([data-theme="light"])) .time-pill.active {
      background: linear-gradient(135deg, rgba(191, 90, 242, 0.2), rgba(74, 144, 217, 0.2));
      border-color: rgba(191, 90, 242, 0.34);
    }
  }
  :global([data-theme="dark"]) .time-pill {
    color: rgba(245, 245, 247, 0.76);
    border-color: rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.03);
  }
  :global([data-theme="dark"]) .time-pill:hover {
    background: rgba(74, 144, 217, 0.16);
    color: rgba(245, 245, 247, 0.92);
  }
  :global([data-theme="dark"]) .time-pill.active {
    background: linear-gradient(135deg, rgba(191, 90, 242, 0.2), rgba(74, 144, 217, 0.2));
    border-color: rgba(191, 90, 242, 0.34);
  }

  .toast-meta {
    font-size: 11px;
    color: var(--text-tertiary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toast-expand-btn {
    all: unset;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    font-size: 10px;
    color: var(--accent);
    font-weight: 500;
    opacity: 0.8;
    padding: 0px 6px;
    min-height: 20px;
    border-radius: 4px;
    white-space: nowrap;
    justify-self: end;
    position: relative;
    z-index: 1;
    transition: background 0.12s;
  }
  .toast-expand-btn:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    opacity: 1;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .toast-expand-btn:hover {
      background: rgba(74, 144, 217, 0.18);
    }
  }
  :global([data-theme="dark"]) .toast-expand-btn:hover {
    background: rgba(74, 144, 217, 0.18);
  }

  /* Collapsed body: show bullet titles only (before ---) */
  .summary-card-body {
    margin: 0;
    font-size: 13.5px;
    font-weight: 400;
    line-height: 1.65;
    color: var(--text-primary);
    overflow: hidden;
  }
  /* Hide everything after the <hr> in collapsed mode */
  .summary-card-body :global(hr),
  .summary-card-body :global(hr ~ *) {
    display: none;
  }

  /* Expanded overlay */
  .summary-card-overlay {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    padding: 10px 16px;
    background: #f9f6fc;
    border: 0.5px solid rgba(175, 82, 222, 0.22);
    border-radius: 14px;
    box-shadow: 0 8px 32px rgba(175, 82, 222, 0.12), var(--shadow-md);
    z-index: 30;
    cursor: pointer;
    animation: overlay-expand 0.3s cubic-bezier(0.22, 1, 0.36, 1) both;
    transform-origin: top center;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .summary-card-overlay {
      background: #1c1c20;
      border-color: rgba(191, 90, 242, 0.28);
      box-shadow: 0 18px 40px rgba(0, 0, 0, 0.38), 0 0 0 1px rgba(255, 255, 255, 0.05);
    }
  }
  :global([data-theme="dark"]) .summary-card-overlay {
    background: #1c1c20;
    border-color: rgba(191, 90, 242, 0.28);
    box-shadow: 0 18px 40px rgba(0, 0, 0, 0.38), 0 0 0 1px rgba(255, 255, 255, 0.05);
  }
  .summary-card-full {
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text-primary);
  }
  /* In expanded view, list-style none for explanation section below hr */
  .summary-card-full :global(hr ~ ul),
  .summary-card-full :global(hr ~ ol) {
    list-style: none;
    padding-left: 0;
  }

  /* Markdown in card body and overlay */
  .summary-card-body.md :global(hr),
  .summary-card-full.md :global(hr) {
    margin: 8px 0;
    border: none;
    border-top: 0.5px solid var(--glass-border);
  }
  .summary-card-body.md :global(p),
  .summary-card-full.md :global(p) { margin: 0 0 4px; }
  .summary-card-body.md :global(p:last-child),
  .summary-card-full.md :global(p:last-child) { margin-bottom: 0; }
  .summary-card-body.md :global(ul), .summary-card-body.md :global(ol),
  .summary-card-full.md :global(ul), .summary-card-full.md :global(ol) { margin: 0 0 4px; padding-left: 16px; }
  .summary-card-body.md :global(li),
  .summary-card-full.md :global(li) { margin-bottom: 2px; }
  .summary-card-body.md :global(h1), .summary-card-body.md :global(h2), .summary-card-body.md :global(h3),
  .summary-card-body.md :global(h4), .summary-card-body.md :global(h5),
  .summary-card-full.md :global(h1), .summary-card-full.md :global(h2), .summary-card-full.md :global(h3),
  .summary-card-full.md :global(h4), .summary-card-full.md :global(h5) {
    font-size: 13px;
    font-weight: 600;
    margin: 6px 0 3px;
    color: var(--text-primary);
  }
  .summary-card-body.md :global(h1:first-child), .summary-card-body.md :global(h2:first-child),
  .summary-card-body.md :global(h3:first-child),
  .summary-card-full.md :global(h1:first-child), .summary-card-full.md :global(h2:first-child),
  .summary-card-full.md :global(h3:first-child) { margin-top: 0; }
  .summary-card-body.md :global(code),
  .summary-card-full.md :global(code) {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    padding: 1px 4px;
    border-radius: 4px;
    font-size: 0.88em;
  }
  .summary-card-body.md :global(pre),
  .summary-card-full.md :global(pre) {
    background: color-mix(in srgb, var(--text-primary) 4%, transparent);
    padding: 8px 10px;
    border-radius: 8px;
    overflow-x: auto;
    font-size: 12px;
    line-height: 1.5;
  }
  .summary-card-body.md :global(pre code),
  .summary-card-full.md :global(pre code) { background: transparent; padding: 0; }
  .summary-card-body.md :global(blockquote),
  .summary-card-full.md :global(blockquote) {
    margin: 4px 0;
    padding-left: 10px;
    border-left: 2px solid var(--border);
    color: var(--text-secondary);
  }
  .summary-card-body.md :global(strong),
  .summary-card-full.md :global(strong) { font-weight: 600; }
  .summary-card-body.md :global(a),
  .summary-card-full.md :global(a) { color: var(--accent); text-decoration: none; }
  .summary-card-body.md :global(a:hover),
  .summary-card-full.md :global(a:hover) { text-decoration: underline; }

  @keyframes toast-enter {
    from { opacity: 0; transform: translateY(-6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @keyframes card-enter {
    from {
      opacity: 0;
      transform: translateY(-10px) scale(0.96);
      filter: blur(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
      filter: blur(0);
    }
  }

  @keyframes overlay-expand {
    from {
      opacity: 0;
      transform: scaleY(0.92) translateY(-4px);
    }
    to {
      opacity: 1;
      transform: scaleY(1) translateY(0);
    }
  }

  /* ── Lyrics Stage ── */
  .lyrics-stage {
    min-height: 50vh;
    position: relative;
    display: flex;
    flex-direction: column;
  }

  .lyrics-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    color: var(--text-tertiary);
    font-size: 13px;
    min-height: 50vh;
  }

  .empty-hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    text-align: center;
  }
  .empty-hero p {
    margin: 0;
    font-size: 13px;
    color: var(--text-tertiary);
    line-height: 1.7;
  }

  /* ── Waiting Visualizer (audio bars) ── */
  .waiting-vis {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 28px;
  }
  .vis-bar {
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
    opacity: 0.5;
    animation: vis-wave 1.2s ease-in-out infinite;
  }
  .vis-bar:nth-child(1) { height: 8px; animation-delay: 0s; }
  .vis-bar:nth-child(2) { height: 16px; animation-delay: 0.15s; }
  .vis-bar:nth-child(3) { height: 22px; animation-delay: 0.3s; }
  .vis-bar:nth-child(4) { height: 14px; animation-delay: 0.45s; }
  .vis-bar:nth-child(5) { height: 10px; animation-delay: 0.6s; }
  @keyframes vis-wave {
    0%, 100% { transform: scaleY(0.4); opacity: 0.35; }
    50% { transform: scaleY(1); opacity: 0.7; }
  }

  /* ── Lyrics Track ── */
  .lyrics-track {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 20px 8px 5vh;
    user-select: text;
    -webkit-user-select: text;
  }

  .lyric-line {
    display: flex;
    align-items: baseline;
    gap: 14px;
    padding: 8px 12px;
    border-radius: 10px;
    transition:
      opacity 0.5s cubic-bezier(0.22, 1, 0.36, 1),
      transform 0.5s cubic-bezier(0.22, 1, 0.36, 1),
      filter 0.5s cubic-bezier(0.22, 1, 0.36, 1),
      background 0.3s ease;
    animation: lyric-enter 0.45s cubic-bezier(0.22, 1, 0.36, 1) both;
  }

  @keyframes lyric-enter {
    from {
      opacity: 0;
      transform: translateY(14px) scale(0.97);
      filter: blur(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
      filter: blur(0);
    }
  }

  /* Past lines: faded, smaller */
  .lyric-line.past {
    opacity: 0.38;
    transform: scale(0.97);
  }
  .lyric-line.past:hover {
    opacity: 0.65;
    background: color-mix(in srgb, var(--text-primary) 3%, transparent);
  }

  /* Active line: full brightness, larger, emphasized */
  .lyric-line.active {
    opacity: 1;
    transform: scale(1);
  }
  .lyric-line.active .lyric-text {
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--text-primary);
  }
  .lyric-line.active .lyric-time {
    color: var(--accent);
    opacity: 1;
  }

  /* Partial (live recognition) glow */
  .lyric-line.partial {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
    border: 0.5px solid color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .lyric-line.partial .lyric-text {
    background: linear-gradient(90deg, var(--text-primary) 60%, var(--accent));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .lyric-time {
    font-size: 11px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: var(--text-tertiary);
    opacity: 0.7;
    flex-shrink: 0;
    min-width: 38px;
    text-align: right;
    letter-spacing: -0.01em;
    transition: color 0.3s, opacity 0.3s;
  }

  .lyric-text {
    font-size: 15px;
    font-weight: 400;
    line-height: 1.6;
    color: var(--text-primary);
    word-break: break-word;
    transition:
      font-size 0.4s cubic-bezier(0.22, 1, 0.36, 1),
      font-weight 0.4s cubic-bezier(0.22, 1, 0.36, 1),
      color 0.3s ease;
  }

  /* Typing cursor */
  .typing-cursor {
    display: inline-block;
    width: 2px;
    height: 1em;
    background: var(--accent);
    margin-left: 2px;
    vertical-align: text-bottom;
    border-radius: 1px;
    animation: cursor-blink 1s steps(2, start) infinite;
  }
  @keyframes cursor-blink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0; }
  }

  /* Line count badge */
  .lyrics-hidden-hint {
    align-self: center;
    font-size: 10px;
    color: var(--text-tertiary);
    opacity: 0.6;
    padding: 4px 12px;
    margin-bottom: 4px;
    letter-spacing: 0.02em;
  }

  .lyrics-count {
    position: sticky;
    bottom: 8px;
    align-self: flex-end;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--glass-bg, rgba(255, 255, 255, 0.6));
    backdrop-filter: blur(12px) var(--glass-saturate);
    -webkit-backdrop-filter: blur(12px) var(--glass-saturate);
    padding: 3px 10px;
    border-radius: 8px;
    border: 0.5px solid var(--glass-border);
    pointer-events: none;
    margin-right: 8px;
  }

  /* ── Save Capsule Notification ── */
  .save-capsule {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 18px;
    border-radius: 100px;
    animation: capsule-in 0.35s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .save-capsule.saving {
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-card));
    border: 0.5px solid color-mix(in srgb, var(--accent) 18%, var(--glass-border));
  }
  .save-capsule.done {
    background: linear-gradient(135deg, rgba(196, 128, 232, 0.08), rgba(107, 172, 240, 0.08));
    border: 0.5px solid rgba(175, 82, 222, 0.22);
  }
  .save-capsule-icon {
    flex-shrink: 0;
  }
  .save-capsule-text {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }
  .save-capsule.done .save-capsule-text {
    background: linear-gradient(135deg, #c480e8, #6bacf0);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }
  .save-capsule-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid color-mix(in srgb, var(--accent) 25%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  .save-summary {
    margin-top: 12px;
    font-size: 13px;
    line-height: 1.7;
    color: var(--text-secondary);
    text-align: center;
    max-width: 320px;
    animation: capsule-in 0.4s cubic-bezier(0.22, 1, 0.36, 1) 0.1s both;
  }

  @keyframes capsule-in {
    from {
      opacity: 0;
      transform: scale(0.9) translateY(6px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ── Responsive ── */
  @media (max-width: 600px) {
    .capsule-course { max-width: 120px; }
    .capsule-select { max-width: 160px; }
    .capsule-clock { font-size: 12px; }
    .capsule-inner { gap: 4px; padding: 4px 4px 4px 8px; }
  }

  p { margin: 0; }

  /* ── Scroll to Bottom Button ── */
  .scroll-to-bottom {
    position: absolute;
    bottom: 16px;
    left: 0;
    right: 0;
    margin: 0 auto;
    width: fit-content;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 14px 6px 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--glass-bg, rgba(255, 255, 255, 0.7));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    border-radius: 100px;
    box-shadow: var(--shadow-glass), 0 2px 8px rgba(0, 0, 0, 0.06);
    cursor: pointer;
    z-index: 15;
    animation: capsule-in 0.25s cubic-bezier(0.22, 1, 0.36, 1) both;
    transition: transform 0.15s ease, box-shadow 0.15s ease;
  }
  .scroll-to-bottom:hover {
    transform: scale(1.04);
    box-shadow: var(--shadow-glass), 0 4px 14px rgba(0, 0, 0, 0.1);
  }
  .scroll-to-bottom:active {
    transform: scale(0.97);
  }

  /* ── Clear Confirm Dialog ── */
  .confirm-overlay {
    position: absolute;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.35);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    animation: capsule-in 0.18s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .confirm-card {
    background: var(--glass-bg, rgba(255,255,255,0.85));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    border-radius: 16px;
    box-shadow: var(--shadow-glass), 0 8px 32px rgba(0,0,0,0.18);
    padding: 20px 22px 16px;
    width: min(320px, calc(100% - 40px));
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .confirm-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }
  .confirm-desc {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0;
    line-height: 1.5;
  }
  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .confirm-btn {
    padding: 6px 16px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    border: 0.5px solid var(--glass-border);
    transition: opacity 0.15s;
  }
  .confirm-btn:hover { opacity: 0.8; }
  .confirm-btn.cancel {
    background: var(--glass-bg, rgba(255,255,255,0.6));
    color: var(--text-primary);
  }
  .confirm-btn.danger {
    background: rgba(255, 59, 48, 0.12);
    color: #ff3b30;
    border-color: rgba(255, 59, 48, 0.25);
  }
  .confirm-btn.danger:hover { background: rgba(255, 59, 48, 0.2); opacity: 1; }

  /* ── Clear inline tooltip ── */
  .clear-wrap {
    position: relative;
  }
  .clear-tooltip {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 8px;
    background: var(--glass-bg, rgba(255,255,255,0.92));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    border-radius: 10px;
    box-shadow: var(--shadow-glass), 0 4px 16px rgba(0,0,0,0.14);
    white-space: nowrap;
    animation: capsule-in 0.15s cubic-bezier(0.22, 1, 0.36, 1) both;
    z-index: 50;
  }
  .clear-tooltip::after {
    content: '';
    position: absolute;
    bottom: 100%;
    right: 14px;
    border: 5px solid transparent;
    border-bottom-color: var(--glass-border, rgba(0,0,0,0.12));
  }
  .clear-tooltip-msg {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    padding-right: 2px;
  }
  .clear-tip-btn {
    padding: 3px 10px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    border: 0.5px solid var(--glass-border);
    transition: opacity 0.12s;
  }
  .clear-tip-btn:hover { opacity: 0.75; }
  .clear-tip-btn.cancel {
    background: var(--glass-bg, rgba(255,255,255,0.5));
    color: var(--text-primary);
  }
  .clear-tip-btn.danger {
    background: rgba(255, 59, 48, 0.12);
    color: #ff3b30;
    border-color: rgba(255, 59, 48, 0.3);
  }
  .clear-tip-btn.danger:hover { background: rgba(255, 59, 48, 0.22); opacity: 1; }
</style>
