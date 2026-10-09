<!-- Frozen card markup before visible-window mounting; CSS excluded. -->
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
