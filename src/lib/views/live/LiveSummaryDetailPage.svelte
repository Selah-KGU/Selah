<script lang="ts">
  import type { LiveTermExplanation } from "../../api";
  import { parseSummaryDetail } from "./liveMarkdown";

  // Full-screen secondary page (not a popup) for the stage summary: switch
  // periods, read the whole rendered body, and the difficult-word notes.
  // Neutral, minimal styling — no accent/color theming.
  type SummaryEntry = {
    range_label: string;
    body: string;
    isOverall: boolean;
    terms: LiveTermExplanation[];
  };

  interface Props {
    entries: SummaryEntry[];
    activeIdx: number;
    renderMd: (text: string) => string;
    onSelectSummaryView: (event: MouseEvent, idx: number) => void;
    onClose: () => void;
  }

  let { entries, activeIdx, renderMd, onSelectSummaryView, onClose }: Props = $props();

  const chunk = $derived(entries[activeIdx]);
  const detail = $derived(parseSummaryDetail(chunk?.body ?? ""));
  const terms = $derived(
    (chunk?.terms ?? []).filter((t) => t.term?.trim() && t.explanation?.trim()),
  );
</script>

<section class="detail" aria-label="阶段摘要の詳細">
  <header class="detail-head">
    <button type="button" class="detail-back" onclick={onClose} aria-label="戻る" title="戻る">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
    </button>
    {#if entries.length > 1}
      <div class="detail-pills">
        {#each entries as s, idx (idx)}
          <button
            class="pill"
            class:active={idx === activeIdx}
            onclick={(e) => onSelectSummaryView(e, idx)}
          >{s.isOverall ? '全体' : s.range_label}</button>
        {/each}
      </div>
    {:else if chunk}
      <span class="detail-title">{chunk.range_label}</span>
    {/if}
  </header>

  <div class="detail-scroll">
    {#if chunk}
      {#if detail.structured}
        <article class="detail-body">
          {#if detail.overview.length > 0}
            <ol class="overview">
              {#each detail.overview as point, i (i)}
                <li>{point}</li>
              {/each}
            </ol>
          {/if}
          <div class="sections">
            {#each detail.sections as s, i (i)}
              <section class="sec">
                {#if s.heading}
                  <h4 class="sec-head">
                    <span class="sec-num">{i + 1}</span>
                    <span class="sec-title">{s.heading}</span>
                  </h4>
                {/if}
                <div class="sec-text md">{@html renderMd(s.text)}</div>
              </section>
            {/each}
          </div>
        </article>
      {:else}
        <article class="detail-body md raw">{@html renderMd(chunk.body)}</article>
      {/if}

      {#if terms.length > 0}
        <section class="detail-terms" aria-label="難語">
          <h3 class="detail-terms-head">難語</h3>
          <ul class="term-list">
            {#each terms as t, i (i)}
              <li class="term-item">
                <div class="term-name">{t.term}</div>
                <div class="term-exp">{t.explanation}</div>
                {#if t.source_excerpt}
                  <div class="term-sub"><span class="term-sub-tag">引用</span>{t.source_excerpt}</div>
                {/if}
                {#if t.external_source}
                  <div class="term-sub"><span class="term-sub-tag">出典</span>{t.external_source}</div>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}
  </div>
</section>

<style>
  .detail {
    position: absolute;
    inset: -24px;
    z-index: 60;
    display: flex;
    flex-direction: column;
    background: var(--bg-primary);
    animation: detail-in 0.24s cubic-bezier(0.22, 1, 0.36, 1) both;
  }

  .detail-head {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 24px 12px;
  }
  /* Same chip family as the pills: round, 0.5px neutral border, neutral hover. */
  /* Same chip family as the pills: round, 0.5px neutral border, neutral hover. */
  .detail-back {
    width: 34px;
    height: 34px;
    flex: 0 0 auto;
    padding: 0;
    border: 0.5px solid var(--glass-border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-tertiary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s, color 0.15s, transform 0.15s;
  }
  .detail-back:hover {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    color: var(--text-primary);
  }
  .detail-back:active { transform: scale(0.94); }
  .detail-back svg { width: 17px; height: 17px; }
  .detail-title {
    min-width: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail-pills {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .detail-pills::-webkit-scrollbar { display: none; }
  .pill {
    all: unset;
    cursor: pointer;
    flex-shrink: 0;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text-tertiary);
    padding: 7px 14px;
    border-radius: 999px;
    border: 0.5px solid var(--glass-border);
    transition: background 0.15s, color 0.15s, border-color 0.15s;
  }
  .pill:hover {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    color: var(--text-secondary);
  }
  .pill.active {
    background: color-mix(in srgb, var(--text-primary) 10%, transparent);
    border-color: color-mix(in srgb, var(--text-primary) 22%, var(--glass-border));
    color: var(--text-primary);
  }

  .detail-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 24px 32px;
    scrollbar-width: thin;
  }

  /* ── Summary body ── */
  .detail-body {
    max-width: 720px;
    margin: 0 auto;
    font-size: 14.5px;
    line-height: 1.75;
    color: var(--text-primary);
  }
  .detail-body :global(hr) {
    margin: 16px 0;
    border: none;
    border-top: 0.5px solid var(--glass-border);
  }
  .detail-body :global(p) { margin: 0 0 9px; }
  .detail-body :global(ul), .detail-body :global(ol) {
    margin: 0 0 9px;
    padding-left: 20px;
  }
  .detail-body :global(li) { margin-bottom: 4px; }
  .detail-body :global(h1), .detail-body :global(h2),
  .detail-body :global(h3), .detail-body :global(h4) {
    font-size: 14px;
    font-weight: 700;
    margin: 16px 0 6px;
    color: var(--text-primary);
  }
  .detail-body :global(h1:first-child), .detail-body :global(h2:first-child),
  .detail-body :global(h3:first-child) { margin-top: 0; }
  .detail-body :global(strong) { font-weight: 700; }
  .detail-body :global(code) {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    padding: 1px 4px;
    border-radius: 4px;
    font-size: 0.88em;
  }
  .detail-body :global(a) {
    color: var(--text-primary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  /* ── Overview: at-a-glance index (primary) ── */
  .detail-body .overview {
    list-style: none;
    counter-reset: ov;
    margin: 0 0 22px;
    padding: 14px 16px;
    border: 0.5px solid var(--glass-border);
    border-radius: 12px;
    background: color-mix(in srgb, var(--text-primary) 3%, transparent);
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .detail-body .overview li {
    counter-increment: ov;
    position: relative;
    margin: 0;
    padding-left: 26px;
    font-size: 14px;
    font-weight: 600;
    line-height: 1.5;
    color: var(--text-primary);
  }
  .detail-body .overview li::before {
    content: counter(ov);
    position: absolute;
    left: 0;
    top: 0.1em;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-primary) 9%, transparent);
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  /* ── Detail sections: numbered heading (primary) + explanation (secondary) ── */
  .sections {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .sec-head {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 0 0 5px;
  }
  .sec-num {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-primary) 86%, transparent);
    color: var(--bg-primary);
    font-size: 11px;
    font-weight: 800;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .sec-title {
    font-size: 15px;
    font-weight: 700;
    line-height: 1.4;
    color: var(--text-primary);
  }
  .sec-text {
    padding-left: 29px;
    font-size: 13.5px;
    line-height: 1.72;
    color: var(--text-secondary);
  }
  .sec-text :global(p) { margin: 0 0 6px; }
  .sec-text :global(p:last-child) { margin-bottom: 0; }

  /* ── Difficult-word notes ── */
  .detail-terms {
    max-width: 720px;
    margin: 22px auto 0;
    padding-top: 16px;
    border-top: 0.5px solid var(--glass-border);
  }
  .detail-terms-head {
    margin: 0 0 10px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.4px;
    color: var(--text-tertiary);
  }
  .term-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .term-name {
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.4;
  }
  .term-exp {
    margin-top: 3px;
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text-secondary);
  }
  .term-sub {
    margin-top: 4px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-tertiary);
  }
  .term-sub-tag {
    display: inline-block;
    margin-right: 6px;
    padding: 0 5px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--text-tertiary) 14%, transparent);
    font-size: 10px;
    font-weight: 700;
    vertical-align: 1px;
  }

  @keyframes detail-in {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
