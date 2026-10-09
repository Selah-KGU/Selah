<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import { get } from "svelte/store";
  import { listen } from "@tauri-apps/api/event";
  import { ResourceScope, ResourceSlot, acquireResourceGroup } from "../resourceScope";
  import { LatestViewRead } from "../latestViewRead";
  import { createCacheSyncQueue } from "../cacheSyncQueue";
  import { getSttStreamState, idleSttStreamState, liveSttPhase } from "../sttSessionApi";
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
  import { renderMd } from "./live/liveMarkdown";
  import {
    emptyLiveSurfaceSnapshot,
    liveSavedPreview,
    applyTranscriptDelta,
    isCurrentLiveSessionEvent,
    isCurrentLiveSttEvent,
    mergeLiveSnapshot,
    type LiveSavedPreview,
    type LiveSurfaceSnapshot,
  } from "./live/liveTranscript";
  import { applyLiveSessionNotification, type LiveSessionNotification } from "./live/liveNotification";
  import { applyLiveFinishProgress, isLiveBusy, LIVE_FINISH_LABELS, liveSavePresentation } from "./live/liveFinish";
  import { LiveTranscriptFollow } from "./live/liveTranscriptFollow";
  import type { LiveFinishProgress, LiveTranscriptUpdate } from "../liveSessionApi";
  import {
    getScheduleSnapshot,
    getAiConfig,
    isAiReady,
    liveCancelSession,
    liveClearDayCache,
    liveFinishSurface,
    liveGenerateOverallSummary,
    liveGetSurface,
    livePeekDaySurface,
    liveStartSurface,
    isDemoActive,
    openSettingsWindow,
    openSubtitleOverlay,
    closeSubtitleOverlay,
    type LiveCourseInfo,
    type LiveSurfaceSaveResult,
    type LiveSessionSnapshot,
  } from "../api";
  import type { ScheduleResponse } from "../types";
  import { PERIOD_TIMES } from "../types";
  import { buildCourseSlots, type CourseSlot } from "../schedule";
  import { computeWhiteboardLayout, prepareWhiteboardLayout, whiteboardLayoutReady, whiteboardTopics } from "../whiteboardLayout";
  import { expandLiveSurfaceSave, type CompactLiveSurfaceSaveResult } from "../liveBoardTransport";
  import type {
    LiveControlModel,
    NoticeAction,
    NoticeKind,
    NoticeSource,
    NoticeState,
    SttPhase,
    WhiteboardStagePreset,
  } from "./live/liveTypes";

  const resources = new ResourceScope();
  const sttSubscription = new ResourceSlot(resources);
  let scheduleData = $state<ScheduleResponse | null>(null);
  let allCourseOptions = $state<CourseSlot[]>([]);
  let courseOptions = $state<CourseSlot[]>([]);
  let selectedKey = $state("");
  let snapshot = $state.raw<LiveSurfaceSnapshot>(emptyLiveSurfaceSnapshot());
  let partialText = $state("");
  let lastPartialSeq = 0;
  let sttListening = $state(false);
  let sttPhase = $state<SttPhase>("idle");
  let busy = $state(false);
  const controlsBusy = $derived(isLiveBusy(snapshot, busy));
  let pageLoading = $state(true);
  let notice = $state<NoticeState>(null);
  let liveReady = $state(false);
  let readinessMessage = $state("");
  let lastSaved = $state.raw<LiveSavedPreview | null>(null);
  let showSaveNotif = $state(false);
  let saveNotifTimer: (() => void) | null = null;

  function rememberSaved(result: LiveSurfaceSaveResult) {
    if (!resources.active) return;
    if (snapshot.active && snapshot.session_id && snapshot.session_id !== result.snapshot.session_id) return;
    sessionUpdateRevision = Math.max(sessionUpdateRevision, result.snapshot.update_revision ?? 0);
    lastSaved = liveSavedPreview(result);
    if (!result.saved) return;
    showSaveNotif = true;
    saveNotifTimer?.();
    saveNotifTimer = resources.schedule(() => {
      showSaveNotif = false;
      saveNotifTimer = null;
    }, 6000);
  }
  let saveProgress = $state("");
  // Structured progress for the LIVE 終了/要約 pipeline so the capsule can show a
  // step counter + progress bar + "next step" hint instead of a single label.
  let saveSteps = $state<string[]>([]);
  let saveStepIndex = $state(0);
  const backendSaveProgress = $derived(liveSavePresentation(snapshot));

  const STOP_STEP = LIVE_FINISH_LABELS.stopping;
  const AUTO_STOP_STEP = "自動終了の準備中";
  const OVERALL_STEP = "全体要約を生成中";

  function beginSave(steps: string[], index = 0) {
    if (!resources.active) return;
    saveSteps = steps;
    saveStepIndex = Math.min(Math.max(index, 0), steps.length - 1);
    saveProgress = steps[saveStepIndex] ? `${steps[saveStepIndex]}…` : "";
  }
  function endSave() {
    if (!resources.active) return;
    saveSteps = [];
    saveStepIndex = 0;
    saveProgress = "";
  }
  let summaryViewIndex = $state(-1); // -1 = auto (latest)
  let summaryDetailOpen = $state(false); // full secondary page (not a popup)
  let overallSummary = $state("");
  let overallSummaryAt = $state(""); // "HH:MM" the overall summary was generated
  let noticeTimer: (() => void) | null = null;
  let scheduleFocusTimer: (() => void) | null = null;
  let liveMounted = false;
  let liveWindowHidden = $state(typeof document !== "undefined" && document.hidden);
  const liveSurfaceVisible = $derived($activeTab === "live" && !liveWindowHidden);
  let liveSurfaceWasVisible = false;
  let sttBindToken = 0;
  let sttListenersBound = false;
  let aiReplyLanguage = $state("ja");
  let now = $state(new Date());
  let scrollEl = $state<HTMLElement | null>(null);
  const NO_EFFECTIVE_SPEECH_AUTO_PAUSE_MS = 10 * 60 * 1000;
  const PAUSED_AUTO_FINISH_MS = 20 * 60 * 1000;
  const LIVE_AUTO_GUARD_INTERVAL_MS = 60 * 1000;
  let pendingStartSessionId: string | null = null;
  let lastEffectiveSpeechAtMs: number | null = null;
  let pausedSinceMs: number | null = null;
  let liveAutoGuardTimer: (() => void) | null = null;
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
  const summaries = $derived(snapshot.summaries);
  const summaryEntries = $derived([
    ...summaries.map((c) => ({
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
  const segmentCount = $derived(summaries.length);
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
    const chunk = summaries[activeSummaryIdx];
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
  // Transcript deltas preserve the chunk references. A recovery or segment
  // change may replace the terms with an equivalent array; keep the selection
  // unless the term set changes and the current index needs clamping.
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
    if (!whiteboardExpanded) return;
    if (!activeWhiteboardLayout) {
      whiteboardExpanded = false;
    }
  });
  function openWhiteboardOverlay() {
    // Enable the lazy layout before reading its actual stage preset.
    whiteboardExpanded = true;
    // Reset pan/zoom to preset defaults; the auto-fit effect will recalculate
    // once the canvas dimensions are measured after the DOM renders.
    const preset = getWhiteboardStagePreset(activeWhiteboardLayout);
    whiteboardZoom = preset.zoom;
    whiteboardPanX = 0;
    whiteboardPanY = 0;
    initialFitDone = false;
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

  const rawWhiteboard = $derived(summaries[activeSummaryIdx]?.whiteboard ?? null);
  $effect(() => {
    if (!rawWhiteboard) return;
    void prepareWhiteboardLayout().catch((error) => {
      console.warn("[Live] whiteboard layout failed to load:", error);
    });
  });
  // Topic switcher: a dense board carries several main topics; the bottom bar
  // lets the user show one (default) or several at a time instead of cramming
  // every topic onto one canvas.
  const whiteboardTopicList = $derived.by(() => {
    void $whiteboardLayoutReady;
    return whiteboardTopics(rawWhiteboard);
  });
  let selectedTopicIds = $state<string[]>([]);
  // Keep the selected topics when a recovery or segment change replaces the
  // board with equivalent topic IDs. Speech deltas retain the board reference.
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
  const activeWhiteboardLayout = $derived.by(() => {
    // The overview has its own layout. Defer the selected-topic forest until
    // its overlay is opened, including when a new summary arrives in LIVE.
    if (!whiteboardExpanded) return null;
    void $whiteboardLayoutReady;
    return computeWhiteboardLayout(rawWhiteboard, {
      fallbackBoardTitle: termFloatLabels.boardTitle,
      externalNodeLabel: termFloatLabels.externalNode,
      topicIds: whiteboardTopicList.length > 1 ? selectedTopicIds : undefined,
    });
  });
  // The rail preview is an overview — it always shows the whole board; topic
  // filtering only applies inside the expanded overlay.
  const previewWhiteboardLayout = $derived.by(() => {
    void $whiteboardLayoutReady;
    return computeWhiteboardLayout(rawWhiteboard, {
      fallbackBoardTitle: termFloatLabels.boardTitle,
      externalNodeLabel: termFloatLabels.externalNode,
    });
  });
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
  // primitives (segment index, overlay flag); a new layout within the same
  // segment should not clear a node selected by the user.
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

  let sessionEventVersion = 0;
  // Keep this watermark when the displayed snapshot becomes a course preview.
  let sessionUpdateRevision = 0;

  function mergeSessionRead(fresh: LiveSessionSnapshot | LiveSurfaceSnapshot) {
    snapshot = mergeLiveSnapshot(snapshot, fresh, isDemoActive() ? 0 : sessionUpdateRevision);
    sessionUpdateRevision = Math.max(sessionUpdateRevision, fresh.update_revision ?? 0);
  }

  const sessionRecovery = createCacheSyncQueue(async () => {
    if (!resources.active) return;
    try {
      const fresh = await liveGetSurface();
      if (!resources.active) return;
      // Full reads and events share capture order, including stop/start.
      mergeSessionRead(fresh);
    } catch (error) {
      if (resources.active) console.warn("[Live] session resync failed:", error);
      // Recovery failures are already presented by this batch. Return normally
      // so callers share its completion without adding one catch per event.
    }
  });

  function resyncSession(): Promise<void> {
    if (!resources.active) return Promise.resolve();
    // A gap queued during a failed read still gets its own following batch.
    return sessionRecovery(["live_session"]);
  }

  function listenLive<T>(name: string, handler: (event: { payload: T }) => void) {
    return resources.acquire(() => listen<T>(name, resources.guard(handler)));
  }

  const hasContent = $derived(snapshot.transcript_line_count > 0 || partialText.trim().length > 0);
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

  const visibleLines = $derived(snapshot.visible_lines);
  const hiddenLineCount = $derived(
    Math.max(0, snapshot.transcript_line_count - visibleLines.length)
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

  // Display-only timer, separate from the recording's lifecycle checks.
  function bindLiveDisplayClock() {
    const running = $derived(snapshot.active && liveSurfaceVisible);
    $effect(() => {
      if (!running) return;
      now = new Date();
      return resources.interval(() => { now = new Date(); }, 30_000);
    });
  }
  bindLiveDisplayClock();

  function bindLiveAutoGuard() {
    const running = $derived(snapshot.active);
    $effect(() => {
      if (!resources.active) return;
      if (running) {
        if (!liveAutoGuardTimer) {
          liveAutoGuardTimer = resources.interval(() => {
            checkLiveAutoLifecycle().catch((e: any) => {
              console.warn("[Live] auto lifecycle check failed:", e);
            });
          }, LIVE_AUTO_GUARD_INTERVAL_MS);
        }
      } else {
        stopLiveAutoGuardTimer();
        clearLiveAutoLifecycle();
      }
    });
  }
  bindLiveAutoGuard();

  function bindLiveTranscriptFollow() {
    const follow = new LiveTranscriptFollow(resources);
    $effect(() => {
      follow.update({
        recordingId: snapshot.active ? snapshot.session_id ?? null : null,
        lineCount: snapshot.transcript_line_count,
        target: scrollEl,
        visible: liveSurfaceVisible && !whiteboardExpanded && !summaryDetailOpen,
        listening: sttListening,
        autoFollow,
      });
    });
  }
  bindLiveTranscriptFollow();

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
    !snapshot.active && liveReady && !controlsBusy && (freeNoteSelected || !!selectedCourse),
  );
  const canStop = $derived(snapshot.active && !controlsBusy);
  const canGenerateOverallSummary = $derived(snapshot.active && snapshot.transcript_line_count > 0 && !controlsBusy);

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

  const lineCountLabel = $derived(`${snapshot.transcript_line_count}行`);
  const summaryCountLabel = $derived(
    snapshot.summaries.length > 0 ? `${snapshot.summaries.length}要約` : "要約待ち",
  );

  const liveControl = $derived.by((): LiveControlModel => {
    const saved = !snapshot.active && showSaveNotif && !!lastSaved;
    const blocked = !snapshot.active && !pageLoading && !liveReady;
    const progress = backendSaveProgress?.label ?? saveProgress;
    const thinking = !!progress;
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
      phase === "thinking" ? progress
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
      : primaryAction === "pause" || primaryAction === "resume" ? controlsBusy
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
      progressLabel: phase === "thinking" ? progress : "",
      saveSteps: phase === "thinking" ? backendSaveProgress?.steps ?? saveSteps : [],
      saveStepIndex: backendSaveProgress?.index ?? saveStepIndex,
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

  const previewRead = new LatestViewRead(resources, livePeekDaySurface, (cached) => {
    if (untrack(() => snapshot.active || showSaveNotif || busy)) return;
    if (cached.transcript_line_count > 0 || cached.summaries.length > 0) {
      snapshot = cached;
    } else if (untrack(() => snapshot.course)) {
      snapshot = emptyLiveSurfaceSnapshot();
    }
  });

  // Schedule clock updates rebuild CourseSlot objects. Use a primitive identity
  // so an unchanged course does not reread a full transcript every minute.
  const previewIdentity = $derived.by(() => {
    const course = selectedCourse;
    return course ? JSON.stringify([
      courseKey(course), course.name, now.getFullYear(), now.getMonth(), now.getDate(),
    ]) : "";
  });

  function bindLiveCoursePreview() {
    // Resume a selection skipped during a mutation, recording or saved badge.
    // Primitive gates avoid rereading on ordinary inactive snapshot updates.
    const enabled = $derived(!snapshot.active && !showSaveNotif && !busy);
    $effect(() => {
      void previewIdentity;
      if (!enabled || !resources.active) return;
      untrack(() => {
        const course = selectedCourse;
        overallSummary = "";
        summaryDetailOpen = false;
        summaryViewIndex = -1;
        if (!course) {
          if (snapshot.course) snapshot = emptyLiveSurfaceSnapshot();
          return;
        }
        void previewRead.refresh(toLiveCourse(course)).catch(() => {});
      });
      // Also invalidate when selecting free-note, disabling previews or closing.
      return () => previewRead.invalidate();
    });
  }
  bindLiveCoursePreview();

  function clearNoticeTimer() {
    if (noticeTimer) {
      noticeTimer();
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
    if (!resources.active) return;
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
      noticeTimer = resources.schedule(() => {
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

  const scheduleRead = new LatestViewRead(resources, async (_preserveSelection: boolean) => {
    const selection = selectedKey;
    return { data: await getScheduleSnapshot(), selection };
  }, ({ data, selection }, preserveSelection) => {
    applyScheduleSnapshot(data, new Date(), preserveSelection || selection !== selectedKey);
  });

  function refreshSchedule(preserveSelection = true) {
    return scheduleRead.refresh(preserveSelection);
  }

  function refreshFocusedCoursesFromClock() {
    if (!resources.active) return;
    const current = new Date();
    now = current;
    if (!scheduleData || snapshot.active) return;
    applyScheduleSnapshot(scheduleData, current, true);
  }

  const readinessRead = new LatestViewRead(resources, async () => {
    const [cfg, ready] = await Promise.all([getAiConfig(), isAiReady()]);
    return { cfg, ready };
  }, ({ cfg, ready }) => {
    aiReplyLanguage = cfg.reply_language || "ja";
    liveReady = ready;
    if (ready) {
      readinessMessage = "";
      clearReadinessNotice();
    } else {
      readinessMessage = buildReadinessMessage(cfg, ready);
      setReadinessNotice(readinessMessage);
    }
  }, (error) => {
    liveReady = false;
    readinessMessage = error instanceof Error ? error.message : String(error);
    setReadinessNotice(readinessMessage);
  });

  function refreshReadiness() { return readinessRead.refresh(); }

  async function ensureReadyToStart(): Promise<boolean> {
    // A settings event may supersede the check while it is running. Confirm a
    // current result before starting audio rather than using the earlier state.
    while (resources.active && !await refreshReadiness()) { /* read again */ }
    if (!resources.active) return false;
    if (!liveReady) {
      throw new Error(readinessMessage || (notice?.source === "readiness" ? notice.text : "AIの準備ができていません"));
    }
    return true;
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
      liveAutoGuardTimer();
      liveAutoGuardTimer = null;
    }
  }

  async function checkLiveAutoLifecycle() {
    if (!resources.active || !snapshot.active || controlsBusy || autoLifecycleBusy) return;
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
    if (!resources.active || snapshot.active || controlsBusy) return;
    busy = true;
    clearNotice();
    sttListening = false;
    sttPhase = "checking";
    setSttNotice("音声入力モデルを確認中…");
    pendingStartSessionId = null;
    let createdSessionId: string | null = null;
    const initialVersion = sessionEventVersion;
    try {
      if (!await ensureReadyToStart()) return;
      if (snapshot.active) return;
      sttPhase = "starting";
      setSttNotice("音声入力を起動中…");
      const started = await liveStartSurface(course);
      createdSessionId = started.session_id ?? null;
      // The backend recording survives view navigation. Do not start new audio
      // or publish this delayed response from a page which has already closed.
      if (!resources.active) return;
      mergeSessionRead(started);
      if (!snapshot.active || snapshot.session_id !== createdSessionId || isLiveBusy(snapshot, false)) return;
      pendingStartSessionId = createdSessionId;
      overallSummary = "";
      partialText = "";
      lastSaved = null;
      if (isDemoActive()) {
        sttListening = true;
        sttPhase = "listening";
        markLiveListeningStarted();
        clearSttNotice();
      } else {
        if (!createdSessionId) throw new Error("Live録音IDがありません");
        await invoke("stt_start_stream", { caller: "live", liveSessionId: createdSessionId });
      }
      if (resources.active && snapshot.active && snapshot.session_id === createdSessionId && !isLiveBusy(snapshot, false)) autoFollow = true;
    } catch (e: any) {
      const ownsStart = pendingStartSessionId === createdSessionId;
      if (ownsStart) pendingStartSessionId = null;
      const present = resources.active && (createdSessionId
        ? ownsStart && snapshot.active && snapshot.session_id === createdSessionId && !isLiveBusy(snapshot, false)
        : initialVersion === sessionEventVersion && !snapshot.active);
      if (present) {
        sttPhase = "idle";
        clearSttNotice();
        setMessage("error", e?.message || String(e));
      }
      if (createdSessionId && present) {
        try {
          await liveCancelSession(createdSessionId);
          if (resources.active && (!snapshot.active || snapshot.session_id === createdSessionId)) await resyncSession();
        } catch {}
      }
      if (present && resources.active && !isLiveBusy(snapshot, false)
        && (!snapshot.active || snapshot.session_id === createdSessionId)) clearLiveAutoLifecycle();
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
    if (!resources.active || !snapshot.active || controlsBusy) return;
    const sessionId = snapshot.session_id;
    if (!sessionId) return;
    busy = true;
    clearNotice();
    clearSttNotice();
    pendingStartSessionId = null;
    try {
      if (!isDemoActive()) {
        await invoke("stt_stop_stream", { caller: "live", liveSessionId: sessionId });
      }
      if (!resources.active || snapshot.session_id !== sessionId) return;
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
      if (snapshot.session_id === sessionId) setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  async function pauseLive() {
    await pauseLiveInternal(false);
  }

  async function resumeLive() {
    if (!resources.active || !snapshot.active || controlsBusy) return;
    const sessionId = snapshot.session_id;
    if (!sessionId) return;
    busy = true;
    clearNotice();
    sttListening = false;
    sttPhase = "checking";
    setSttNotice("音声入力モデルを確認中…");
    pendingStartSessionId = null;
    try {
      if (!await ensureReadyToStart()) return;
      if (snapshot.session_id !== sessionId || !snapshot.active || isLiveBusy(snapshot, false)) return;
      sttPhase = "starting";
      setSttNotice("音声入力を起動中…");
      if (isDemoActive()) {
        sttListening = true;
        sttPhase = "listening";
        markLiveListeningStarted();
        clearSttNotice();
      } else {
        await invoke("stt_start_stream", { caller: "live", liveSessionId: sessionId });
      }
      if (resources.active && snapshot.session_id === sessionId) autoFollow = true;
    } catch (e: any) {
      if (resources.active && snapshot.session_id === sessionId) {
        pendingStartSessionId = null;
        sttPhase = "idle";
        markLivePaused();
        clearSttNotice();
        setMessage("error", e?.message || String(e));
      }
    } finally {
      busy = false;
    }
  }

  async function stopLiveInternal(automated = false) {
    if (!resources.active || !snapshot.active || controlsBusy) return;
    const sessionId = snapshot.session_id;
    if (!sessionId) return;
    busy = true;
    clearNotice();
    clearSttNotice();
    pendingStartSessionId = null;
    const stopLabel = automated ? AUTO_STOP_STEP : STOP_STEP;
    // The backend reserves this recording before stopping its microphone and
    // draining the tail. Show its actual stages rather than predicting AI work
    // from a frontend snapshot captured before the decoder has finished.
    saveSteps = [];
    saveStepIndex = 0;
    saveProgress = `${stopLabel}…`;
    try {
      const saved = await liveFinishSurface(sessionId);
      if (!resources.active || (snapshot.session_id != null && snapshot.session_id !== sessionId)) return;
      sttListening = false;
      sttPhase = "idle";
      partialText = "";
      rememberSaved(saved);
      overallSummary = "";
      snapshot = saved.snapshot;
      clearLiveAutoLifecycle();
      endSave();
      if (saved.saved) {
        if (automated) setMessage("success", "20分間再開されなかったため、LIVEを自動保存しました");
        if (saved.todos_pending) {
          liveTodoPending.set(true);
          activeTab.set("todo");
        }
      } else {
        setMessage("success", automated ? "20分間再開されなかったため、LIVEを自動終了しました" : "LIVEを終了しました");
      }
    } catch (e: any) {
      if (resources.active && (snapshot.session_id == null || snapshot.session_id === sessionId)) {
        endSave();
        setMessage("error", e?.message || String(e));
      }
    } finally {
      busy = false;
    }
  }

  async function stopLive() {
    await stopLiveInternal(false);
  }

  async function generateOverallSummary() {
    if (!resources.active || !canGenerateOverallSummary) return;
    const sessionId = snapshot.session_id;
    if (!sessionId) return;
    busy = true;
    clearNotice();
    beginSave([OVERALL_STEP]);
    try {
      const generated = await liveGenerateOverallSummary(sessionId);
      if (!resources.active || snapshot.session_id !== sessionId || !snapshot.active) return;
      overallSummary = generated;
      const at = new Date();
      overallSummaryAt = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
      await resyncSession();
      if (!resources.active || snapshot.session_id !== sessionId || !snapshot.active) return;
      // Reset to auto so the freshly-added overall entry (always last) is shown.
      summaryViewIndex = -1;
      setMessage("success", "現在までの全体要約を生成しました");
    } catch (e: any) {
      if (snapshot.active && snapshot.session_id === sessionId) setMessage("error", e?.message || String(e));
    } finally {
      endSave();
      busy = false;
    }
  }

  function clearCourseData() {
    if (!selectedCourse || controlsBusy) return;
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
    if (!resources.active || !selectedCourse || snapshot.active || controlsBusy) return;
    const course = selectedCourse;
    const identity = previewIdentity;
    const displayed = snapshot;
    const savedPreview = lastSaved;
    const sessionVersion = sessionEventVersion;
    const current = () => resources.active && previewIdentity === identity
      && snapshot === displayed && !snapshot.active && lastSaved === savedPreview
      && sessionEventVersion === sessionVersion;
    busy = true;
    // A preview captured before deletion must not restore the removed cache.
    previewRead.invalidate();
    clearNotice();
    try {
      await liveClearDayCache(toLiveCourse(course));
      if (!current()) return;
      snapshot = emptyLiveSurfaceSnapshot();
      overallSummary = "";
      setMessage("success", `${course.name} のキャッシュをクリアしました`);
    } catch (e: any) {
      if (current()) setMessage("error", e?.message || String(e));
    } finally {
      busy = false;
    }
  }

  const sttStateRead = new LatestViewRead(resources, async () => {
    const sessionId = snapshot.session_id;
    const state = isDemoActive() ? idleSttStreamState() : await getSttStreamState();
    return { sessionId, state };
  }, ({ sessionId, state }) => {
    if (sessionId !== snapshot.session_id) return;
    applyLiveSttPhase(liveSttPhase(state, sessionId));
  });

  function applyLiveSttPhase(phase: "idle" | "initializing" | "listening") {
    const previousPhase = sttPhase;
    const wasListening = sttListening;
    sttPhase = phase;
    sttListening = phase === "initializing" || phase === "listening";
    if (phase === "initializing") {
      setSttNotice("マイクと音声認識を初期化中…");
    } else if (phase === "listening") {
      clearSttNotice();
      pendingStartSessionId = null;
      // Foreground reads must not reset the silence deadline on every focus.
      if (previousPhase !== "listening") markLiveListeningStarted();
    } else {
      clearSttNotice();
      if (snapshot.active) markLivePaused();
    }
    if (sttListening && !wasListening) autoFollow = true;
  }

  async function refreshLiveSttState() {
    try { await sttStateRead.refresh(); }
    catch (error) {
      if (resources.active) console.warn("[Live] STT state read failed:", error);
    }
  }

  function onLiveVisibilityChange() {
    liveWindowHidden = document.hidden;
  }

  function stopScheduleFocusTimer() {
    if (scheduleFocusTimer) {
      scheduleFocusTimer();
      scheduleFocusTimer = null;
    }
  }

  function unbindLiveSttListeners() {
    sttBindToken += 1;
    sttListenersBound = false;
    sttSubscription.clear();
  }

  async function bindLiveSttListeners() {
    if (!resources.active || sttListenersBound) return;
    const token = ++sttBindToken;
    sttListenersBound = true;
    try {
      const bound = await sttSubscription.replace((current, registrationScope) => {
        function subscribe<T>(name: string, handler: (event: { payload: T }) => void) {
          return (group: ResourceScope) => listen<T>(name, group.guard((event) => {
            if (current()) handler(event);
          }));
        }
        return acquireResourceGroup(registrationScope, [
          subscribe<{ text: string; caller: string; seq?: number; live_session_id?: string }>("stt-partial", (event) => {
            if (!isCurrentLiveSttEvent(snapshot, event.payload)) return;
            const seq = event.payload.seq ?? 0;
            if (seq > 0 && seq < lastPartialSeq) return;
            if (seq > 0) lastPartialSeq = seq;
            partialText = event.payload.text || "";
          }),
          subscribe<{ text: string; caller: string; seq?: number; live_session_id?: string }>("stt-final", (event) => {
            if (!isCurrentLiveSttEvent(snapshot, event.payload)) return;
            if (!snapshot.active) return;
            const seq = event.payload.seq ?? 0;
            // An older final can finish after a newer partial. Keep the live line,
            // but still commit the finished sentence to the transcript.
            if (seq === 0 || seq >= lastPartialSeq) {
              if (seq > 0) lastPartialSeq = seq;
              partialText = "";
            }
            // Rust already committed this final. This event only clears the partial;
            // live-transcript-appended carries the ordered UI delta.
          }),
          subscribe<{ state: string; caller: string; live_session_id?: string }>("stt-state", (event) => {
            if (!isCurrentLiveSttEvent(snapshot, event.payload)) return;
            sttStateRead.invalidate();
            applyLiveSttPhase(event.payload.state === "initializing" || event.payload.state === "listening"
              ? event.payload.state : "idle");
          }),
          subscribe<{ message: string; caller: string; live_session_id?: string }>("stt-error", (event) => {
            if (!isCurrentLiveSttEvent(snapshot, event.payload)) return;
            sttStateRead.invalidate();
            const wasStarting = sttPhase === "starting" || sttPhase === "initializing";
            sttListening = false;
            sttPhase = "idle";
            clearSttNotice();
            if (snapshot.active) markLivePaused();
            setMessage("error", event.payload.message);
            if (wasStarting && pendingStartSessionId === event.payload.live_session_id) {
              pendingStartSessionId = null;
              const failedSessionId = event.payload.live_session_id;
              if (!failedSessionId) return;
              void (async () => {
                try {
                  await liveCancelSession(failedSessionId);
                  if (!resources.active || (snapshot.active && snapshot.session_id !== failedSessionId)) return;
                  await resyncSession();
                  if (resources.active && !snapshot.active) partialText = "";
                } catch {}
              })();
            }
          }),
          subscribe<{ message: string; caller: string; live_session_id?: string }>("stt-info", (event) => {
            if (!isCurrentLiveSttEvent(snapshot, event.payload)) return;
            setMessage("success", event.payload.message);
          }),
        ]);
      });
      if (token === sttBindToken && !bound) sttListenersBound = false;
    } catch (error) {
      if (resources.active && token === sttBindToken) {
        sttListenersBound = false;
        console.warn("[Live] STT listener bind failed:", error);
      }
    }
  }

  function applyLiveSurfacePolicy(onLive: boolean, hidden: boolean, sessionActive: boolean) {
    if (!resources.active) return;
    const visible = onLive && !hidden;
    if (!visible) stopScheduleFocusTimer();
    else if (!scheduleFocusTimer) {
      scheduleFocusTimer = resources.interval(refreshFocusedCoursesFromClock, 60_000);
    }
    if (!onLive && hidden && !sessionActive) unbindLiveSttListeners();
    else void bindLiveSttListeners();
    if (visible && !liveSurfaceWasVisible) {
      void refreshLiveSttState();
      refreshFocusedCoursesFromClock();
    }
    liveSurfaceWasVisible = visible;
  }

  $effect(() => {
    const onLive = $activeTab === "live";
    const hidden = liveWindowHidden;
    const sessionActive = snapshot.active;
    if (!liveMounted) return;
    applyLiveSurfacePolicy(onLive, hidden, sessionActive);
  });

  async function initializeLive() {
    resources.own(onCacheUpdate<ScheduleResponse>("schedule_data", resources.guard((fresh) => {
      scheduleRead.invalidate();
      applyScheduleSnapshot(fresh, new Date(), true);
    })));
    try {
      // Register in parallel before recovery and slow schedule/AI reads.
      // Owned registrations which finish after teardown release immediately.
      const win = getCurrentWindow();
      const registrations = await Promise.allSettled([
        listenLive<LiveSurfaceSaveResult>("live-surface-saved", (event) => {
          rememberSaved(event.payload);
        }),
        listenLive<CompactLiveSurfaceSaveResult>("live-surface-compact-saved", (event) => {
          try {
            rememberSaved(expandLiveSurfaceSave(event.payload));
          } catch (error) {
            console.warn("[Live] saved whiteboard transport failed:", error);
            void resyncSession();
          }
        }),
        listenLive<LiveFinishProgress>("live-finish-progress", (event) => {
          snapshot = applyLiveFinishProgress(snapshot, event.payload);
        }),
        listenLive<LiveSessionNotification>("live-session-updated", (event) => {
          if (isDemoActive()) return;
          const result = applyLiveSessionNotification(snapshot, event.payload, sessionUpdateRevision);
          if (result.snapshot === snapshot && !result.needsResync) return;
          sessionEventVersion += 1;
          snapshot = result.snapshot;
          sessionUpdateRevision = Math.max(sessionUpdateRevision, snapshot.update_revision ?? 0);
          if (result.needsResync) void resyncSession();
        }),
        listenLive<LiveTranscriptUpdate>("live-transcript-appended", (event) => {
          const result = applyTranscriptDelta(snapshot, event.payload);
          snapshot = result.snapshot;
          if (result.needsResync) void resyncSession();
          else if (snapshot.session_id === event.payload.session_id) markEffectiveSpeech();
        }),
        listenLive<{ message: string; session_id: string }>("live-summary-error", (event) => {
          if (!isCurrentLiveSessionEvent(snapshot, event.payload)) return;
          // A scheduled AI summary failed. The backend has already cleared the
          // "生成中" flag; surface the reason (incl. any provider error code) so
          // the user knows it stalled instead of silently waiting for the next tick.
          const detail = (event.payload.message || "").trim();
          setNotice("error", detail ? `AI要約に失敗しました：${detail}` : "AI要約に失敗しました。次の区間で再試行します。", {
            source: "general",
            autoClearMs: 8000,
          });
        }),
        listenLive("ai-config-changed", () => {
          void refreshReadiness().catch((error) => {
            if (resources.active) console.warn("[Live] readiness check failed:", error);
          });
        }),
        resources.acquire(() => win.listen("tauri://blur", resources.guard(() => {
          void openSubtitleOverlay().catch(() => {});
        }))),
        resources.acquire(() => win.listen("tauri://focus", resources.guard(() => {
          void refreshSchedule(true).catch(() => {});
          void closeSubtitleOverlay().catch(() => {});
        }))),
        bindLiveSttListeners(),
      ]);
      if (!resources.active) return;
      for (const registration of registrations) {
        if (registration.status === "rejected") console.warn("[Live] listener registration failed:", registration.reason);
      }
      await resyncSession();
      if (!resources.active) return;
      await Promise.all([refreshSchedule(false), refreshReadiness()]);
      if (!resources.active) return;
      await refreshLiveSttState();
    } catch (error) {
      if (resources.active) setMessage("error", error instanceof Error ? error.message : String(error));
    } finally {
      if (resources.active) pageLoading = false;
    }
    if (!resources.active) return;
    void closeSubtitleOverlay().catch(() => {});
    liveMounted = true;
    applyLiveSurfacePolicy(get(activeTab) === "live", liveWindowHidden, snapshot.active);
  }

  onMount(() => {
    document.addEventListener("visibilitychange", onLiveVisibilityChange);
    resources.own(() => document.removeEventListener("visibilitychange", onLiveVisibilityChange));
    void initializeLive().catch((error) => {
      if (resources.active) console.warn("[Live] initialization failed:", error);
    });
  });

  onDestroy(() => {
    resources.dispose();
    stopLiveAutoGuardTimer();
    saveNotifTimer?.();
    saveNotifTimer = null;
    clearNoticeTimer();
    liveMounted = false;
    unbindLiveSttListeners();
    stopScheduleFocusTimer();
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
    busy={controlsBusy}
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
    />



    <div class="scroll-spacer-bottom"></div>
  </div>

  <LiveScrollToBottomButton visible={showScrollBtn && hasContent} onScrollToBottom={scrollToBottom} />

  <LiveRightRail
    visible={liveSurfaceVisible && !whiteboardExpanded && !summaryDetailOpen}
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
