<!-- Frozen card markup before visible-window mounting; CSS excluded. -->
<script lang="ts">
  import type { LiveTermExplanation } from "../../api";
  import type { WhiteboardLayoutResult } from "../../whiteboardLayout";
  import type { TermFloatLabels } from "./liveTypes";
  import LiveSummaryCard from "./LiveSummaryCard.svelte";

  type SummaryEntry = { range_label: string; body: string; isOverall: boolean };

  interface Props {
    summaryEntries: SummaryEntry[];
    /** Index of the active SEGMENT (into the non-overall entries). */
    activeSummaryIdx: number;
    summarySegmentCount: number;
    renderMd: (text: string) => string;
    onOpenSummaryDetail: () => void;
    onSelectSegment: (idx: number) => void;
    onOpenOverall: () => void;
    summarizing: boolean;
    summaryStatusLabel: string;
    previewLayout: WhiteboardLayoutResult | null;
    activeSummaryTerms: LiveTermExplanation[];
    termCardIdx: number;
    termFloatLabels: TermFloatLabels;
    termStackOffset: (index: number) => number;
    onOpenWhiteboard: () => void;
    onSelectTermCard: (index: number) => void;
    onTermCardPrev: () => void;
    onTermCardNext: () => void;
  }

  let {
    summaryEntries,
    activeSummaryIdx,
    summarySegmentCount,
    renderMd,
    onOpenSummaryDetail,
    onSelectSegment,
    onOpenOverall,
    summarizing,
    summaryStatusLabel,
    previewLayout,
    activeSummaryTerms,
    termCardIdx,
    termFloatLabels,
    termStackOffset,
    onOpenWhiteboard,
    onSelectTermCard,
    onTermCardPrev,
    onTermCardNext,
  }: Props = $props();

  // Glanceable thumbnail of the whole board: edges in the layout's pixel
  // stage, nodes positioned by percentage at a fixed readable size, stretched
  // to fill the card. It's a rough "there's a board" indicator — tap to open.
  const previewViewBox = $derived(
    previewLayout?.stage
      ? `0 0 ${previewLayout.stage.width} ${previewLayout.stage.height}`
      : "0 0 100 100",
  );

  // The overall summary is a separate quick-entry, not a card-driving segment.
  const segments = $derived(summaryEntries.filter((e) => !e.isOverall));
  const hasOverall = $derived(summaryEntries.some((e) => e.isOverall));

  const canPrevSegment = $derived(activeSummaryIdx > 0);
  const canNextSegment = $derived(activeSummaryIdx < segments.length - 1);
  function prevSegment() {
    if (canPrevSegment) onSelectSegment(activeSummaryIdx - 1);
  }
  function nextSegment() {
    if (canNextSegment) onSelectSegment(activeSummaryIdx + 1);
  }
</script>

{#if summaryEntries.length > 0 || previewLayout || activeSummaryTerms.length > 0 || summaryStatusLabel}
  <div class="right-rail">
    {#if segments.length > 0 || summaryStatusLabel}
      <div class="seg-bar" class:status-only={segments.length === 0} aria-label="区間切替">
        {#if segments.length > 0}
          <div class="seg-nav">
            <button type="button" class="seg-nav-btn" onclick={prevSegment} disabled={!canPrevSegment} aria-label="前の区間" title="前の区間">
              <svg width="9" height="9" viewBox="0 0 10 10" fill="none"><path d="M6.5 2L3 5l3.5 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
            <button type="button" class="seg-nav-btn" onclick={nextSegment} disabled={!canNextSegment} aria-label="次の区間" title="次の区間">
              <svg width="9" height="9" viewBox="0 0 10 10" fill="none"><path d="M3.5 2L7 5l-3.5 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
          </div>
        {/if}
        {#if summaryStatusLabel}
          <div class="seg-status" class:generating={summarizing} role="status">
            {#if summarizing}<span class="mini-spinner" aria-hidden="true"></span>{/if}
            <span>{summaryStatusLabel}</span>
          </div>
        {/if}
        {#if hasOverall}
          <button type="button" class="seg-overall" onclick={onOpenOverall} title="全体要約を開く">全体</button>
        {/if}
      </div>
    {/if}
    {#if summaryEntries.length > 0}
      <LiveSummaryCard
        entries={summaryEntries}
        activeIdx={activeSummaryIdx}
        segmentCount={summarySegmentCount}
        {renderMd}
        onOpenDetail={onOpenSummaryDetail}
      />
    {/if}
    {#if activeSummaryTerms.length > 0}
      <aside class="term-stack" class:multi={activeSummaryTerms.length > 1} aria-label={termFloatLabels.title}>
        {#each activeSummaryTerms as item, i (i + "-" + item.term)}
          {@const offset = termStackOffset(i)}
          {@const visible = offset >= 0 && offset <= 2}
          <button
            type="button"
            class="term-card"
            class:active={offset === 0}
            class:peek={offset > 0}
            style="
              transform: translateY({offset * 10}px) scale({1 - offset * 0.04});
              opacity: {offset === 0 ? 1 : 0.72 - (offset - 1) * 0.22};
              z-index: {100 - offset};
              pointer-events: {visible ? 'auto' : 'none'};
              visibility: {visible ? 'visible' : 'hidden'};
              {visible ? '' : 'transition: none;'}
            "
            onclick={() => (offset === 0 ? onOpenSummaryDetail() : onSelectTermCard(i))}
            aria-hidden={!visible}
            tabindex={offset === 0 ? 0 : -1}
          >
            <div class="term-card-term">{item.term}</div>
            <div class="term-card-body">{item.explanation}</div>
            {#if item.source_excerpt || item.external_source}
              <div class="term-card-meta">
                {#if item.source_excerpt}
                  <div class="term-card-source"><span>{termFloatLabels.source}</span>{item.source_excerpt}</div>
                {/if}
                {#if item.external_source}
                  <div class="term-card-source external"><span>{termFloatLabels.externalSource}</span>{item.external_source}</div>
                {/if}
              </div>
            {/if}
          </button>
        {/each}
        <div class="term-stack-nav">
          {#if activeSummaryTerms.length > 1}
            <button class="term-stack-arrow" onclick={onTermCardPrev} aria-label={termFloatLabels.previous} title={termFloatLabels.previous}>
              <svg width="9" height="9" viewBox="0 0 10 10" fill="none"><path d="M7 2L3 5l4 3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
          {/if}
          <span class="term-stack-counter">{termCardIdx + 1}/{activeSummaryTerms.length}</span>
          {#if activeSummaryTerms.length > 1}
            <button class="term-stack-arrow" onclick={onTermCardNext} aria-label={termFloatLabels.next} title={termFloatLabels.next}>
              <svg width="9" height="9" viewBox="0 0 10 10" fill="none"><path d="M3 2l4 3-4 3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
          {/if}
        </div>
      </aside>
    {/if}

    {#if previewLayout}
      <aside class="board-stack" aria-label={termFloatLabels.boardTitle}>
        <button
          type="button"
          class="board-preview-card"
          class:dense={previewLayout.nodes.length > 8}
          class:very-dense={previewLayout.nodes.length > 14}
          onclick={onOpenWhiteboard}
          aria-label={termFloatLabels.expand}
          title={termFloatLabels.expand}
        >
          <div class="board-preview-canvas">
            <svg
              class="board-preview-links"
              viewBox={previewViewBox}
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              {#each previewLayout.edges as edge (edge.id)}
                <line
                  class="edge-kind-{edge.colorKind} edge-source-{edge.colorSourceType}"
                  class:trunk={edge.trunk}
                  x1={edge.x1}
                  y1={edge.y1}
                  x2={edge.x2}
                  y2={edge.y2}
                />
              {/each}
            </svg>
            {#each previewLayout.nodes as node (node.id)}
              <span
                class="board-preview-node kind-{node.kind}"
                class:role-main={node.role === "main"}
                class:role-branch={node.role !== "main"}
                class:external={node.sourceType === "external"}
                style="left: {node.x}%; top: {node.y}%;"
              >{node.label}</span>
            {/each}
          </div>
        </button>
      </aside>
    {/if}
  </div>
{/if}
