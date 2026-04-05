<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  interface Props {
    open: boolean;
    title?: string;
    subtitle?: string;
    closeable?: boolean;
    onclose: () => void;
    children: Snippet;
  }

  let { open, title, subtitle, closeable = true, onclose, children }: Props = $props();
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && open && closeable) onclose(); }} />

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-backdrop" onclick={() => closeable && onclose()} onkeydown={() => {}}></div>
  <div class="modal-container">
    <div class="modal-header">
      <div class="modal-title-area">
        {#if subtitle}
          <span class="modal-subtitle">{subtitle}</span>
        {/if}
        {#if title}
          <h3 class="modal-title">{title}</h3>
        {/if}
      </div>
      {#if closeable}
        <button class="modal-close" onclick={onclose}>
          <Icon name="xmark" size={16} />
        </button>
      {/if}
    </div>
    <div class="modal-body">
      {@render children()}
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 99;
    background: rgba(0, 0, 0, 0.35);
    animation: fade-in 0.2s ease;
  }
  .modal-container {
    position: fixed;
    z-index: 100;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(520px, calc(100vw - 40px));
    max-height: calc(100vh - 60px);
    background: var(--bg-card);
    border-radius: 14px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.2), 0 0 0 0.5px var(--border);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    animation: modal-in 0.2s ease;
  }
  @keyframes modal-in {
    from {
      opacity: 0;
      transform: translate(-50%, -48%) scale(0.96);
    }
    to {
      opacity: 1;
      transform: translate(-50%, -50%) scale(1);
    }
  }
  .modal-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    padding: 16px 20px 12px;
    gap: 12px;
    flex-shrink: 0;
    border-bottom: 0.5px solid var(--border);
  }
  .modal-title-area {
    flex: 1;
    min-width: 0;
  }
  .modal-subtitle {
    font-size: 11px;
    color: var(--text-secondary);
    letter-spacing: 0.02em;
  }
  .modal-title {
    font-size: 17px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 2px 0 0;
    line-height: 1.3;
  }
  .modal-close {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-secondary);
    background: var(--bg-secondary);
    border: none;
    cursor: pointer;
    transition: background 0.1s;
    margin-top: 2px;
  }
  .modal-close:hover {
    background: var(--bg-tertiary);
  }
  .modal-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px 20px;
  }
</style>
