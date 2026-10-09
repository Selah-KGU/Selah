<script lang="ts">
  import { AGENT_ATTACHMENT_ACCEPT, MAX_AGENT_ATTACHMENTS } from "./agentAttachments";
  let { reading = 0, count = 0, error = "" }: { reading?: number; count?: number; error?: string } = $props();
</script>

<div class="attachment-feedback">
  <p class="attachment-help" title={AGENT_ATTACHMENT_ACCEPT}>
    画像・PDF・Word・PowerPoint・Excel・テキスト ／ <span>最大{MAX_AGENT_ATTACHMENTS}件・1件10MB</span>
  </p>
  <div class="attachment-status" role="status" aria-live="polite" aria-atomic="true">
    {#if reading > 0}<p>添付を読み込み中… 読み取り完了後に送信できます</p>
    {:else if count > 0}<p>添付{count}件を準備できました。送信時にAgentへ渡します</p>{/if}
    {#if error}<p class="attachment-error">{error}</p>{/if}
  </div>
</div>

<style>
  .attachment-feedback { margin: 0 6px 6px; font-size: 11px; line-height: 1.5; color: var(--agent-text-2, var(--text-secondary, #666)); overflow-wrap: anywhere; }
  .attachment-help span { white-space: nowrap; }
  p { margin: 0; }
  .attachment-status p { margin-top: 3px; }
  .attachment-error { color: var(--danger-color, #b33a3a); }
  :global([data-theme="dark"]) .attachment-error { color: #ffb4ab; }
</style>
