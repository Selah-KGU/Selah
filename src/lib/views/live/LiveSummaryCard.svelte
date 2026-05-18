<script lang="ts">
  import type { LiveSessionSnapshot } from "../../api";

  type SummaryChunk = LiveSessionSnapshot["summaries"][number];

  interface Props {
    summaries: SummaryChunk[];
    activeSummaryIdx: number;
    summaryExpanded: boolean;
    renderMd: (text: string) => string;
    onSelectSummaryView: (event: MouseEvent, idx: number) => void;
    onExpand: () => void;
    onCollapse: () => void;
    bindSummaryOverlayDismiss: (node: HTMLDivElement) => { destroy?: () => void } | void;
  }

  let {
    summaries,
    activeSummaryIdx,
    summaryExpanded,
    renderMd,
    onSelectSummaryView,
    onExpand,
    onCollapse,
    bindSummaryOverlayDismiss,
  }: Props = $props();
</script>

{#if summaries.length > 0}
  {@const chunk = summaries[activeSummaryIdx]}
  {@const total = summaries.length}
  {#if chunk}
    <div class="summary-card" class:expanded={summaryExpanded}>
      <div class="summary-card-header">
        <span class="toast-ai-badge"><svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke-width="1.3"><defs><linearGradient id="ai-g1" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#c480e8"/><stop offset="100%" stop-color="#6bacf0"/></linearGradient></defs><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="url(#ai-g1)" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="url(#ai-g1)" stroke-linejoin="round" stroke-width="1"/></svg><span class="toast-badge-text">AI 要点</span></span>
        {#if total > 1}
          <div class="summary-time-pills">
            {#each summaries as s, idx}
              <button
                class="time-pill"
                class:active={idx === activeSummaryIdx}
                onclick={(e) => onSelectSummaryView(e, idx)}
              >{s.range_label}</button>
            {/each}
          </div>
        {:else}
          <span class="toast-meta">{chunk.range_label}</span>
        {/if}
        <button class="toast-expand-btn" onclick={summaryExpanded ? onCollapse : onExpand}>{summaryExpanded ? '収める' : '展開'}</button>
      </div>
      <div class="summary-card-body md">{@html renderMd(chunk.body)}</div>
      {#if summaryExpanded}
        <div class="summary-card-overlay" use:bindSummaryOverlayDismiss>
          <div class="summary-card-header">
            <span class="toast-ai-badge"><svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke-width="1.3"><defs><linearGradient id="ai-g2" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#c480e8"/><stop offset="100%" stop-color="#6bacf0"/></linearGradient></defs><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="url(#ai-g2)" stroke-linejoin="round"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="url(#ai-g2)" stroke-linejoin="round" stroke-width="1"/></svg><span class="toast-badge-text">AI 要点</span></span>
            {#if total > 1}
              <div class="summary-time-pills">
                {#each summaries as s, idx}
                  <button
                    class="time-pill"
                    class:active={idx === activeSummaryIdx}
                    onclick={(e) => onSelectSummaryView(e, idx)}
                  >{s.range_label}</button>
                {/each}
              </div>
            {:else}
              <span class="toast-meta">{chunk.range_label}</span>
            {/if}
            <button class="toast-expand-btn" onclick={onCollapse}>収める</button>
          </div>
          <div class="summary-card-full md">{@html renderMd(chunk.body)}</div>
        </div>
      {/if}
    </div>
  {/if}
{/if}

<style>
  .summary-card {
    position: sticky;
    top: 56px;
    z-index: 35;
    margin-bottom: 14px;
    background: #f9f6fc;
    border: 0.5px solid rgba(175, 82, 222, 0.22);
    border-radius: 14px;
    padding: 10px 16px;
    box-shadow: 0 2px 16px rgba(175, 82, 222, 0.08), 0 1px 3px rgba(0, 0, 0, 0.04);
    animation: card-enter 0.4s cubic-bezier(0.22, 1, 0.36, 1) both;
    overflow: hidden;
    transition: box-shadow 0.3s ease;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .summary-card {
      background: #1c1c20;
      border-color: rgba(191, 90, 242, 0.24);
      box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28), 0 0 0 1px rgba(255, 255, 255, 0.04);
    }
    :global(:root:not([data-theme="light"])) .summary-card.expanded {
      box-shadow: 0 14px 36px rgba(0, 0, 0, 0.34), 0 0 0 1px rgba(255, 255, 255, 0.05);
    }
  }
  :global([data-theme="dark"]) .summary-card {
    background: #1c1c20;
    border-color: rgba(191, 90, 242, 0.24);
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28), 0 0 0 1px rgba(255, 255, 255, 0.04);
  }
  :global([data-theme="dark"]) .summary-card.expanded {
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.34), 0 0 0 1px rgba(255, 255, 255, 0.05);
  }
  .summary-card.expanded {
    overflow: visible;
    box-shadow: 0 4px 24px rgba(175, 82, 222, 0.12), 0 1px 3px rgba(0, 0, 0, 0.04);
  }

  .summary-card-header {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 8px;
    margin-bottom: 4px;
    min-width: 0;
  }
  .toast-ai-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .toast-badge-text {
    font-size: 12px;
    font-weight: 700;
    background: linear-gradient(135deg, #c480e8, #6bacf0);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    letter-spacing: 0.3px;
    line-height: 1;
  }

  .summary-time-pills {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    width: 100%;
    flex-wrap: nowrap;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }
  .summary-time-pills::-webkit-scrollbar { display: none; }
  .time-pill {
    all: unset;
    cursor: pointer;
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    padding: 2px 8px;
    border-radius: 20px;
    border: 0.5px solid color-mix(in srgb, var(--text-tertiary) 20%, transparent);
    transition: all 0.2s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .time-pill:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--text-secondary);
    transform: scale(1.04);
  }
  .time-pill.active {
    background: linear-gradient(135deg, rgba(196, 128, 232, 0.12), rgba(107, 172, 240, 0.12));
    border-color: rgba(175, 82, 222, 0.3);
    color: var(--text-primary);
    font-weight: 600;
    transform: scale(1.06);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .time-pill {
      color: rgba(245, 245, 247, 0.76);
      border-color: rgba(255, 255, 255, 0.1);
      background: rgba(255, 255, 255, 0.03);
    }
    :global(:root:not([data-theme="light"])) .time-pill:hover {
      background: rgba(74, 144, 217, 0.16);
      color: rgba(245, 245, 247, 0.92);
    }
    :global(:root:not([data-theme="light"])) .time-pill.active {
      background: linear-gradient(135deg, rgba(191, 90, 242, 0.2), rgba(74, 144, 217, 0.2));
      border-color: rgba(191, 90, 242, 0.34);
    }
  }
  :global([data-theme="dark"]) .time-pill {
    color: rgba(245, 245, 247, 0.76);
    border-color: rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.03);
  }
  :global([data-theme="dark"]) .time-pill:hover {
    background: rgba(74, 144, 217, 0.16);
    color: rgba(245, 245, 247, 0.92);
  }
  :global([data-theme="dark"]) .time-pill.active {
    background: linear-gradient(135deg, rgba(191, 90, 242, 0.2), rgba(74, 144, 217, 0.2));
    border-color: rgba(191, 90, 242, 0.34);
  }

  .toast-meta {
    font-size: 11px;
    color: var(--text-tertiary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toast-expand-btn {
    all: unset;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    font-size: 10px;
    color: var(--accent);
    font-weight: 500;
    opacity: 0.8;
    padding: 0px 6px;
    min-height: 20px;
    border-radius: 4px;
    white-space: nowrap;
    justify-self: end;
    position: relative;
    z-index: 1;
    transition: background 0.12s;
  }
  .toast-expand-btn:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    opacity: 1;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .toast-expand-btn:hover {
      background: rgba(74, 144, 217, 0.18);
    }
  }
  :global([data-theme="dark"]) .toast-expand-btn:hover {
    background: rgba(74, 144, 217, 0.18);
  }

  .summary-card-body {
    margin: 0;
    font-size: 13.5px;
    font-weight: 400;
    line-height: 1.65;
    color: var(--text-primary);
    overflow: hidden;
  }
  .summary-card-body :global(hr),
  .summary-card-body :global(hr ~ *) {
    display: none;
  }

  .summary-card-overlay {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    padding: 10px 16px;
    background: #f9f6fc;
    border: 0.5px solid rgba(175, 82, 222, 0.22);
    border-radius: 14px;
    box-shadow: 0 8px 32px rgba(175, 82, 222, 0.12), var(--shadow-md);
    z-index: 70;
    cursor: pointer;
    animation: overlay-expand 0.3s cubic-bezier(0.22, 1, 0.36, 1) both;
    transform-origin: top center;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .summary-card-overlay {
      background: #1c1c20;
      border-color: rgba(191, 90, 242, 0.28);
      box-shadow: 0 18px 40px rgba(0, 0, 0, 0.38), 0 0 0 1px rgba(255, 255, 255, 0.05);
    }
  }
  :global([data-theme="dark"]) .summary-card-overlay {
    background: #1c1c20;
    border-color: rgba(191, 90, 242, 0.28);
    box-shadow: 0 18px 40px rgba(0, 0, 0, 0.38), 0 0 0 1px rgba(255, 255, 255, 0.05);
  }
  .summary-card-full {
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text-primary);
  }
  .summary-card-full :global(hr ~ ul),
  .summary-card-full :global(hr ~ ol) {
    list-style: none;
    padding-left: 0;
  }

  .summary-card-body.md :global(hr),
  .summary-card-full.md :global(hr) {
    margin: 8px 0;
    border: none;
    border-top: 0.5px solid var(--glass-border);
  }
  .summary-card-body.md :global(p),
  .summary-card-full.md :global(p) { margin: 0 0 4px; }
  .summary-card-body.md :global(p:last-child),
  .summary-card-full.md :global(p:last-child) { margin-bottom: 0; }
  .summary-card-body.md :global(ul), .summary-card-body.md :global(ol),
  .summary-card-full.md :global(ul), .summary-card-full.md :global(ol) { margin: 0 0 4px; padding-left: 16px; }
  .summary-card-body.md :global(li),
  .summary-card-full.md :global(li) { margin-bottom: 2px; }
  .summary-card-body.md :global(h1), .summary-card-body.md :global(h2), .summary-card-body.md :global(h3),
  .summary-card-body.md :global(h4), .summary-card-body.md :global(h5),
  .summary-card-full.md :global(h1), .summary-card-full.md :global(h2), .summary-card-full.md :global(h3),
  .summary-card-full.md :global(h4), .summary-card-full.md :global(h5) {
    font-size: 13px;
    font-weight: 600;
    margin: 6px 0 3px;
    color: var(--text-primary);
  }
  .summary-card-body.md :global(h1:first-child), .summary-card-body.md :global(h2:first-child),
  .summary-card-body.md :global(h3:first-child),
  .summary-card-full.md :global(h1:first-child), .summary-card-full.md :global(h2:first-child),
  .summary-card-full.md :global(h3:first-child) { margin-top: 0; }
  .summary-card-body.md :global(code),
  .summary-card-full.md :global(code) {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    padding: 1px 4px;
    border-radius: 4px;
    font-size: 0.88em;
  }
  .summary-card-body.md :global(pre),
  .summary-card-full.md :global(pre) {
    background: color-mix(in srgb, var(--text-primary) 4%, transparent);
    padding: 8px 10px;
    border-radius: 8px;
    overflow-x: auto;
    font-size: 12px;
    line-height: 1.5;
  }
  .summary-card-body.md :global(pre code),
  .summary-card-full.md :global(pre code) { background: transparent; padding: 0; }
  .summary-card-body.md :global(blockquote),
  .summary-card-full.md :global(blockquote) {
    margin: 4px 0;
    padding-left: 10px;
    border-left: 2px solid var(--border);
    color: var(--text-secondary);
  }
  .summary-card-body.md :global(strong),
  .summary-card-full.md :global(strong) { font-weight: 600; }
  .summary-card-body.md :global(a),
  .summary-card-full.md :global(a) { color: var(--accent); text-decoration: none; }
  .summary-card-body.md :global(a:hover),
  .summary-card-full.md :global(a:hover) { text-decoration: underline; }

  @keyframes card-enter {
    from {
      opacity: 0;
      transform: translateY(-10px) scale(0.96);
      filter: blur(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
      filter: blur(0);
    }
  }

  @keyframes overlay-expand {
    from {
      opacity: 0;
      transform: scaleY(0.92) translateY(-4px);
    }
    to {
      opacity: 1;
      transform: scaleY(1) translateY(0);
    }
  }
</style>
