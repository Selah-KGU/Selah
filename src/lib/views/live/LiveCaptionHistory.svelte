<script lang="ts">
  import { onDestroy } from "svelte";
  import { ResourceScope } from "../../resourceScope";
  import { LiveTranscriptFollow } from "./liveTranscriptFollow";
  import type { RefinedCaption } from "./liveCaptionRefine";

  let { items, visible }: { items: RefinedCaption[]; visible: boolean } = $props();
  let scrollEl = $state<HTMLElement | null>(null);
  let autoFollow = $state(true);
  const resources = new ResourceScope();
  const follow = new LiveTranscriptFollow(resources);
  let previousItems: RefinedCaption[] | undefined;
  let revision = 0;

  $effect(() => {
    if (items !== previousItems) {
      previousItems = items;
      revision += 1;
    }
    follow.update({
      recordingId: "caption-history",
      lineCount: revision,
      target: scrollEl,
      visible,
      listening: true,
      autoFollow,
    });
  });
  onDestroy(() => resources.dispose());
</script>

<section
  class="caption-history"
  aria-label="字幕翻訳の履歴"
  tabindex="0"
  bind:this={scrollEl}
  onscroll={() => {
    if (visible && scrollEl) autoFollow = scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight < 24;
  }}
>
  {#each items as item (`${item.segment_id}:${item.sentence_index ?? 0}`)}
    <div class="lyric-refine" class:pending={!item.ready}>
      <span class="lyric-corrected">{item.corrected}</span>
      {#if item.translation}<span class="lyric-translation">訳 {item.translation}</span>{/if}
    </div>
  {/each}
</section>

<style>
  .caption-history {
    max-height: min(28vh, 220px);
    overflow-y: auto;
    overflow-anchor: none;
    overscroll-behavior: contain;
    scrollbar-width: thin;
    margin: 8px 12px 8px 56px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .lyric-refine { padding: 6px 10px; }
  .lyric-refine.pending { opacity: 0.72; }
  .lyric-corrected {
    display: block;
    font-size: 15px;
    line-height: 1.5;
    color: var(--text-primary);
  }
  .lyric-translation {
    display: block;
    margin-top: 2px;
    font-size: 13px;
    line-height: 1.45;
    color: var(--accent);
  }
</style>
