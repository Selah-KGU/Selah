<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onCacheUpdate } from "../stores";
  import LiveNotice from "./live/LiveNotice.svelte";
  import LiveRightRail from "./live/LiveRightRail.svelte";
  import LiveScrollToBottomButton from "./live/LiveScrollToBottomButton.svelte";
  import LiveSummaryCard from "./live/LiveSummaryCard.svelte";
  import LiveTopCapsule from "./live/LiveTopCapsule.svelte";
  import LiveTranscriptStage from "./live/LiveTranscriptStage.svelte";
  import LiveWhiteboardPage from "./live/LiveWhiteboardPage.svelte";
  import {
    chooseFocusedCourseOptions,
    courseKey,
    courseLabel,
    createFreeNoteCourse,
    defaultSelectedCourseKey,
    toLiveCourse,
  } from "./live/liveCourseSelection";
  import { extractOverallSummary, renderMd } from "./live/liveMarkdown";
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
  } from "../api";
  import type { ScheduleResponse } from "../types";
  import { PERIOD_TIMES } from "../types";
  import { buildCourseSlots, type CourseSlot } from "../schedule";
  import { computeWhiteboardLayout } from "../whiteboardLayout";
  import type { LiveTodoDraft, NoticeAction, NoticeKind, NoticeSource, NoticeState, SttPhase, WhiteboardStagePreset } from "./live/liveTypes";

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
  let aiReplyLanguage = $state("ja");
  let timeTimer: ReturnType<typeof setInterval> | null = null;
  let now = $state(new Date());
  let scrollEl: HTMLElement | null = null;
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

  function debugLog(...args: unknown[]) {
    try {
      if (localStorage.getItem("selah-debug-logs") === "1") console.log(...args);
    } catch { /* ignore */ }
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
  const activeSummaryTerms = $derived.by(() => {
    const chunk = snapshot.summaries[activeSummaryIdx];
    return (chunk?.terms ?? []).filter((term) => term.term?.trim() && term.explanation?.trim());
  });

  // Stacked-card pager state for term annotations.
  // No wheel interception — switching is via click on a back card or the prev/next chips.
  let termCardIdx = $state(0);
  let termsCollapsed = $state(false);
  // activeSummaryTerms is a $derived built with .filter(), so it returns a NEW
  // array reference every time the snapshot updates (every few hundred ms during
  // a live session). Watching the array itself would reset termCardIdx on every
  // transcript tick. Instead, derive a stable primitive fingerprint and only reset
  // when the term set actually changes.
  const termFingerprint = $derived(
    activeSummaryTerms.map((t) => t.term).join("|")
  );
  const collapsedTermPreview = $derived(activeSummaryTerms.slice(0, 3));
  $effect(() => {
    termFingerprint;
    // Only clamp if our current pick is now out of range (e.g. user switched
    // segments to one with fewer terms). Don't otherwise touch termCardIdx —
    // appending new terms shouldn't yank the user back to the first card.
    // Use untrack so writing termCardIdx does not cause this effect to re-run.
    if (untrack(() => termCardIdx) >= activeSummaryTerms.length) {
      termCardIdx = 0;
    }
  });
  function selectTermCard(i: number) {
    termCardIdx = Math.max(0, Math.min(activeSummaryTerms.length - 1, i));
  }
  function termStackOffset(i: number): number {
    const total = activeSummaryTerms.length;
    return total <= 0 ? 0 : (i - termCardIdx + total) % total;
  }
  function termCardPrev() {
    const total = activeSummaryTerms.length;
    if (total > 0) termCardIdx = (termCardIdx - 1 + total) % total;
  }
  function termCardNext() {
    const total = activeSummaryTerms.length;
    if (total > 0) termCardIdx = (termCardIdx + 1) % total;
  }
  function toggleTermsCollapsed() {
    termsCollapsed = !termsCollapsed;
  }

  let whiteboardExpanded = $state(false);
  let whiteboardZoom = $state(0.78);
  let whiteboardPanX = $state(0);
  let whiteboardPanY = $state(0);
  let whiteboardDragStart = $state<{ x: number; y: number; panX: number; panY: number } | null>(null);
  let whiteboardWasDragged = $state(false);
  let selectedBoardNodeId = $state<string | null>(null);
  // Canvas dimensions are bound from the DOM; stage size adapts so the board
  // fills the available area instead of being centered in a fixed-pixel box.
  let boardCanvasWidth = $state(0);
  let boardCanvasHeight = $state(0);
  let initialFitDone = $state(false);
  $effect(() => {
    // If the active segment has no whiteboard (e.g. user clicked a time-pill
    // for a segment without one, or AI removed the board), drop expanded
    // state so reopening starts from a clean slate. We deliberately do NOT
    // close on segment-change when the new segment also has a board —
    // swapping content in-place is less jarring than forcing a back/forth.
    const hasBoard = !!activeWhiteboardLayout;
    if (untrack(() => whiteboardExpanded) && !hasBoard) {
      whiteboardExpanded = false;
    }
  });
  function openWhiteboardOverlay() {
    // Reset pan/zoom to preset defaults; the auto-fit effect will recalculate
    // once the canvas dimensions are measured after the DOM renders.
    const preset = getWhiteboardStagePreset(activeWhiteboardLayout?.nodes.length ?? 0);
    whiteboardZoom = preset.zoom;
    whiteboardPanX = 0;
    whiteboardPanY = 0;
    initialFitDone = false;
    whiteboardExpanded = true;
  }
  function closeWhiteboardOverlay() {
    whiteboardExpanded = false;
  }
  function clampWhiteboardZoom(value: number): number {
    return Math.max(0.05, Math.round(value * 100) / 100);
  }
  function setWhiteboardZoom(value: number) {
    whiteboardZoom = clampWhiteboardZoom(value);
  }
  function resetWhiteboardView() {
    const preset = getWhiteboardStagePreset(activeWhiteboardLayout?.nodes.length ?? 0);
    if (boardCanvasWidth > 0 && boardCanvasHeight > 0) {
      // Fit the full stage inside the measured canvas, leaving a small margin.
      const fitZoom = Math.min(boardCanvasWidth / preset.width, boardCanvasHeight / preset.height) * 0.94;
      whiteboardZoom = clampWhiteboardZoom(fitZoom);
    } else {
      whiteboardZoom = preset.zoom;
    }
    whiteboardPanX = 0;
    whiteboardPanY = 0;
  }
  function handleWhiteboardWheel(event: WheelEvent) {
    event.preventDefault();
    const delta = event.deltaY > 0 ? -0.08 : 0.08;
    setWhiteboardZoom(whiteboardZoom + delta);
  }
  function handleWhiteboardPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (target.closest(".board-zoom-controls")) return;
    // Clicks on nodes shouldn't start a pan — let the node's own onclick run.
    if (target.closest(".visual-board-node")) return;
    whiteboardWasDragged = false;
    whiteboardDragStart = { x: event.clientX, y: event.clientY, panX: whiteboardPanX, panY: whiteboardPanY };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function handleWhiteboardPointerMove(event: PointerEvent) {
    if (!whiteboardDragStart) return;
    const dx = event.clientX - whiteboardDragStart.x;
    const dy = event.clientY - whiteboardDragStart.y;
    if (!whiteboardWasDragged && (Math.abs(dx) > 4 || Math.abs(dy) > 4)) whiteboardWasDragged = true;
    whiteboardPanX = whiteboardDragStart.panX + dx;
    whiteboardPanY = whiteboardDragStart.panY + dy;
  }
  function handleWhiteboardPointerUp(event: PointerEvent) {
    whiteboardDragStart = null;
    try {
      (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already be released if the OS cancelled the drag.
    }
  }
  function bindWhiteboardOverlayDismiss(node: HTMLElement) {
    // Page-style overlay: no click-outside (the page fills the view).
    // Escape returns to the Live transcript — matches OS back-gesture intent.
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeWhiteboardOverlay();
    };
    window.addEventListener("keydown", onKey);
    return {
      destroy() {
        window.removeEventListener("keydown", onKey);
      }
    };
  }
  const termFloatLabels = $derived.by(() => {
    switch ((aiReplyLanguage || "ja").toLowerCase()) {
      case "zh":
      case "zh-cn":
      case "cn":
        return { title: "用语注释", boardTitle: "知识整理", empty: "本段没有需要解释的术语", source: "课堂依据", externalSource: "外部来源", externalNode: "外部", collapse: "折叠", expand: "展开", previous: "上一个术语", next: "下一个术语" };
      case "en":
        return { title: "Key Terms", boardTitle: "Knowledge Board", empty: "No terms for this segment", source: "Class source", externalSource: "External source", externalNode: "External", collapse: "Collapse", expand: "Expand", previous: "Previous term", next: "Next term" };
      case "ko":
        return { title: "핵심 용어", boardTitle: "지식 정리", empty: "이 구간의 용어 설명이 없습니다", source: "수업 근거", externalSource: "외부 출처", externalNode: "외부", collapse: "접기", expand: "펼치기", previous: "이전 용어", next: "다음 용어" };
      default:
        return { title: "用語注釈", boardTitle: "知識整理", empty: "この区間の注釈はありません", source: "講義内根拠", externalSource: "外部出典", externalNode: "外部", collapse: "折りたたむ", expand: "展開", previous: "前の用語", next: "次の用語" };
    }
  });

  const activeWhiteboardLayout = $derived.by(() =>
    computeWhiteboardLayout(snapshot.summaries[activeSummaryIdx]?.whiteboard ?? null, {
      fallbackBoardTitle: termFloatLabels.boardTitle,
      externalNodeLabel: termFloatLabels.externalNode,
    })
  );
  const activeWhiteboardStage = $derived(getWhiteboardStagePreset(activeWhiteboardLayout?.nodes.length ?? 0));

  const boardHighlight = $derived.by(() => {
    if (!selectedBoardNodeId || !activeWhiteboardLayout) return null;
    const nodes = new Set<string>([selectedBoardNodeId]);
    const edges = new Set<string>();
    for (const e of activeWhiteboardLayout.edges) {
      if (e.from === selectedBoardNodeId) {
        nodes.add(e.to);
        edges.add(e.id);
      } else if (e.to === selectedBoardNodeId) {
        nodes.add(e.from);
        edges.add(e.id);
      }
    }
    return { nodes, edges };
  });

  function toggleBoardNodeSelection(id: string, event: MouseEvent | KeyboardEvent) {
    event.stopPropagation();
    selectedBoardNodeId = selectedBoardNodeId === id ? null : id;
  }

  function clearBoardSelection() {
    // Suppress the click that fires at the end of a pan drag — only treat
    // genuine taps on empty canvas as "deselect".
    if (whiteboardWasDragged) return;
    selectedBoardNodeId = null;
  }

  // Drop selection when the segment changes or the overlay closes. We track
  // primitives (segment index, overlay flag) — NOT activeWhiteboardLayout,
  // since live transcript updates re-derive that on every chunk and would
  // otherwise reset the selection the instant the user clicks.
  $effect(() => {
    void activeSummaryIdx;
    void whiteboardExpanded;
    untrack(() => { selectedBoardNodeId = null; });
  });

  // Auto-fit: once the board-page canvas has been measured, recalculate the
  // initial zoom so the stage fills the real available area. We only do this
  // once per open (initialFitDone guard) to avoid fighting with user pans/zooms.
  $effect(() => {
    if (!whiteboardExpanded) {
      untrack(() => { initialFitDone = false; });
      return;
    }
    const w = boardCanvasWidth;
    const h = boardCanvasHeight;
    if (w <= 0 || h <= 0) return;
    if (untrack(() => initialFitDone)) return;
    untrack(() => {
      resetWhiteboardView();
      initialFitDone = true;
    });
  });

  function getWhiteboardStagePreset(nodeCount: number): WhiteboardStagePreset {
    if (nodeCount > 14) {
      const extra = nodeCount - 14;
      const spread = Math.sqrt(extra);
      return {
        width: Math.round(1360 + spread * 220 + extra * 16),
        height: Math.round(820 + spread * 120 + extra * 12),
        zoom: 0.86,
      };
    }
    if (nodeCount > 8) return { width: 1220, height: 760, zoom: 0.9 };
    return { width: 1040, height: 660, zoom: 0.96 };
  }

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
  const todoDraftsWithDeadlineCount = $derived(todoDrafts.filter((item) => item.deadline.trim()).length);
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

  let lastScrolledLen = -1; // plain variable — not reactive; writing inside $effect must not re-trigger it
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
    // Use untrack for snapshot/showSaveNotif/todoDrafts reads: writing snapshot
    // inside the async .then() would otherwise re-trigger this effect → infinite loop.
    if (!course || untrack(() => snapshot.active || showSaveNotif || todoDrafts.length > 0)) return;
    livePeekDayCache(toLiveCourse(course)).then((cached) => {
      if (untrack(() => snapshot.active || showSaveNotif || todoDrafts.length > 0)) return;
      if (cached.transcript_lines.length > 0 || cached.summaries.length > 0) {
        snapshot = cached;
      } else if (untrack(() => snapshot.course)) {
        snapshot = { active: false, course: null, started_at: null, transcript_lines: [], pending_lines: [], summaries: [] };
      }
    }).catch(() => {});
  });

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
    debugLog("[LIVE] allCourseOptions =", allCourseOptions.map((c) => ({ day: c.day, period: c.period, name: c.name })));
    debugLog("[LIVE] focusedCourseOptions =", courseOptions.map((c) => ({ day: c.day, period: c.period, name: c.name })));
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
    selectedKey = defaultSelectedCourseKey(courseOptions, date);
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
    aiReplyLanguage = cfg.reply_language || "ja";
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
      saveProgress = snapshot.course?.is_free_note || skipAiSummarization
        ? "ファイルに書き出し中…"
        : "TODO候補とDDLを判定中…";
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
    saveProgress = "TODOを追加中…";
    try {
      const added = await saveLiveGeneratedTodos(selected, todoDraftSourcePath);
      setMessage("success", added.length > 0 ? `${added.length}件のTODOを追加しました` : "既存のTODOと重複していたため追加はありません");
      todoDrafts = [];
      todoDraftSourcePath = "";
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      todoDraftSaving = false;
      saveProgress = "";
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

  function cancelClearCourseData() {
    confirmClear = false;
  }

  function confirmClearCourseData() {
    confirmClear = false;
    void executeClearCourseData();
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
    debugLog("[Live] flush timer started, interval =", intervalMs, "ms");
    flushTimer = setInterval(async () => {
      debugLog("[Live] flush timer tick");
      try {
        snapshot = await liveFlushSummary(true);
        debugLog("[Live] flush done, summaries =", snapshot.summaries.length);
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

<div class="live-root view" class:board-expanded={whiteboardExpanded}>
  <LiveTopCapsule
    {snapshot}
    {sttPhase}
    {liveBadgeLabel}
    {remainingLabel}
    {saveProgress}
    {renderedCourseOptions}
    bind:selectedKey
    {pageLoading}
    {hasContent}
    {selectedCourse}
    {busy}
    {canStart}
    {canStartFreeNote}
    {canStop}
    {sttListening}
    {sttBooting}
    {confirmClear}
    {courseKey}
    {courseLabel}
    onStartLive={startLive}
    onStartFreeNote={startFreeNote}
    onClearCourseData={clearCourseData}
    onCancelClear={cancelClearCourseData}
    onConfirmClear={confirmClearCourseData}
    onStopLive={stopLive}
    onPauseLive={pauseLive}
    onResumeLive={resumeLive}
  />

  <!-- ─── Main scrollable area ─── -->
  <div class="main-scroll" bind:this={scrollEl} use:bindManualScroll role="region" aria-label="LIVE transcript">
    <div class="scroll-spacer-top"></div>

    <LiveNotice {notice} onOpenAiSettings={() => openSettingsWindow("ai")} />

    <LiveSummaryCard
      summaries={snapshot.summaries}
      {activeSummaryIdx}
      {summaryExpanded}
      {renderMd}
      onSelectSummaryView={selectSummaryView}
      onExpand={expandSummary}
      onCollapse={collapseSummary}
      {bindSummaryOverlayDismiss}
    />

    <LiveTranscriptStage
      {pageLoading}
      {hasContent}
      {snapshot}
      {partialText}
      {saveProgress}
      {sttBooting}
      {sttBootMessage}
      {todoDrafts}
      {todoDraftsWithDeadlineCount}
      {todoDraftSaving}
      {lastSaved}
      {showSaveNotif}
      {visibleLines}
      {hiddenLineCount}
      {renderMd}
      {extractOverallSummary}
      onToggleTodoDraft={toggleTodoDraft}
      onCloseTodoDrafts={closeTodoDrafts}
      onConfirmTodoDrafts={confirmTodoDrafts}
    />



    <div class="scroll-spacer-bottom"></div>
  </div>

  <LiveScrollToBottomButton visible={showScrollBtn && hasContent} onScrollToBottom={scrollToBottom} />

  <LiveRightRail
    {activeWhiteboardLayout}
    {activeSummaryTerms}
    {termsCollapsed}
    {collapsedTermPreview}
    {termCardIdx}
    {termFloatLabels}
    {termStackOffset}
    onOpenWhiteboard={openWhiteboardOverlay}
    onToggleTermsCollapsed={toggleTermsCollapsed}
    onSelectTermCard={selectTermCard}
    onTermCardPrev={termCardPrev}
    onTermCardNext={termCardNext}
  />

  {#if activeWhiteboardLayout && whiteboardExpanded}
    <LiveWhiteboardPage
      {activeWhiteboardLayout}
      {activeWhiteboardStage}
      {termFloatLabels}
      {whiteboardZoom}
      {whiteboardPanX}
      {whiteboardPanY}
      {whiteboardDragStart}
      {selectedBoardNodeId}
      {boardHighlight}
      bind:boardCanvasWidth
      bind:boardCanvasHeight
      {bindWhiteboardOverlayDismiss}
      onClose={closeWhiteboardOverlay}
      onZoomOut={() => setWhiteboardZoom(whiteboardZoom - 0.15)}
      onResetZoom={resetWhiteboardView}
      onZoomIn={() => setWhiteboardZoom(whiteboardZoom + 0.15)}
      onWheel={handleWhiteboardWheel}
      onPointerDown={handleWhiteboardPointerDown}
      onPointerMove={handleWhiteboardPointerMove}
      onPointerUp={handleWhiteboardPointerUp}
      onClearSelection={clearBoardSelection}
      onToggleNodeSelection={toggleBoardNodeSelection}
    />
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
  /* When the whiteboard overlay is open, let .board-page bleed into the
     view-panel padding so it fills the full .content area.
     Only change overflow (NOT padding) to avoid any layout reflow / flash. */
  :global(.view-panel:has(.live-root.board-expanded)) {
    overflow: hidden;
  }
  .live-root.board-expanded {
    overflow: visible;
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


</style>
