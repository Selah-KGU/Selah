<script lang="ts">
  import "./styles.css";
  import type { Component } from "svelte";
  import { onMount } from "svelte";
  import { readSurfaceParam } from "./lib/surfaceKind";

  const surface = readSurfaceParam();
  let Surface = $state<Component | null>(null);
  let error = $state("");

  onMount(async () => {
    try {
      if (surface === "document-tabs") {
        Surface = (await import("./lib/DocumentTabs.svelte")).default;
      } else if (surface === "markdown-reader") {
        Surface = (await import("./lib/MarkdownReaderSurface.svelte")).default;
      } else if (surface === "agent-panel") {
        Surface = (await import("./lib/AgentPanel.svelte")).default;
      } else if (surface === "files") {
        Surface = (await import("./lib/FilesSurface.svelte")).default;
      } else if (surface === "home") {
        Surface = (await import("./lib/NewTabSurface.svelte")).default;
      } else if (surface === "detective") {
        Surface = (await import("./lib/views/Detective.svelte")).default;
      } else if (surface === "paper-check") {
        Surface = (await import("./lib/views/PaperCheck.svelte")).default;
      } else if (surface === "split-divider") {
        Surface = (await import("./lib/SplitDividerSurface.svelte")).default;
      } else if (surface === "browser-mouse-selftest") {
        Surface = (await import("./lib/BrowserMouseSelftestSurface.svelte")).default;
      } else if (surface === "university-detail" || surface === "luna-detail") {
        Surface = (await import("./lib/UniversityDetailSurface.svelte")).default;
      } else {
        error = "未知の画面です";
      }
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  });
</script>

{#if Surface}
  <Surface />
{:else}
  <main class="auxiliary-state" class:errored={!!error}>
    {#if error}
      <strong>ページを読み込めませんでした</strong>
      <span>{error}</span>
    {:else}
      <span class="auxiliary-spinner" aria-hidden="true"></span>
    {/if}
  </main>
{/if}

<style>
  .auxiliary-state {
    min-height: 100vh;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 10px;
    box-sizing: border-box;
    padding: 32px;
    color: #60646c;
    background: #f7f8fa;
    font: 13px/1.5 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", "Noto Sans JP", sans-serif;
    text-align: center;
    opacity: 0;
    animation: auxiliary-appear 0.3s ease 0.16s forwards;
  }

  .auxiliary-state.errored {
    opacity: 1;
    animation: none;
  }

  @keyframes auxiliary-appear {
    to { opacity: 1; }
  }

  .auxiliary-state strong {
    color: #24262b;
    font-size: 15px;
  }

  .auxiliary-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid rgba(96, 100, 108, 0.2);
    border-top-color: #60646c;
    border-radius: 50%;
    animation: auxiliary-spin 0.8s linear infinite;
  }

  :global([data-theme="dark"]) .auxiliary-state {
    color: #a9abb2;
    background: #1c1c1e;
  }

  :global([data-theme="dark"]) .auxiliary-state strong {
    color: #f5f5f7;
  }

  @keyframes auxiliary-spin {
    to { transform: rotate(360deg); }
  }
</style>
