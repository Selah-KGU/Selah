<script lang="ts">
  import App from "./App.svelte";
  import "./styles.css";

  function reportRenderError(error: unknown) {
    const detail = error instanceof Error ? error.stack || error.message : String(error);
    window.__SELAH_REPORT_ERROR__?.(`[Selah] render failed: ${detail}`);
    console.error("[Selah] render failed:", detail);
  }
</script>

<svelte:boundary onerror={reportRenderError}>
  <App />
  {#snippet failed(_error, _reset)}
    <main class="render-recovery" role="alert">
      <p>画面の表示中にエラーが発生しました。</p>
      <p>LIVEの録音中は、画面を再読み込みしても録音を継続します。</p>
      <button type="button" onclick={() => window.location.reload()}>画面を再読み込み</button>
    </main>
  {/snippet}
</svelte:boundary>

<style>
  .render-recovery {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 32px;
    color: var(--text-primary);
    background: var(--bg-primary);
  }
  p { margin: 0 0 12px; }
  button { margin-top: 12px; padding: 10px 18px; cursor: pointer; }
</style>
