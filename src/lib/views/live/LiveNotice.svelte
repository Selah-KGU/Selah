<script lang="ts">
  import type { NoticeState } from "./liveTypes";

  interface Props {
    notice: NoticeState;
    onOpenAiSettings: () => void;
  }

  let { notice, onOpenAiSettings }: Props = $props();
</script>

{#if notice}
  <div class="inline-msg {notice.kind}" class:has-action={!!notice.action}>
    <span>{notice.text}</span>
    {#if notice.action === "open-ai-settings"}
      <button class="inline-msg-action" onclick={onOpenAiSettings}>AI設定</button>
    {/if}
  </div>
{/if}

<style>
  .inline-msg {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 9px 14px;
    border-radius: 10px;
    font-size: 12.5px;
    font-weight: 500;
    margin-bottom: 10px;
    animation: toast-enter 0.25s ease-out;
  }
  .inline-msg.has-action {
    flex-wrap: wrap;
  }
  .inline-msg.error {
    background: color-mix(in srgb, var(--red) 10%, transparent);
    color: var(--red);
    border: 0.5px solid color-mix(in srgb, var(--red) 15%, transparent);
  }
  .inline-msg.warning {
    background: color-mix(in srgb, var(--orange, #e67700) 8%, var(--bg-card));
    color: var(--orange, #e67700);
    border: 0.5px solid color-mix(in srgb, var(--orange, #e67700) 18%, transparent);
  }
  .inline-msg.success {
    background: color-mix(in srgb, var(--green) 10%, transparent);
    color: var(--green);
    border: 0.5px solid color-mix(in srgb, var(--green) 15%, transparent);
  }
  .inline-msg-action {
    border: none;
    background: color-mix(in srgb, currentColor 12%, transparent);
    color: inherit;
    border-radius: 999px;
    padding: 4px 10px;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.18s ease, transform 0.18s ease;
  }
  .inline-msg-action:hover {
    background: color-mix(in srgb, currentColor 18%, transparent);
    transform: translateY(-1px);
  }
  .inline-msg-action:focus-visible {
    outline: 2px solid color-mix(in srgb, currentColor 35%, transparent);
    outline-offset: 2px;
  }

  @keyframes toast-enter {
    from { opacity: 0; transform: translateY(-6px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
