<script lang="ts">
  import type { LiveSessionSnapshot } from "../../api";
  import type { CourseSlot } from "../../schedule";
  import type { SttPhase } from "./liveTypes";

  interface Props {
    snapshot: LiveSessionSnapshot;
    sttPhase: SttPhase;
    liveBadgeLabel: string;
    remainingLabel: string;
    saveProgress: string;
    renderedCourseOptions: CourseSlot[];
    selectedKey: string;
    pageLoading: boolean;
    hasContent: boolean;
    selectedCourse: CourseSlot | null;
    busy: boolean;
    canStart: boolean;
    canStartFreeNote: boolean;
    canStop: boolean;
    sttListening: boolean;
    sttBooting: boolean;
    confirmClear: boolean;
    courseKey: (course: CourseSlot) => string;
    courseLabel: (course: CourseSlot) => string;
    onStartLive: () => void;
    onStartFreeNote: () => void;
    onClearCourseData: () => void;
    onCancelClear: () => void;
    onConfirmClear: () => void;
    onStopLive: () => void;
    onPauseLive: () => void;
    onResumeLive: () => void;
  }

  let {
    snapshot,
    sttPhase,
    liveBadgeLabel,
    remainingLabel,
    saveProgress,
    renderedCourseOptions,
    selectedKey = $bindable(),
    pageLoading,
    hasContent,
    selectedCourse,
    busy,
    canStart,
    canStartFreeNote,
    canStop,
    sttListening,
    sttBooting,
    confirmClear,
    courseKey,
    courseLabel,
    onStartLive,
    onStartFreeNote,
    onClearCourseData,
    onCancelClear,
    onConfirmClear,
    onStopLive,
    onPauseLive,
    onResumeLive,
  }: Props = $props();
</script>

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
      {#if saveProgress}
        <span class="capsule-progress">{saveProgress}</span>
      {/if}
    {/if}

    <div class="capsule-actions">
      {#if !snapshot.active}
        <button class="capsule-act primary" onclick={onStartLive} disabled={!canStart}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
          開始
        </button>
        <button class="capsule-act ghost note" onclick={onStartFreeNote} disabled={!canStartFreeNote}>
          自由ノート
        </button>
        {#if hasContent && selectedCourse}
          <div class="clear-wrap">
            <button class="capsule-act ghost danger" onclick={onClearCourseData} disabled={busy}>
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 011-1h4a1 1 0 011 1v2"/></svg>
              クリア
            </button>
            {#if confirmClear}
              <div class="clear-tooltip" role="tooltip">
                <span class="clear-tooltip-msg">本当に削除？</span>
                <button class="clear-tip-btn cancel" onclick={onCancelClear}>いいえ</button>
                <button class="clear-tip-btn danger" onclick={onConfirmClear}>削除</button>
              </div>
            {/if}
          </div>
        {/if}
      {:else}
        <button class="capsule-act stop" onclick={onStopLive} disabled={!canStop}>
          <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="4" width="16" height="16" rx="2"/></svg>
          保存
        </button>
        {#if sttListening || sttBooting}
          <button class="capsule-act ghost" onclick={onPauseLive} disabled={busy}>一時停止</button>
        {:else}
          <button class="capsule-act ghost note" onclick={onResumeLive} disabled={busy}>再開</button>
        {/if}
      {/if}
    </div>
  </div>
</header>

<style>
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

  @media (max-width: 600px) {
    .top-capsule {
      left: 12px;
      right: 12px;
      transform: none;
      max-width: none;
    }
    .capsule-inner { width: 100%; }
    .capsule-course { max-width: 120px; }
    .capsule-select { flex: 1; max-width: none; }
  }

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

  @keyframes capsule-in {
    from { opacity: 0; transform: scale(0.95); }
    to { opacity: 1; transform: scale(1); }
  }
</style>
