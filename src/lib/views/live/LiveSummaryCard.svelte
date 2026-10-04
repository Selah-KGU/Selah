<script lang="ts">
  import { untrack } from "svelte";
  import { splitSummaryHeadlines } from "./liveMarkdown";

  // Entry card for the stage summary, in the right rail above the whiteboard
  // entry. Bulleted headlines are shown as a stacked deck that AUTO-cycles (no
  // manual switcher); tapping the card opens the full detail sub-page. Neutral
  // styling — no accent tint.
  type SummaryEntry = { range_label: string; body: string; isOverall: boolean };

  interface Props {
    entries: SummaryEntry[];
    activeIdx: number;
    /** Number of accumulated segment summaries (chunk entries, excludes 全体). */
    segmentCount: number;
    renderMd: (text: string) => string;
    onOpenDetail: () => void;
  }

  let { entries, activeIdx, segmentCount, renderMd, onOpenDetail }: Props = $props();

  const chunk = $derived(entries[activeIdx]);
  const points = $derived(chunk ? splitSummaryHeadlines(chunk.body) : []);
  // Stable fingerprint so the deck only resets when the point SET changes —
  // not on every transcript tick, which re-derives an equal `points` array.
  const pointsKey = $derived(`${activeIdx}::${points.join("")}`);

  let pointIdx = $state(0);
  $effect(() => {
    pointsKey;
    untrack(() => {
      pointIdx = 0;
    });
  });

  // Auto-advance: the deck rotates on its own, front card moving to the back.
  const TICK_MS = 4500;
  $effect(() => {
    const total = points.length;
    if (total <= 1) return;
    const id = setInterval(() => {
      untrack(() => {
        pointIdx = (pointIdx + 1) % total;
      });
    }, TICK_MS);
    return () => clearInterval(id);
  });

  // Same stacked math as the term deck: 0 = front, 1/2 = peeking behind.
  function stackOffset(i: number): number {
    const total = points.length;
    return total <= 0 ? 0 : (i - pointIdx + total) % total;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onOpenDetail();
  }
</script>

{#if entries.length > 0 && chunk}
  <button
    type="button"
    class="summary-stack"
    class:multi={points.length > 1}
    onclick={onOpenDetail}
    onkeydown={handleKeydown}
    aria-label="阶段摘要を開く"
  >
    {#if points.length === 0}
      <div class="summary-pt-card front">
        <div class="summary-pt-text empty">要点をまとめています…</div>
        <div class="summary-pt-foot">
          <span class="summary-time">{chunk.range_label}</span>
          {#if segmentCount > 0}<span class="segment-count">{segmentCount}区間</span>{/if}
        </div>
      </div>
    {:else}
      {#each points as pt, i (i)}
        {@const offset = stackOffset(i)}
        {@const visible = offset >= 0 && offset <= 2}
        <div
          class="summary-pt-card"
          class:front={offset === 0}
          class:peek={offset > 0}
          style="
            transform: translateY({offset * 10}px) scale({1 - offset * 0.04});
            opacity: {offset === 0 ? 1 : 0.66 - (offset - 1) * 0.22};
            z-index: {100 - offset};
            visibility: {visible ? 'visible' : 'hidden'};
            {visible ? '' : 'transition: none;'}
          "
          aria-hidden={offset !== 0}
        >
          <div class="summary-pt-text md">{@html renderMd(pt)}</div>
          {#if offset === 0}
            <div class="summary-pt-foot">
              <span class="summary-time">{chunk.range_label}</span>
              {#if segmentCount > 0}<span class="segment-count">{segmentCount}区間</span>{/if}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </button>
{/if}

<style>
  /* The deck container is the clickable surface; the cards inside are visual
     layers (pointer-events off) so any tap opens the detail page. */
  .summary-stack {
    position: relative;
    display: block;
    width: 100%;
    padding: 0;
    border: none;
    background: transparent;
    text-align: left;
    font-family: inherit;
    cursor: pointer;
    /* The rail sets pointer-events:none and re-enables it only for its OWN
       direct children; as a child component our root isn't matched by that
       rule, so opt back in explicitly or the card can't be clicked. */
    pointer-events: auto;
    transition: transform 0.18s cubic-bezier(0.22, 1, 0.36, 1);
    animation: summary-stack-in 0.32s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .summary-stack:hover {
    transform: translateY(-1px);
  }
  .summary-stack:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 13px;
  }
  /* Peeking cards extend ~26px below the front; reserve room so they aren't
     clipped by the whiteboard entry sitting below in the rail. */
  .summary-stack.multi {
    margin-bottom: 14px;
  }

  .summary-pt-card {
    position: absolute;
    inset: 0;
    pointer-events: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 11px 13px 10px;
    border-radius: 13px;
    border: 0.5px solid var(--glass-border);
    background: color-mix(in srgb, var(--glass-bg) 40%, var(--bg-primary));
    box-shadow: var(--shadow-glass);
    overflow: hidden;
    transform-origin: 50% 0;
    transition: transform 0.45s cubic-bezier(0.22, 1, 0.36, 1),
                opacity 0.35s ease, border-color 0.18s ease;
  }
  .summary-pt-card.front {
    position: relative;
    inset: auto;
    min-height: 54px;
    background: color-mix(in srgb, var(--glass-bg) 58%, var(--bg-primary));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    box-shadow: var(--shadow-glass), 0 6px 18px rgba(0, 0, 0, 0.08);
  }
  .summary-stack:hover .summary-pt-card.front {
    border-color: color-mix(in srgb, var(--text-primary) 16%, var(--glass-border));
  }

  .summary-pt-text {
    min-width: 0;
    /* Editorial serif for the knowledge text, set apart from the sans UI. */
    font-family: "Hiragino Mincho ProN", "Yu Mincho", "YuMincho",
      "Songti SC", serif;
    font-size: 14.5px;
    font-weight: 600;
    line-height: 1.5;
    color: var(--text-primary);
    word-break: break-word;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .summary-pt-text.empty {
    font-family: inherit;
    font-weight: 500;
    font-size: 13px;
    color: var(--text-tertiary);
  }
  .summary-pt-text :global(p) { margin: 0; }
  .summary-pt-text :global(strong) { font-weight: 800; }
  .summary-pt-text :global(code) {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    padding: 1px 4px;
    border-radius: 4px;
    font-size: 0.88em;
  }
  .summary-pt-text :global(a) { color: var(--accent); text-decoration: none; }

  .summary-pt-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .summary-time {
    font-size: 10.5px;
    font-weight: 600;
    color: var(--text-tertiary);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .segment-count {
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-tertiary) 14%, transparent);
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 700;
    line-height: 1.5;
    white-space: nowrap;
  }

  @keyframes summary-stack-in {
    from { opacity: 0; transform: translateY(10px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }
</style>
