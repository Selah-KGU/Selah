<script lang="ts">
  interface Props {
    tipKey: string;
    title: string;
    body: string;
  }
  const { tipKey, title, body }: Props = $props();

  const storageKey = $derived(`selah-tip-${tipKey}-v1`);
  let dismissed = $state<boolean>(true);

  $effect(() => {
    try { dismissed = localStorage.getItem(storageKey) === "1"; }
    catch { dismissed = true; }
  });

  function dismiss() {
    try { localStorage.setItem(storageKey, "1"); } catch { /* ignore */ }
    dismissed = true;
  }
</script>

{#if !dismissed}
  <div class="tip">
    <div class="tip-body">
      <div class="tip-title">{title}</div>
      <div class="tip-text">{body}</div>
    </div>
    <button class="tip-close" onclick={dismiss} aria-label="閉じる">×</button>
  </div>
{/if}

<style>
  .tip {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 12px;
    margin-bottom: 12px;
    background: color-mix(in srgb, var(--accent) 6%, var(--bg-secondary));
    border: 0.5px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    border-radius: 8px;
    font-size: 11.5px;
    line-height: 1.45;
  }
  .tip-body { flex: 1; min-width: 0; }
  .tip-title { font-weight: 600; color: var(--text-primary); }
  .tip-text { color: var(--text-secondary); margin-top: 2px; }
  .tip-close {
    flex-shrink: 0;
    background: none;
    border: none;
    width: 22px; height: 22px;
    border-radius: 5px;
    color: var(--text-tertiary);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    font-family: inherit;
  }
  .tip-close:hover { background: var(--bg-hover); color: var(--text-primary); }
</style>
