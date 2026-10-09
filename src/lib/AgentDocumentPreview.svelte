<script lang="ts">
  import type { AgentDocumentPart } from "./agentApi";
  let { document }: { document: AgentDocumentPart } = $props();
  const preview = $derived.by(() => {
    let text = "", count = 0;
    for (const character of document.text) {
      if (count++ === 2000) break;
      text += character;
    }
    return text;
  });
  const previewClipped = $derived(preview.length < document.text.length);
</script>

<details class="document-preview">
  <summary>{document.name}<span>{document.truncated ? "内容の一部を使用（読み取り上限または読めないページあり）" : "本文の読み取り完了"}</span></summary>
  <pre>{preview}{previewClipped ? "\n…" : ""}</pre>
  {#if previewClipped}<p class="preview-note">プレビューは先頭2,000文字です。読み取った本文はこの後も続きます。</p>{/if}
</details>

<style>
  .document-preview { max-width: 100%; font-size: 12px; text-align: left; }
  summary { cursor: pointer; overflow-wrap: anywhere; }
  summary span { display: block; margin-top: 3px; color: inherit; opacity: 0.85; font-size: 11px; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 180px; overflow: auto; margin: 8px 0 0; font: inherit; }
  .preview-note { color: inherit; opacity: 0.85; font-size: 11px; margin: 6px 0 0; }
</style>
