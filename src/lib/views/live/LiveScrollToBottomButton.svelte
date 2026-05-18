<script lang="ts">
  interface Props {
    visible: boolean;
    onScrollToBottom: () => void;
  }

  let { visible, onScrollToBottom }: Props = $props();
</script>

{#if visible}
  <button class="scroll-to-bottom" onclick={onScrollToBottom}>
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="7 13 12 18 17 13"/><line x1="12" y1="18" x2="12" y2="6"/></svg>
    最新へ
  </button>
{/if}

<style>
  .scroll-to-bottom {
    position: absolute;
    bottom: 16px;
    left: 0;
    right: 0;
    margin: 0 auto;
    width: fit-content;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 14px 6px 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--glass-bg, rgba(255, 255, 255, 0.7));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    border-radius: 100px;
    box-shadow: var(--shadow-glass), 0 2px 8px rgba(0, 0, 0, 0.06);
    cursor: pointer;
    z-index: 15;
    animation: capsule-in 0.25s cubic-bezier(0.22, 1, 0.36, 1) both;
    transition: transform 0.15s ease, box-shadow 0.15s ease;
  }
  .scroll-to-bottom:hover {
    transform: scale(1.04);
    box-shadow: var(--shadow-glass), 0 4px 14px rgba(0, 0, 0, 0.1);
  }
  .scroll-to-bottom:active {
    transform: scale(0.97);
  }

  @keyframes capsule-in {
    from { opacity: 0; transform: scale(0.95); }
    to { opacity: 1; transform: scale(1); }
  }
</style>
