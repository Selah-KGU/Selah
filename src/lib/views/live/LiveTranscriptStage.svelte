<script lang="ts">
  import TodoDraftCard from "../../TodoDraftCard.svelte";
  import type { LiveSaveResult, LiveSessionSnapshot } from "../../api";
  import type { LiveTodoDraft } from "./liveTypes";

  type TranscriptLine = LiveSessionSnapshot["transcript_lines"][number];

  interface Props {
    pageLoading: boolean;
    hasContent: boolean;
    snapshot: LiveSessionSnapshot;
    partialText: string;
    saveProgress: string;
    sttBooting: boolean;
    sttBootMessage: string;
    todoDrafts: LiveTodoDraft[];
    todoDraftsWithDeadlineCount: number;
    todoDraftSaving: boolean;
    lastSaved: LiveSaveResult | null;
    showSaveNotif: boolean;
    visibleLines: TranscriptLine[];
    hiddenLineCount: number;
    renderMd: (text: string) => string;
    extractOverallSummary: (markdown: string) => string;
    onToggleTodoDraft: (index: number) => void;
    onCloseTodoDrafts: () => void;
    onConfirmTodoDrafts: () => void;
  }

  let {
    pageLoading,
    hasContent,
    snapshot,
    partialText,
    saveProgress,
    sttBooting,
    sttBootMessage,
    todoDrafts,
    todoDraftsWithDeadlineCount,
    todoDraftSaving,
    lastSaved,
    showSaveNotif,
    visibleLines,
    hiddenLineCount,
    renderMd,
    extractOverallSummary,
    onToggleTodoDraft,
    onCloseTodoDrafts,
    onConfirmTodoDrafts,
  }: Props = $props();
</script>

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
          {:else if todoDrafts.length > 0}
            <div class="save-capsule done">
              <svg class="save-capsule-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="url(#notif-grad)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <defs><linearGradient id="notif-grad" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#c480e8"/><stop offset="100%" stop-color="#6bacf0"/></linearGradient></defs>
                <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/>
              </svg>
              <span class="save-capsule-text">保存完了・TODO候補あり</span>
            </div>
            {#if lastSaved}
              <div class="save-summary md">{@html renderMd(extractOverallSummary(lastSaved.markdown))}</div>
            {/if}
            <TodoDraftCard
              title="LiveからTODO候補を追加"
              subtitle={`${todoDrafts.length}件中 ${todoDraftsWithDeadlineCount}件にDDLあり。必要なものだけ選んで追加できます。`}
              drafts={todoDrafts}
              saving={todoDraftSaving}
              inline
              onToggle={onToggleTodoDraft}
              onClose={onCloseTodoDrafts}
              onConfirm={onConfirmTodoDrafts}
            />
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

<style>
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

  .lyric-line.past {
    opacity: 0.38;
    transform: scale(0.97);
  }
  .lyric-line.past:hover {
    opacity: 0.65;
    background: color-mix(in srgb, var(--text-primary) 3%, transparent);
  }

  .lyric-line.active {
    opacity: 1;
    transform: scale(1);
  }
  .lyric-line.active .lyric-text {
    font-size: 21px;
    font-weight: 600;
    color: var(--text-primary);
  }
  .lyric-line.active .lyric-time {
    color: var(--accent);
    font-weight: 600;
  }

  .lyric-line.partial {
    opacity: 0.75;
    animation: none;
  }
  .lyric-line.partial .lyric-text {
    color: var(--accent);
    font-style: italic;
  }

  .lyric-time {
    flex-shrink: 0;
    width: 42px;
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    text-align: right;
    transition: color 0.3s;
  }

  .lyric-text {
    flex: 1;
    font-size: 16px;
    line-height: 1.6;
    color: var(--text-secondary);
    word-break: break-word;
    transition: font-size 0.3s, font-weight 0.3s, color 0.3s;
  }

  .typing-cursor {
    display: inline-block;
    width: 2px;
    height: 1em;
    background: var(--accent);
    margin-left: 3px;
    vertical-align: middle;
    animation: cursor-blink 1s steps(2) infinite;
  }
  @keyframes cursor-blink {
    0%, 45% { opacity: 1; }
    46%, 100% { opacity: 0; }
  }

  .lyrics-hidden-hint {
    align-self: center;
    margin: 2px 0 8px;
    padding: 4px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-primary) 5%, transparent);
    color: var(--text-tertiary);
    font-size: 11px;
  }

  .lyrics-count {
    position: sticky;
    bottom: 10px;
    align-self: flex-end;
    margin-right: 8px;
    font-size: 11px;
    color: var(--text-tertiary);
    background: var(--glass-bg, rgba(255,255,255,0.6));
    backdrop-filter: blur(10px);
    padding: 3px 8px;
    border-radius: 999px;
  }

  .save-capsule {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 600;
  }
  .save-capsule.saving {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
  }
  .save-capsule.done {
    background: color-mix(in srgb, var(--green) 10%, transparent);
    color: var(--green);
  }
  .save-capsule-icon {
    flex: 0 0 auto;
  }
  .save-capsule-text {
    color: inherit;
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
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, currentColor 22%, transparent);
    border-top-color: currentColor;
    animation: spin 0.8s linear infinite;
  }
  .save-summary {
    max-width: min(620px, 90%);
    padding: 12px 16px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--bg-card) 86%, transparent);
    border: 0.5px solid var(--glass-border);
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.6;
    text-align: left;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
