<script lang="ts">
  import type { LiveTermExplanation } from "../../api";
  import type { WhiteboardLayoutResult } from "../../whiteboardLayout";
  import type { TermFloatLabels } from "./liveTypes";
  import LiveSummaryCard from "./LiveSummaryCard.svelte";
  import { visibleDeckCards } from "./liveDeck";

  type SummaryEntry = { range_label: string; body: string; isOverall: boolean };

  interface Props {
    summaryEntries: SummaryEntry[];
    /** Index of the active SEGMENT (into the non-overall entries). */
    activeSummaryIdx: number;
    summarySegmentCount: number;
    visible?: boolean;
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
    onOpenWhiteboard: () => void;
    onSelectTermCard: (index: number) => void;
    onTermCardPrev: () => void;
    onTermCardNext: () => void;
  }

  let {
    summaryEntries,
    activeSummaryIdx,
    summarySegmentCount,
    visible = true,
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
  const visibleTerms = $derived(visibleDeckCards(activeSummaryTerms, termCardIdx));
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
        {visible}
        {renderMd}
        onOpenDetail={onOpenSummaryDetail}
      />
    {/if}
    {#if activeSummaryTerms.length > 0}
      <aside class="term-stack" class:multi={activeSummaryTerms.length > 1} aria-label={termFloatLabels.title}>
        {#each visibleTerms as card (card.index + "-" + card.item.term)}
          {@const item = card.item}
          {@const offset = card.offset}
          <button
            type="button"
            class="term-card"
            class:active={offset === 0}
            class:peek={offset > 0}
            style="
              transform: translateY({offset * 10}px) scale({1 - offset * 0.04});
              opacity: {offset === 0 ? 1 : 0.72 - (offset - 1) * 0.22};
              z-index: {100 - offset};
            "
            onclick={() => (offset === 0 ? onOpenSummaryDetail() : onSelectTermCard(card.index))}
            aria-hidden={false}
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

<style>
  .right-rail {
    position: absolute;
    right: 16px;
    bottom: 16px;
    z-index: 33;
    width: min(280px, calc(100% - 32px));
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
    pointer-events: none;
  }
  .right-rail > * {
    pointer-events: auto;
  }

  /* Integrated control strip above the cards: segment ◀▶ nav, 全体 quick-entry,
     and the next-summary countdown / generating status. */
  /* No shared background — each control is its own floating chip. */
  .seg-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    animation: term-stack-in 0.32s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .seg-bar.status-only {
    width: max-content;
    max-width: 100%;
    align-self: flex-end;
  }
  .seg-nav {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .seg-nav-btn {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0.5px solid var(--glass-border);
    border-radius: 999px;
    background: color-mix(in srgb, var(--glass-bg) 58%, var(--bg-primary));
    box-shadow: var(--shadow-glass);
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    color: var(--text-secondary);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
  }
  .seg-nav-btn:hover:not(:disabled) {
    color: var(--text-primary);
    border-color: color-mix(in srgb, var(--accent) 30%, var(--glass-border));
  }
  .seg-nav-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .seg-overall {
    flex: 0 0 auto;
    padding: 6px 12px;
    border-radius: 999px;
    border: 0.5px solid color-mix(in srgb, var(--accent) 30%, var(--glass-border));
    background: color-mix(in srgb, var(--accent) 12%, var(--bg-input));
    box-shadow: var(--shadow-glass);
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    color: color-mix(in srgb, var(--accent) 82%, var(--text-primary));
    font-family: inherit;
    font-size: 11.5px;
    font-weight: 800;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }
  .seg-overall:hover {
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-input));
    color: color-mix(in srgb, var(--accent) 92%, var(--text-primary));
  }
  .seg-status {
    flex: 0 1 auto;
    min-width: 0;
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 11px;
    border-radius: 999px;
    border: 0.5px solid var(--glass-border);
    background: color-mix(in srgb, var(--glass-bg) 58%, var(--bg-primary));
    box-shadow: var(--shadow-glass);
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    color: var(--text-tertiary);
    font-size: 10.5px;
    font-weight: 700;
  }
  .seg-status > span:last-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg-status.generating {
    color: color-mix(in srgb, var(--accent) 80%, var(--text-primary));
  }
  .mini-spinner {
    flex: 0 0 auto;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.6px solid color-mix(in srgb, currentColor 26%, transparent);
    border-top-color: currentColor;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    .mini-spinner { animation: none; }
  }

  .board-stack {
    animation: term-stack-in 0.32s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .board-preview-card {
    width: 100%;
    display: block;
    padding: 0;
    text-align: left;
    font-family: inherit;
    border-radius: 13px;
    border: 0.5px solid var(--glass-border);
    background: color-mix(in srgb, var(--glass-bg) 58%, var(--bg-primary));
    box-shadow: var(--shadow-glass), 0 6px 18px rgba(0, 0, 0, 0.08);
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    cursor: pointer;
    overflow: hidden;
    transition: transform 0.18s cubic-bezier(0.22, 1, 0.36, 1),
                box-shadow 0.18s ease, border-color 0.18s ease;
  }
  .board-preview-card:hover {
    transform: translateY(-1px);
    box-shadow: var(--shadow-glass), 0 10px 24px rgba(0, 0, 0, 0.1);
    border-color: color-mix(in srgb, var(--text-primary) 16%, var(--glass-border));
  }
  .board-preview-card:active {
    transform: translateY(0);
  }
  .board-preview-card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .board-preview-canvas {
    position: relative;
    width: 100%;
    height: 156px;
    background:
      linear-gradient(color-mix(in srgb, var(--text-tertiary) 7%, transparent) 1px, transparent 1px),
      linear-gradient(90deg, color-mix(in srgb, var(--text-tertiary) 7%, transparent) 1px, transparent 1px),
      color-mix(in srgb, var(--bg-secondary) 60%, transparent);
    background-size: 14px 14px;
    overflow: hidden;
  }
  .board-preview-links {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    color: color-mix(in srgb, var(--blue) 48%, var(--text-tertiary));
    overflow: visible;
  }
  .board-preview-links line {
    stroke: var(--edge-color, currentColor);
    /* Non-scaling: stroke stays 1px no matter how the px viewBox is squished
       into the small preview card. */
    vector-effect: non-scaling-stroke;
    stroke-width: 1;
    stroke-linecap: round;
    opacity: 0.5;
  }
  .board-preview-links line.trunk {
    opacity: 0.78;
  }
  .board-preview-links line.edge-kind-core,
  .board-preview-links line.edge-kind-support {
    --edge-color: color-mix(in srgb, var(--text-tertiary) 72%, var(--text-secondary));
  }
  .board-preview-links line.edge-kind-result {
    --edge-color: color-mix(in srgb, #34c759 62%, var(--text-tertiary));
  }
  .board-preview-links line.edge-kind-question {
    --edge-color: color-mix(in srgb, var(--orange, #e67700) 64%, var(--text-tertiary));
  }
  .board-preview-links line.edge-source-external {
    stroke-dasharray: 3 2;
  }
  .board-preview-node {
    position: absolute;
    transform: translate(-50%, -50%);
    z-index: 2;
    max-width: 66px;
    padding: 2px 6px;
    border-radius: 6px;
    border: 0.5px solid color-mix(in srgb, var(--accent) 22%, var(--glass-border));
    background: color-mix(in srgb, var(--bg-primary) 96%, transparent);
    color: var(--text-primary);
    font-size: 9.5px;
    font-weight: 700;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
  }
  /* Denser boards → smaller preview nodes so more of them stay distinguishable. */
  .board-preview-card.dense .board-preview-node {
    max-width: 54px;
    padding: 1.5px 5px;
    font-size: 8.5px;
  }
  .board-preview-card.very-dense .board-preview-node {
    max-width: 44px;
    padding: 1px 4px;
    font-size: 8px;
  }
  .board-preview-node.kind-core {
    background: color-mix(in srgb, var(--blue) 16%, var(--bg-primary));
    border-color: color-mix(in srgb, var(--blue) 36%, var(--glass-border));
  }
  .board-preview-node.kind-result {
    background: color-mix(in srgb, #34c759 14%, var(--bg-primary));
    border-color: color-mix(in srgb, #34c759 34%, var(--glass-border));
  }
  .board-preview-node.kind-question {
    background: color-mix(in srgb, var(--orange, #e67700) 14%, var(--bg-primary));
    border-color: color-mix(in srgb, var(--orange, #e67700) 32%, var(--glass-border));
  }
  .board-preview-node.external {
    border-style: dashed;
    border-color: color-mix(in srgb, var(--accent) 38%, var(--glass-border));
  }

  .term-stack {
    position: relative;
    width: 100%;
    padding: 0;
    background: transparent;
    border: none;
    animation: term-stack-in 0.32s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  /* Reserve space for the peeking cards (~28px below the front) so they don't
     overlap the whiteboard entry below and the gaps stay even. */
  .term-stack.multi {
    margin-bottom: 14px;
  }

  .term-card {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    text-align: left;
    font-family: inherit;
    padding: 11px 13px 10px;
    border-radius: 13px;
    border: 0.5px solid var(--glass-border);
    background: color-mix(in srgb, var(--glass-bg) 40%, var(--bg-primary));
    box-shadow: var(--shadow-glass);
    overflow: hidden;
    cursor: pointer;
    transform-origin: 50% 0;
    transition: transform 0.28s cubic-bezier(0.22, 1, 0.36, 1),
                opacity 0.22s ease;
  }
  .term-card.active {
    position: relative;
    inset: auto;
    min-height: 92px;
    cursor: pointer;
    background: color-mix(in srgb, var(--glass-bg) 58%, var(--bg-primary));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    box-shadow: var(--shadow-glass), 0 6px 18px rgba(0, 0, 0, 0.08);
  }
  .term-card.active:hover {
    border-color: color-mix(in srgb, var(--text-primary) 16%, var(--glass-border));
  }
  .term-card.peek:hover {
    background: color-mix(in srgb, var(--glass-bg) 50%, var(--bg-primary));
  }
  .term-card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .term-card-term {
    font-size: 12.5px;
    font-weight: 800;
    color: var(--text-primary);
    line-height: 1.3;
    word-break: break-word;
    padding-right: 70px;
  }

  .term-card-body {
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-secondary);
    word-break: break-word;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .term-card-meta {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-top: 2px;
  }

  .term-card-source {
    font-size: 10.5px;
    line-height: 1.4;
    color: var(--text-tertiary);
    word-break: break-word;
    border-left: 1.5px solid color-mix(in srgb, var(--accent) 28%, transparent);
    padding-left: 7px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .term-card-source.external {
    border-left-color: color-mix(in srgb, #34c759 36%, transparent);
  }
  .term-card-source span {
    display: inline-block;
    margin-right: 5px;
    font-weight: 800;
    color: color-mix(in srgb, var(--accent) 78%, var(--text-secondary));
  }
  .term-card-source.external span {
    color: color-mix(in srgb, #34c759 78%, var(--text-secondary));
  }

  .term-stack-nav {
    position: absolute;
    top: 7px;
    right: 9px;
    z-index: 110;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--bg-primary) 88%, transparent);
    border: 0.5px solid color-mix(in srgb, var(--accent) 22%, var(--glass-border));
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
  }
  .term-stack-arrow {
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: 50%;
    color: var(--accent);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease, opacity 0.15s ease;
  }
  .term-stack-arrow:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .term-stack-arrow:active:not(:disabled) {
    transform: scale(0.92);
  }
  .term-stack-arrow:disabled {
    color: var(--text-tertiary);
    cursor: default;
    opacity: 0.35;
  }
  .term-stack-counter {
    padding: 0 4px;
    color: var(--accent);
    font-size: 10.5px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    min-width: 24px;
    text-align: center;
  }

  @keyframes term-stack-in {
    from { opacity: 0; transform: translateY(10px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  @media (max-width: 700px) {
    .right-rail {
      left: 12px;
      right: 12px;
      bottom: 12px;
      width: auto;
    }
  }
</style>
