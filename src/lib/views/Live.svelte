<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onCacheUpdate, activeTab, liveTodoPending } from "../stores";
  import LiveRightRail from "./live/LiveRightRail.svelte";
  import LiveScrollToBottomButton from "./live/LiveScrollToBottomButton.svelte";
  import LiveSummaryDetailPage from "./live/LiveSummaryDetailPage.svelte";
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
    liveGenerateOverallSummary,
    liveGetSession,
    livePeekDayCache,
    liveStartSession,
    isDemoActive,
    openSettingsWindow,
    openSubtitleOverlay,
    closeSubtitleOverlay,
    type LiveCourseInfo,
    type LiveSaveResult,
    type LiveSessionSnapshot,
  } from "../api";
  import type { ScheduleResponse } from "../types";
  import { PERIOD_TIMES } from "../types";
  import { buildCourseSlots, type CourseSlot } from "../schedule";
  import { computeWhiteboardLayout, whiteboardTopics } from "../whiteboardLayout";
  import type {
    LiveControlModel,
    NoticeAction,
    NoticeKind,
    NoticeSource,
    NoticeState,
    SttPhase,
    WhiteboardStagePreset,
  } from "./live/liveTypes";

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
  let lastPartialSeq = 0;
  let sttListening = $state(false);
  let sttPhase = $state<SttPhase>("idle");
  let busy = $state(false);
  let pageLoading = $state(true);
  let notice = $state<NoticeState>(null);
  let liveReady = $state(false);
  let readinessMessage = $state("");
  let lastSaved = $state<LiveSaveResult | null>(null);
  let showSaveNotif = $state(false);
  let saveProgress = $state("");
  // Structured progress for the LIVE 終了/要約 pipeline so the capsule can show a
  // step counter + progress bar + "next step" hint instead of a single label.
  let saveSteps = $state<string[]>([]);
  let saveStepIndex = $state(0);

  const STOP_STEP = "録音を停止中";
  const AUTO_STOP_STEP = "自動終了の準備中";
  const RECORD_WRITE_STEP = "録音内容を保存中";
  const SUMMARY_STEP = "AI要約を生成中";
  const FINAL_WRITE_STEP = "AI反映版を書き出し中";
  const TODO_STEP = "やること・締切を抽出中";
  const OVERALL_STEP = "全体要約を生成中";

  function beginSave(steps: string[], index = 0) {
    saveSteps = steps;
    saveStepIndex = Math.min(Math.max(index, 0), steps.length - 1);
    saveProgress = steps[saveStepIndex] ? `${steps[saveStepIndex]}…` : "";
  }
  function gotoSave(label: string) {
    const i = saveSteps.indexOf(label);
    if (i >= 0) saveStepIndex = i;
    saveProgress = `${label}…`;
  }
  function gotoSaveIfPresent(label: string) {
    if (saveSteps.includes(label)) gotoSave(label);
  }
  function applyFinishProgress(step: string) {
    if (step === "record_saved" || step === "ai") {
      if (saveSteps.includes(SUMMARY_STEP)) {
        gotoSave(SUMMARY_STEP);
      } else {
        gotoSaveIfPresent(FINAL_WRITE_STEP);
      }
    } else if (step === "final_save") {
      gotoSaveIfPresent(FINAL_WRITE_STEP);
    }
  }
  function endSave() {
    saveSteps = [];
    saveStepIndex = 0;
    saveProgress = "";
  }
  let summaryViewIndex = $state(-1); // -1 = auto (latest)
  let summaryDetailOpen = $state(false); // full secondary page (not a popup)
  let overallSummary = $state("");
  let overallSummaryAt = $state(""); // "HH:MM" the overall summary was generated
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;
  let scheduleFocusTimer: ReturnType<typeof setInterval> | null = null;
  let aiReplyLanguage = $state("ja");
  let timeTimer: ReturnType<typeof setInterval> | null = null;
  let now = $state(new Date());
  let scrollEl: HTMLElement | null = null;
  const MIN_AI_SUMMARIZATION_MS = 2 * 60 * 1000;
  const NO_EFFECTIVE_SPEECH_AUTO_PAUSE_MS = 10 * 60 * 1000;
  const PAUSED_AUTO_FINISH_MS = 20 * 60 * 1000;
  const LIVE_AUTO_GUARD_INTERVAL_MS = 60 * 1000;
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

  function openSummaryDetail() {
    // Open the detail on the segment the rail is currently showing (not the
    // overall), so tapping a card stays in context.
    if (activeSegmentIdx >= 0) summaryViewIndex = activeSegmentIdx;
    summaryDetailOpen = true;
  }

  function openOverallSummary() {
    // 全体要約 is always the trailing entry when present; jump straight into it.
    summaryViewIndex = summaryEntries.length - 1;
    summaryDetailOpen = true;
  }

  function selectRailSegment(idx: number) {
    summaryViewIndex = idx;
  }

  function closeSummaryDetail() {
    summaryDetailOpen = false;
  }

  function selectSummaryView(event: MouseEvent, idx: number) {
    event.stopPropagation();
    summaryViewIndex = idx;
  }

  // The stage-summary card shows the periodic chunks AND — when one has been
  // generated — the "現在までの全体要約" as a trailing entry (no longer a
  // separate floating card at the top of the history). The overall entry is
  // always last, so auto-select (-1) surfaces it the moment it appears.
  const summaryEntries = $derived([
    ...snapshot.summaries.map((c) => ({
      range_label: c.range_label,
      body: c.body,
      isOverall: false,
      terms: c.terms ?? [],
    })),
    ...(overallSummary
      ? [
          {
            range_label: `${overallSummaryAt}までの全体要約`,
            body: overallSummary,
            isOverall: true,
            terms: [],
          },
        ]
      : []),
  ]);
  const activeEntryIdx = $derived(
    summaryViewIndex < 0 || summaryViewIndex >= summaryEntries.length
      ? summaryEntries.length - 1
      : summaryViewIndex
  );
  // The right-rail cards (summary / terms / whiteboard) always reflect a real
  // SEGMENT — the 全体要約 never drives them (全体要約不参与卡片显示). When the
  // overall entry happens to be the selected one (e.g. auto = trailing entry),
  // the rail falls back to the latest segment so the cards still load.
  const segmentCount = $derived(snapshot.summaries.length);
  const activeSegmentIdx = $derived(
    segmentCount === 0
      ? -1
      : activeEntryIdx >= 0 && activeEntryIdx < segmentCount
        ? activeEntryIdx
        : segmentCount - 1,
  );
  // Chunk index used for term annotations and the whiteboard.
  const activeSummaryIdx = $derived(activeSegmentIdx);

  // Rail control-strip status: either "generating" or a countdown to the next
  // scheduled periodic summary (both backed by snapshot.next_summary_at_ms /
  // .summarizing — see live.rs). `now` ticks every 30s, so minute resolution.
  const summarizing = $derived(!!snapshot.summarizing);
  const summaryStatusLabel = $derived.by(() => {
    if (summarizing) return "要約を生成中…";
    if (!snapshot.active) return "";
    const at = snapshot.next_summary_at_ms;
    if (!at) return "";
    const diff = at - now.getTime();
    if (diff < 60_000) return "まもなく次の要約";
    return `次の要約まで約${Math.ceil(diff / 60_000)}分`;
  });

  const activeSummaryTerms = $derived.by(() => {
    const chunk = snapshot.summaries[activeSummaryIdx];
    return (chunk?.terms ?? []).filter((term) => term.term?.trim() && term.explanation?.trim());
  });

  // Close the detail sub-page if its content goes away (session stopped/cleared).
  $effect(() => {
    if (summaryEntries.length === 0 && untrack(() => summaryDetailOpen)) {
      summaryDetailOpen = false;
    }
  });

  // Stacked-card pager state for term annotations.
  // No wheel interception — switching is via click on a back card or the prev/next chips.
  let termCardIdx = $state(0);
  // activeSummaryTerms is a $derived built with .filter(), so it returns a NEW
  // array reference every time the snapshot updates (every few hundred ms during
  // a live session). Watching the array itself would reset termCardIdx on every
  // transcript tick. Instead, derive a stable primitive fingerprint and only reset
  // when the term set actually changes.
  const termFingerprint = $derived(
    activeSummaryTerms.map((t) => t.term).join("|")
  );
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
    const preset = getWhiteboardStagePreset(activeWhiteboardLayout);
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
    const preset = getWhiteboardStagePreset(activeWhiteboardLayout);
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
  const isFreeNoteSession = $derived(Boolean(snapshot.course?.is_free_note));
  const termFloatLabels = $derived.by(() => {
    const sourceLabel = isFreeNoteSession
      ? { zh: "录音依据", en: "Recording source", ko: "녹음 근거", ja: "録音内根拠" }
      : { zh: "课堂依据", en: "Class source", ko: "수업 근거", ja: "講義内根拠" };
    switch ((aiReplyLanguage || "ja").toLowerCase()) {
      case "zh":
      case "zh-cn":
      case "cn":
        return { title: "用语注释", boardTitle: "知识整理", empty: "本段没有需要解释的术语", source: sourceLabel.zh, externalSource: "外部来源", externalNode: "外部", collapse: "折叠", expand: "展开", previous: "上一个术语", next: "下一个术语", selectAll: "全选", deselectAll: "取消全选" };
      case "en":
        return { title: "Key Terms", boardTitle: "Knowledge Board", empty: "No terms for this segment", source: sourceLabel.en, externalSource: "External source", externalNode: "External", collapse: "Collapse", expand: "Expand", previous: "Previous term", next: "Next term", selectAll: "Select all", deselectAll: "Deselect all" };
      case "ko":
        return { title: "핵심 용어", boardTitle: "지식 정리", empty: "이 구간의 용어 설명이 없습니다", source: sourceLabel.ko, externalSource: "외부 출처", externalNode: "외부", collapse: "접기", expand: "펼치기", previous: "이전 용어", next: "다음 용어", selectAll: "전체 선택", deselectAll: "선택 해제" };
      default:
        return { title: "用語注釈", boardTitle: "知識整理", empty: "この区間の注釈はありません", source: sourceLabel.ja, externalSource: "外部出典", externalNode: "外部", collapse: "折りたたむ", expand: "展開", previous: "前の用語", next: "次の用語", selectAll: "すべて選択", deselectAll: "選択解除" };
    }
  });

  const rawWhiteboard = $derived(snapshot.summaries[activeSummaryIdx]?.whiteboard ?? null);
  // Topic switcher: a dense board carries several main topics; the bottom bar
  // lets the user show one (default) or several at a time instead of cramming
  // every topic onto one canvas.
  const whiteboardTopicList = $derived(whiteboardTopics(rawWhiteboard));
  let selectedTopicIds = $state<string[]>([]);
  // Reset the selection only when the *set* of topics changes — the board
  // object is re-derived on every transcript tick, but as long as the topic
  // ids are unchanged we keep the user's current pick.
  const whiteboardTopicFingerprint = $derived(whiteboardTopicList.map((t) => t.id).join("|"));
  $effect(() => {
    whiteboardTopicFingerprint;
    untrack(() => {
      const ids = whiteboardTopicList.map((t) => t.id);
      const kept = selectedTopicIds.filter((id) => ids.includes(id));
      // Default to the first topic only — one topic shown at a time.
      selectedTopicIds = kept.length ? kept : ids.slice(0, 1);
    });
  });
  const activeWhiteboardLayout = $derived.by(() =>
    computeWhiteboardLayout(rawWhiteboard, {
      fallbackBoardTitle: termFloatLabels.boardTitle,
      externalNodeLabel: termFloatLabels.externalNode,
      topicIds: whiteboardTopicList.length > 1 ? selectedTopicIds : undefined,
    })
  );
  // The rail preview is an overview — it always shows the whole board; topic
  // filtering only applies inside the expanded overlay.
  const previewWhiteboardLayout = $derived(
    computeWhiteboardLayout(rawWhiteboard, {
      fallbackBoardTitle: termFloatLabels.boardTitle,
      externalNodeLabel: termFloatLabels.externalNode,
    })
  );
  function toggleWhiteboardTopic(id: string) {
    if (selectedTopicIds.includes(id)) {
      // Keep at least one topic selected.
      if (selectedTopicIds.length > 1) {
        selectedTopicIds = selectedTopicIds.filter((x) => x !== id);
      }
    } else {
      selectedTopicIds = [...selectedTopicIds, id];
    }
    selectedBoardNodeId = null;
    // The stage size depends on node count, so refit the view to the new set.
    initialFitDone = false;
  }
  function toggleAllWhiteboardTopics() {
    const ids = whiteboardTopicList.map((t) => t.id);
    // One click: select every topic, or — when all are already on — collapse
    // back to just the first.
    selectedTopicIds = selectedTopicIds.length >= ids.length ? ids.slice(0, 1) : ids;
    selectedBoardNodeId = null;
    initialFitDone = false;
  }
  const activeWhiteboardStage = $derived(getWhiteboardStagePreset(activeWhiteboardLayout));

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
  // initial zoom so the stage fills the real available area. We do this once
  // per open (initialFitDone guard) to avoid fighting with user pans/zooms —
  // and again whenever the topic selection changes, since that resizes the
  // stage (toggleWhiteboardTopic clears the guard).
  $effect(() => {
    void selectedTopicIds;
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

  function getWhiteboardStagePreset(layout: typeof activeWhiteboardLayout): WhiteboardStagePreset {
    // The forest layout reports the exact pixel canvas it was computed for;
    // the auto-fit effect then derives a zoom that fits it to the viewport.
    if (layout?.stage) {
      return { width: layout.stage.width, height: layout.stage.height, zoom: 0.8 };
    }
    // No layout yet (board still null) — a neutral default until one arrives.
    return { width: 1040, height: 660, zoom: 0.96 };
  }

  let unlistenPartial: (() => void) | null = null;
  let unlistenFinal: (() => void) | null = null;
  let unlistenState: (() => void) | null = null;
  let unlistenError: (() => void) | null = null;
  let unlistenInfo: (() => void) | null = null;
  let unlistenLive: (() => void) | null = null;
  let unlistenSaved: (() => void) | null = null;
  let unlistenFinishProgress: (() => void) | null = null;
  let unlistenSummaryError: (() => void) | null = null;
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

  function formatDuration(ms: number): string {
    const totalMinutes = Math.max(0, Math.floor(ms / 60_000));
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    if (hours <= 0) return `${minutes}分`;
    return `${hours}:${String(minutes).padStart(2, "0")}`;
  }

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

  // Free-note is now an option inside the mode selector, not a separate button.
  const FREE_NOTE_KEY = "__free_note__";
  const freeNoteSelected = $derived(selectedKey === FREE_NOTE_KEY);
  const canStart = $derived(
    !snapshot.active && liveReady && !busy && (freeNoteSelected || !!selectedCourse),
  );
  const canStop = $derived(snapshot.active && !busy);
  const canGenerateOverallSummary = $derived(snapshot.active && snapshot.transcript_lines.length > 0 && !busy);

  const activeTargetLabel = $derived.by(() => {
    if (snapshot.course) {
      return snapshot.course.is_free_note ? "自由ノート" : snapshot.course.course_name;
    }
    if (freeNoteSelected) return "自由ノート";
    return selectedCourse?.name ?? "録音対象";
  });

  const selectedTargetMeta = $derived.by(() => {
    if (pageLoading) return "読み込み中";
    if (freeNoteSelected) return "自由入力";
    if (!selectedCourse) return "授業候補なし";
    const room = selectedCourse.room?.trim();
    return `${selectedCourse.period}限${room ? `・${room}` : ""}`;
  });

  const elapsedLabel = $derived.by(() => {
    if (!snapshot.active || !snapshot.started_at) return "";
    const startedAt = snapshotStartedAtMs(snapshot.started_at);
    if (startedAt == null) return "";
    return `経過 ${formatDuration(now.getTime() - startedAt)}`;
  });

  const pausedDurationLabel = $derived.by(() => {
    if (!snapshot.active || sttListening || sttBooting || !pausedSinceMs) return "";
    return `停止 ${formatDuration(now.getTime() - pausedSinceMs)}`;
  });

  const pauseHintLabel = $derived.by(() => {
    if (!snapshot.active || sttListening || sttBooting || !pausedSinceMs) return "";
    const remainingMs = Math.max(0, PAUSED_AUTO_FINISH_MS - (now.getTime() - pausedSinceMs));
    return `自動保存まで ${formatDuration(remainingMs)}`;
  });

  const lineCountLabel = $derived(`${snapshot.transcript_lines.length}行`);
  const summaryCountLabel = $derived(
    snapshot.summaries.length > 0 ? `${snapshot.summaries.length}要約` : "要約待ち",
  );

  const liveControl = $derived.by((): LiveControlModel => {
    const saved = !snapshot.active && showSaveNotif && !!lastSaved;
    const blocked = !snapshot.active && !pageLoading && !liveReady;
    const thinking = !!saveProgress;
    const phase = thinking
      ? "thinking"
      : snapshot.active && sttBooting
        ? "booting"
        : snapshot.active && sttListening
          ? "recording"
          : snapshot.active
            ? "paused"
            : saved
              ? "saved"
              : blocked
                ? "blocked"
                : "idle";

    const statusLabel =
      phase === "recording" ? "REC"
      : phase === "booting" ? "準備中"
      : phase === "paused" ? "一時停止"
      : phase === "thinking" ? "処理中"
      : phase === "saved" ? "保存完了"
      : phase === "blocked" ? "要設定"
      : "LIVE";

    const targetMeta = snapshot.active
      ? (phase === "paused" ? pausedDurationLabel : remainingLabel || elapsedLabel)
      : selectedTargetMeta;

    const detailLabel =
      phase === "thinking" ? saveProgress
      : phase === "booting" ? sttBootMessage
      : phase === "paused" ? pauseHintLabel || "転写は一時停止中です"
      : phase === "blocked" ? readinessMessage || "AI設定を確認してください"
      : phase === "saved" ? "ノートを書き出しました"
      : phase === "recording" ? "文字起こし中"
      : hasContent && selectedCourse ? "保存済みの内容があります" : "録音対象を選んで開始";

    const primaryAction =
      phase === "blocked" ? "settings"
      : phase === "recording" ? "pause"
      : phase === "paused" ? "resume"
      : phase === "idle" || phase === "saved" ? "start"
      : "none";

    const primaryLabel =
      primaryAction === "settings" ? "AI設定"
      : primaryAction === "pause" ? "一時停止"
      : primaryAction === "resume" ? "再開"
      : primaryAction === "start" ? "開始"
      : "処理中";

    const primaryDisabled =
      primaryAction === "settings" ? false
      : primaryAction === "pause" || primaryAction === "resume" ? busy
      : primaryAction === "start" ? !canStart
      : true;

    return {
      phase,
      tone:
        phase === "recording" ? "recording"
        : phase === "booting" || phase === "thinking" ? "thinking"
        : phase === "paused" ? "paused"
        : phase === "blocked" ? "blocked"
        : phase === "saved" ? "saved"
        : canStart ? "ready" : "neutral",
      statusLabel,
      targetLabel: activeTargetLabel,
      targetMeta,
      progressLabel: phase === "thinking" ? saveProgress : "",
      saveSteps: phase === "thinking" ? saveSteps : [],
      saveStepIndex,
      detailLabel,
      elapsedLabel,
      lineCountLabel,
      summaryCountLabel,
      pauseHintLabel,
      primaryAction,
      primaryLabel,
      primaryDisabled,
      primaryTitle:
        primaryAction === "settings" ? "AI設定を開く"
        : primaryAction === "pause" ? "録音を一時停止"
        : primaryAction === "resume" ? "録音を再開"
        : primaryAction === "start" ? "録音を開始"
        : detailLabel,
      showModeSelect: !snapshot.active && phase !== "thinking",
      showSummaryAction: snapshot.active && (phase === "recording" || phase === "paused"),
      showSaveAction: snapshot.active && (phase === "recording" || phase === "paused"),
      showClearAction: !snapshot.active && hasContent && !!selectedCourse,
    };
  });

  // When the selected course changes (and session not active), load cached history
  $effect(() => {
    const course = selectedCourse;
    // Use untrack for snapshot/showSaveNotif reads: writing snapshot inside the
    // async .then() would otherwise re-trigger this effect → infinite loop.
    if (!course || untrack(() => snapshot.active || showSaveNotif)) return;
    livePeekDayCache(toLiveCourse(course)).then((cached) => {
      if (untrack(() => snapshot.active || showSaveNotif)) return;
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

  // STT init progress (確認中 / 起動中 / 初期化中) is surfaced by the top capsule
  // itself (準備中 + boot message), so the redundant inline notice bar is gone.
  // Kept as a no-op so the call sites + clearSttNotice stay structurally intact.
  function setSttNotice(_message: string) {}

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
    if (
      preserveSelection &&
      (selectedKey === FREE_NOTE_KEY ||
        courseOptions.some((course) => courseKey(course) === selectedKey))
    ) {
      return;
    }
    // Fall back to free-note when there are no course candidates, so the mode
    // selector always has a valid selection.
    selectedKey = defaultSelectedCourseKey(courseOptions, date) || FREE_NOTE_KEY;
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
    const ready = await isAiReady();
    liveReady = ready;
    if (liveReady) {
      readinessMessage = "";
      clearReadinessNotice();
      return;
    }
    readinessMessage = buildReadinessMessage(cfg, ready);
    setReadinessNotice(readinessMessage);
  }

  async function ensureReadyToStart() {
    await refreshReadiness();
    if (!liveReady) {
      throw new Error(readinessMessage || (notice?.source === "readiness" ? notice.text : "AIの準備ができていません"));
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
    cancelSessionOnStartFailure = true;
    try {
      await ensureReadyToStart();
      sttPhase = "starting";
      setSttNotice("音声入力を起動中…");
      snapshot = await liveStartSession(course);
      overallSummary = "";
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
    } catch (e: any) {
      cancelSessionOnStartFailure = false;
      sttPhase = "idle";
      clearSttNotice();
      setMessage("error", e?.message || String(e));
      try {
        await liveCancelSession();
        snapshot = await liveGetSession();
      } catch {}
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

  // Dispatch by the unified mode selector: free-note option vs a course.
  async function startSelected() {
    if (freeNoteSelected) {
      await startFreeNote();
    } else {
      await startLive();
    }
  }

  async function pauseLiveInternal(automated = false) {
    busy = true;
    clearNotice();
    clearSttNotice();
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
      markLivePaused();
      // Manual pause needs no toast — the island already shows the 一時停止
      // state. The automated case keeps a warning that explains *why* it paused.
      if (automated) {
        setNotice("warning", "10分間有効な音声が認識されなかったため、LIVEを一時停止しました。");
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
    } catch (e: any) {
      cancelSessionOnStartFailure = false;
      sttPhase = "idle";
      markLivePaused();
      clearSttNotice();
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  async function stopLiveInternal(automated = false) {
    busy = true;
    clearNotice();
    clearSttNotice();
    cancelSessionOnStartFailure = false;
    sttPhase = "idle";
    const stopLabel = automated ? AUTO_STOP_STEP : STOP_STEP;
    // Provisional full pipeline; corrected once we know empty/skip below.
    beginSave([stopLabel, RECORD_WRITE_STEP, SUMMARY_STEP, FINAL_WRITE_STEP, TODO_STEP]);
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
        beginSave([stopLabel, RECORD_WRITE_STEP], 1);
        const ended = await liveFinishSession();
        lastSaved = ended.saved ? ended : null;
        snapshot = await liveGetSession();
        clearLiveAutoLifecycle();
        endSave();
        if (!ended.saved) {
          setMessage("success", automated ? "20分間再開されなかったため、LIVEを自動終了しました" : "LIVEを終了しました");
        }
        return;
      }
      const skipAiSummarization = shouldSkipAiSummarizationForSnapshot(snapshot);
      if (skipAiSummarization) {
        beginSave([stopLabel, RECORD_WRITE_STEP, FINAL_WRITE_STEP, TODO_STEP], 1);
      } else {
        gotoSave(RECORD_WRITE_STEP);
      }
      const saved = await liveFinishSession();
      lastSaved = saved.saved ? saved : null;
      overallSummary = "";
      snapshot = await liveGetSession();
      clearLiveAutoLifecycle();
      endSave();
      if (saved.saved) {
        showSaveNotif = true;
        setTimeout(() => { showSaveNotif = false; }, 6000);
        if (automated) {
          setMessage("success", "20分間再開されなかったため、LIVEを自動保存しました");
        }
        // TODO/DDL judgment runs in the background; jump to the TODO page so the
        // suggestions show up there to add once ready, instead of blocking here.
        if (saved.todos_pending) {
          liveTodoPending.set(true);
          activeTab.set("todo");
        }
      } else {
        setMessage("success", automated ? "20分間再開されなかったため、LIVEを自動終了しました" : "LIVEを終了しました");
      }
    } catch (e: any) {
      endSave();
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  async function stopLive() {
    await stopLiveInternal(false);
  }

  async function generateOverallSummary() {
    if (!canGenerateOverallSummary) return;
    busy = true;
    clearNotice();
    beginSave([OVERALL_STEP]);
    try {
      overallSummary = await liveGenerateOverallSummary();
      const at = new Date();
      overallSummaryAt = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
      snapshot = await liveGetSession();
      // Reset to auto so the freshly-added overall entry (always last) is shown.
      summaryViewIndex = -1;
      setMessage("success", "現在までの全体要約を生成しました");
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      endSave();
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
      overallSummary = "";
      setMessage("success", `${name} のキャッシュをクリアしました`);
    } catch (e: any) {
      setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
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

      unlistenScheduleCache = onCacheUpdate<ScheduleResponse>("schedule_data", (fresh) => {
        applyScheduleSnapshot(fresh, new Date(), true);
      });
      scheduleFocusTimer = setInterval(refreshFocusedCoursesFromClock, 60_000);

      unlistenPartial = await listen<{ text: string; caller: string; seq?: number }>("stt-partial", (event) => {
        if (event.payload.caller !== "live") return;
        const seq = event.payload.seq ?? 0;
        if (seq > 0 && seq < lastPartialSeq) return;
        if (seq > 0) lastPartialSeq = seq;
        partialText = event.payload.text || "";
      });
      unlistenFinal = await listen<{ text: string; caller: string; seq?: number }>("stt-final", async (event) => {
        if (event.payload.caller !== "live") return;
        if (!snapshot.active) return;
        const seq = event.payload.seq ?? 0;
        // An older final can finish after a newer partial. Keep the live line,
        // but still commit the finished sentence to the transcript.
        if (seq === 0 || seq >= lastPartialSeq) {
          if (seq > 0) lastPartialSeq = seq;
          partialText = "";
        }
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
        sttListening = event.payload.state === "initializing" || event.payload.state === "listening";
        if (event.payload.state === "initializing") {
          sttPhase = "initializing";
          setSttNotice("マイクと音声認識を初期化中…");
        } else if (event.payload.state === "listening") {
          sttPhase = "listening";
          clearSttNotice();
          cancelSessionOnStartFailure = false;
          // No green "開始/再開" confirmation bar — the capsule flips to REC.
          if (!wasListening) markLiveListeningStarted();
        } else {
          sttPhase = "idle";
          clearSttNotice();
          if (snapshot.active) markLivePaused();
        }
        if (sttListening && !wasListening) autoFollow = true;
      });
      unlistenError = await listen<{ message: string; caller: string }>("stt-error", (event) => {
        if (event.payload.caller !== "live") return;
        const wasStarting = sttPhase === "starting" || sttPhase === "initializing";
        sttListening = false;
        sttPhase = "idle";
        clearSttNotice();
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
        if (!snapshot.active) {
          // Backend-owned flush driver stops itself when the session changes.
        }
      });
      unlistenSaved = await listen<LiveSaveResult>("live-session-saved", (event) => {
        lastSaved = event.payload;
      });
      unlistenFinishProgress = await listen<{ step: string }>("live-finish-progress", (event) => {
        applyFinishProgress(event.payload.step);
      });
      unlistenSummaryError = await listen<{ message: string }>("live-summary-error", (event) => {
        // A scheduled AI summary failed. The backend has already cleared the
        // "生成中" flag; surface the reason (incl. any provider error code) so
        // the user knows it stalled instead of silently waiting for the next tick.
        const detail = (event.payload.message || "").trim();
        setNotice("error", detail ? `AI要約に失敗しました：${detail}` : "AI要約に失敗しました。次の区間で再試行します。", {
          source: "general",
          autoClearMs: 8000,
        });
      });
      unlistenAiConfig = await listen("ai-config-changed", () => {
        refreshReadiness().catch((e: any) => {
          liveReady = false;
          readinessMessage = e?.message || "LIVEにはAIの準備が必要です。AI設定を確認してください。";
          setReadinessNotice(readinessMessage);
        });
      });
      // Automatic Live summary/whiteboard flushing is owned by the backend.
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
    unlistenFinishProgress?.();
    unlistenSummaryError?.();
    unlistenAiConfig?.();
    unlistenScheduleCache?.();
    unlistenWinFocus?.();
    unlistenWinBlur?.();
    if (scheduleFocusTimer) clearInterval(scheduleFocusTimer);
    // Live ページを離れたら浮窗を再表示
    openSubtitleOverlay().catch(() => {});
  });
</script>

<div class="live-root view" class:board-expanded={whiteboardExpanded || summaryDetailOpen}>
  <LiveTopCapsule
    control={liveControl}
    {notice}
    {renderedCourseOptions}
    bind:selectedKey
    {pageLoading}
    {busy}
    {canStop}
    {canGenerateOverallSummary}
    {confirmClear}
    freeNoteKey={FREE_NOTE_KEY}
    {courseKey}
    {courseLabel}
    onStart={startSelected}
    onClearCourseData={clearCourseData}
    onCancelClear={cancelClearCourseData}
    onConfirmClear={confirmClearCourseData}
    onStopLive={stopLive}
    onGenerateOverallSummary={generateOverallSummary}
    onPauseLive={pauseLive}
    onResumeLive={resumeLive}
    onOpenAiSettings={() => openSettingsWindow("ai")}
  />

  <!-- ─── Main scrollable area ─── -->
  <div class="main-scroll" bind:this={scrollEl} use:bindManualScroll role="region" aria-label="LIVE transcript">
    <div class="scroll-spacer-top"></div>

    <LiveTranscriptStage
      {pageLoading}
      {hasContent}
      {snapshot}
      {partialText}
      {lastSaved}
      {showSaveNotif}
      {visibleLines}
      {hiddenLineCount}
      {renderMd}
      {extractOverallSummary}
    />



    <div class="scroll-spacer-bottom"></div>
  </div>

  <LiveScrollToBottomButton visible={showScrollBtn && hasContent} onScrollToBottom={scrollToBottom} />

  <LiveRightRail
    summaryEntries={summaryEntries}
    activeSummaryIdx={activeSegmentIdx}
    summarySegmentCount={snapshot.summaries.length}
    {renderMd}
    onOpenSummaryDetail={openSummaryDetail}
    onSelectSegment={selectRailSegment}
    onOpenOverall={openOverallSummary}
    {summarizing}
    {summaryStatusLabel}
    previewLayout={previewWhiteboardLayout}
    {activeSummaryTerms}
    {termCardIdx}
    {termFloatLabels}
    {termStackOffset}
    onOpenWhiteboard={openWhiteboardOverlay}
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
      topics={whiteboardTopicList}
      {selectedTopicIds}
      onToggleTopic={toggleWhiteboardTopic}
      onToggleAllTopics={toggleAllWhiteboardTopics}
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

  {#if summaryDetailOpen && summaryEntries.length > 0}
    <LiveSummaryDetailPage
      entries={summaryEntries}
      activeIdx={activeEntryIdx}
      {renderMd}
      onSelectSummaryView={selectSummaryView}
      onClose={closeSummaryDetail}
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
