<script lang="ts">
  import type { LiveTermExplanation } from "../../api";
  import type { WhiteboardLayoutResult } from "../../whiteboardLayout";
  import type { TermFloatLabels } from "./liveTypes";

  interface Props {
    activeWhiteboardLayout: WhiteboardLayoutResult | null;
    activeSummaryTerms: LiveTermExplanation[];
    termsCollapsed: boolean;
    collapsedTermPreview: LiveTermExplanation[];
    termCardIdx: number;
    termFloatLabels: TermFloatLabels;
    termStackOffset: (index: number) => number;
    onOpenWhiteboard: () => void;
    onToggleTermsCollapsed: () => void;
    onSelectTermCard: (index: number) => void;
    onTermCardPrev: () => void;
    onTermCardNext: () => void;
  }

  let {
    activeWhiteboardLayout,
    activeSummaryTerms,
    termsCollapsed,
    collapsedTermPreview,
    termCardIdx,
    termFloatLabels,
    termStackOffset,
    onOpenWhiteboard,
    onToggleTermsCollapsed,
    onSelectTermCard,
    onTermCardPrev,
    onTermCardNext,
  }: Props = $props();
</script>

{#if activeWhiteboardLayout || activeSummaryTerms.length > 0}
  <div class="right-rail">
    {#if activeWhiteboardLayout}
      <aside class="board-stack" aria-label={termFloatLabels.boardTitle}>
        <button
          type="button"
          class="board-preview-card"
          class:dense={activeWhiteboardLayout.nodes.length > 8}
          class:very-dense={activeWhiteboardLayout.nodes.length > 14}
          onclick={onOpenWhiteboard}
          aria-label={termFloatLabels.expand}
          title={termFloatLabels.expand}
        >
          <div class="board-preview-canvas">
            <svg class="board-preview-links" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
              {#each activeWhiteboardLayout.edges as edge (edge.id)}
                <line x1={edge.x1} y1={edge.y1} x2={edge.x2} y2={edge.y2} />
              {/each}
            </svg>
            {#each activeWhiteboardLayout.nodes as node (node.id)}
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

    {#if activeSummaryTerms.length > 0}
      <aside class="term-stack" class:collapsed={termsCollapsed} aria-label={termFloatLabels.title}>
        {#if termsCollapsed}
          <button
            type="button"
            class="term-stack-collapsed"
            onclick={onToggleTermsCollapsed}
            aria-label={termFloatLabels.expand}
            title={termFloatLabels.expand}
          >
            <span class="term-stack-preview" aria-hidden="true">
              {#each collapsedTermPreview as item, i (i + "-" + item.term)}
                <span class="term-stack-preview-chip">{item.term}</span>
              {/each}
            </span>
            <svg class="term-stack-expand-icon" width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden="true"><path d="M3 7.5 6 4.5l3 3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>
          </button>
        {:else}
          {#each activeSummaryTerms as item, i (i + "-" + item.term)}
            {@const offset = termStackOffset(i)}
            {@const visible = offset >= 0 && offset <= 2}
            <button
              type="button"
              class="term-card"
              class:active={offset === 0}
              class:peek={offset > 0}
              style="
                transform: translateY({offset * 14}px) scale({1 - offset * 0.04});
                opacity: {offset === 0 ? 1 : 0.72 - (offset - 1) * 0.22};
                z-index: {100 - offset};
                pointer-events: {visible ? 'auto' : 'none'};
                visibility: {visible ? 'visible' : 'hidden'};
                {visible ? '' : 'transition: none;'}
              "
              onclick={() => onSelectTermCard(i)}
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
            <button class="term-stack-arrow collapse" onclick={onToggleTermsCollapsed} aria-label={termFloatLabels.collapse} title={termFloatLabels.collapse}>
              <svg width="10" height="10" viewBox="0 0 12 12" fill="none"><path d="M3 4.5 6 7.5l3-3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
          </div>
        {/if}
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
    width: min(300px, calc(100% - 32px));
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
    pointer-events: none;
  }
  .right-rail > * {
    pointer-events: auto;
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
    border: 0.5px solid color-mix(in srgb, var(--accent) 18%, var(--glass-border));
    background: color-mix(in srgb, var(--bg-primary) 90%, transparent);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
    backdrop-filter: blur(18px) saturate(1.25);
    -webkit-backdrop-filter: blur(18px) saturate(1.25);
    cursor: pointer;
    overflow: hidden;
    transition: transform 0.18s cubic-bezier(0.22, 1, 0.36, 1),
                box-shadow 0.18s ease, border-color 0.18s ease;
  }
  .board-preview-card:hover {
    transform: translateY(-1px);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.1);
    border-color: color-mix(in srgb, var(--accent) 32%, var(--glass-border));
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
    height: 150px;
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
    stroke: currentColor;
    stroke-width: 0.7;
    stroke-linecap: round;
    opacity: 0.7;
  }
  .board-preview-node {
    position: absolute;
    transform: translate(-50%, -50%);
    z-index: 2;
    max-width: 64px;
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
  .board-preview-card.dense .board-preview-node {
    max-width: 52px;
    padding: 1.5px 5px;
    font-size: 8.5px;
  }
  .board-preview-card.very-dense .board-preview-node {
    max-width: 42px;
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
  .term-stack.collapsed {
    width: fit-content;
    max-width: 100%;
    align-self: flex-end;
  }

  .term-stack-collapsed {
    width: auto;
    max-width: 100%;
    min-height: 32px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    font-family: inherit;
    color: var(--text-primary);
    text-align: left;
    border-radius: 999px;
    border: 0.5px solid color-mix(in srgb, var(--accent) 20%, var(--glass-border));
    background: color-mix(in srgb, var(--bg-primary) 90%, transparent);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
    backdrop-filter: blur(18px) saturate(1.2);
    -webkit-backdrop-filter: blur(18px) saturate(1.2);
    cursor: pointer;
  }
  .term-stack-collapsed:hover {
    background: color-mix(in srgb, var(--bg-primary) 84%, transparent);
  }
  .term-stack-preview {
    min-width: 0;
    flex: 0 1 auto;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .term-stack-preview-chip {
    min-width: 18px;
    max-width: 80px;
    flex: 0 1 auto;
    padding: 2px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--text-primary);
    font-size: 10.5px;
    font-weight: 700;
    line-height: 1.35;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .term-stack-expand-icon {
    flex: 0 0 auto;
    color: var(--accent);
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
    border: 0.5px solid color-mix(in srgb, var(--accent) 18%, var(--glass-border));
    background: color-mix(in srgb, var(--bg-primary) 96%, transparent);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
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
    cursor: default;
    background: color-mix(in srgb, var(--bg-primary) 90%, transparent);
    backdrop-filter: blur(18px) saturate(1.25);
    -webkit-backdrop-filter: blur(18px) saturate(1.25);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
  }
  .term-card.peek:hover {
    background: color-mix(in srgb, var(--bg-primary) 94%, transparent);
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
    -webkit-line-clamp: 4;
    line-clamp: 4;
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
  .term-stack-arrow.collapse {
    margin-left: 2px;
    color: var(--text-secondary);
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
