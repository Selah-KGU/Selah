<script lang="ts">
  import type { LiveTranscriptLine } from "../../api";

  import type { LiveSavedPreview, LiveSurfaceSnapshot } from "./liveTranscript";

  interface Props {
    pageLoading: boolean;
    hasContent: boolean;
    snapshot: Pick<LiveSurfaceSnapshot, "active" | "transcript_line_count">;
    partialText: string;
    lastSaved: LiveSavedPreview | null;
    showSaveNotif: boolean;
    visibleLines: LiveTranscriptLine[];
    hiddenLineCount: number;
    renderMd: (text: string) => string;
  }

  let {
    pageLoading,
    hasContent,
    snapshot,
    partialText,
    lastSaved,
    showSaveNotif,
    visibleLines,
    hiddenLineCount,
    renderMd,
  }: Props = $props();
</script>

<section class="lyrics-stage">
  {#if pageLoading}
    <div class="lyrics-empty">読み込み中…</div>
  {:else if !hasContent}
    <div class="lyrics-empty">
      {#if snapshot.active}
        <div class="waiting-vis">
          <span class="vis-bar"></span>
          <span class="vis-bar"></span>
          <span class="vis-bar"></span>
          <span class="vis-bar"></span>
          <span class="vis-bar"></span>
        </div>
        <span>音声待機中…</span>
      {:else if showSaveNotif && lastSaved}
        <!-- Saved-note preview (content). The 保存完了 *status* lives only in the
             top island now — not repeated here. -->
        <div class="empty-hero">
          <div class="save-summary md">{@html renderMd(lastSaved.summary_markdown)}</div>
        </div>
      {:else}
        <div class="empty-hero">
          <svg width="52" height="52" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round" opacity="0.18">
            <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/>
            <path d="M19 10v2a7 7 0 0 1-14 0v-2"/>
            <line x1="12" y1="19" x2="12" y2="23"/>
            <line x1="8" y1="23" x2="16" y2="23"/>
          </svg>
          <p>授業または自由ノートを開始すると<br/>リアルタイム文字起こしがここに表示されます</p>
        </div>
      {/if}
    </div>
  {:else}
    <div class="lyrics-track">
      {#if hiddenLineCount > 0}
        <div class="lyrics-hidden-hint">前{hiddenLineCount}行は保存済み（表示省略）</div>
      {/if}
      {#each visibleLines as line, i (hiddenLineCount + i)}
        {@const isLast = i === visibleLines.length - 1 && !partialText.trim()}
        <div class="lyric-line" class:past={!isLast} class:active={isLast}>
          <span class="lyric-time">{line.at}</span>
          <span class="lyric-text">{line.text}</span>
        </div>
      {/each}
      {#if partialText.trim()}
        <div class="lyric-line active partial">
          <span class="lyric-time">now</span>
          <span class="lyric-text">{partialText.trim()}</span>
        </div>
      {/if}
    </div>
    <div class="lyrics-count">{snapshot.transcript_line_count}行</div>
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
      background 0.3s ease;
    animation: lyric-enter 0.45s cubic-bezier(0.22, 1, 0.36, 1) both;
  }

  @keyframes lyric-enter {
    from {
      opacity: 0;
      transform: translateY(14px) scale(0.97);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
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
    animation: none;
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
    align-self: flex-start;
    margin-left: 8px;
    font-size: 11px;
    color: var(--text-tertiary);
    background: var(--glass-bg, rgba(255,255,255,0.6));
    backdrop-filter: blur(10px);
    padding: 3px 8px;
    border-radius: 999px;
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
</style>
