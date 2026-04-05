<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    loading: boolean;
    error: string;
    empty?: boolean;
    emptyMessage?: string;
    children: Snippet;
  }

  let { loading, error, empty = false, emptyMessage = "データがありません", children }: Props = $props();
</script>

{#if loading}
  <div class="state-msg">
    <span class="loading-spinner"></span>
  </div>
{:else if error}
  <div class="state-msg error">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: 6px; flex-shrink: 0;">
      <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
    </svg>
    {error}
  </div>
{:else if empty}
  <div class="state-msg">{emptyMessage}</div>
{:else}
  {@render children()}
{/if}
