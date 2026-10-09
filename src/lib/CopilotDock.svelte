<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import Icon from "./Icon.svelte";
  import { tabFaviconUrl, type DocumentTabSummary as DocumentTab } from "./documentTabs";
  import { acquireResourceGroup, ResourceScope } from "./resourceScope";
  import { LatestViewRead } from "./latestViewRead";
  import { createCacheSyncQueue } from "./cacheSyncQueue";

  const OWNER = "document-tabs";

  let tabs = $state<DocumentTab[]>([]);
  let busy = $state(false);

  // Cap the dock list; the full set stays reachable inside the Copilot window.
  const MAX_VISIBLE = 4;
  const visibleTabs = $derived(tabs.slice(0, MAX_VISIBLE));
  const resources = new ResourceScope();
  let subscribing = false;
  let subscribed = false;
  let faviconFailed = $state<Set<string>>(new Set());

  // Pause the continuously-repainting flowing border while the window is hidden
  // (minimized / app in background) to avoid burning CPU/battery off-screen.
  let docHidden = $state(typeof document !== "undefined" && document.hidden);
  function onVisibility(): void {
    const wasHidden = docHidden;
    docHidden = document.hidden;
    if (wasHidden && !docHidden) {
      if (subscribed) void refresh();
      else startDock();
    }
  }

  function onFaviconError(src: string): void {
    if (src && !faviconFailed.has(src)) faviconFailed = new Set(faviconFailed).add(src);
  }

  function shortTitle(tab: DocumentTab): string {
    const s = (tab.title || tab.url || tab.type || "タブ").trim();
    return s.length > 26 ? `${s.slice(0, 25)}…` : s;
  }

  const tabsRead = new LatestViewRead(resources,
    () => invoke<DocumentTab[]>("document_tabs_list", { owner: OWNER }),
    (next) => { tabs = next; });
  const queueRead = createCacheSyncQueue(async () => {
    // A missing/not-yet-created window does not clear the latest pushed state.
    await tabsRead.refresh().catch(() => {});
  });
  function refresh(): Promise<void> {
    if (!resources.active) return Promise.resolve();
    return queueRead(["tabs"]);
  }

  async function run(action: () => Promise<unknown>): Promise<void> {
    if (!resources.active || busy) return;
    busy = true;
    tabsRead.invalidate();
    try {
      await action();
    } catch (e) {
      if (resources.active) console.warn("[Copilot dock]", e);
    } finally {
      if (resources.active) busy = false;
    }
  }

  function newTab(): void {
    // A fresh tab also shows/focuses the Copilot window (ensure_window).
    void run(() => invoke("document_tabs_new_tab"));
  }

  function revealWindow(): void {
    void run(() => invoke("document_tabs_reveal", { owner: OWNER }));
  }

  function revealTab(id: string): void {
    void run(() => invoke("document_tabs_reveal", { owner: OWNER, id }));
  }

  function closeTab(id: string): void {
    void run(() => invoke("document_tabs_close", { owner: OWNER, id, focus: false }));
  }

  async function initializeDock(): Promise<void> {
    if (!resources.active || subscribing || subscribed) return;
    subscribing = true;
    try {
      await acquireResourceGroup(resources, [
        (group) => listen<{ owner: string; tabs: DocumentTab[] }>("document-tabs-changed", group.guard((event) => {
          if (event.payload?.owner !== OWNER) return;
          tabsRead.invalidate();
          tabs = event.payload.tabs || [];
        })),
      ]);
      if (!resources.active) return;
      subscribed = true;
      await refresh();
    } finally {
      subscribing = false;
    }
  }

  function startDock(): void {
    void initializeDock().catch((e) => {
      if (resources.active) console.warn("[Copilot dock] subscription failed:", e);
    });
  }

  onMount(() => {
    const visibility = resources.guard(onVisibility);
    document.addEventListener("visibilitychange", visibility);
    resources.own(() => document.removeEventListener("visibilitychange", visibility));
    startDock();
  });
  onDestroy(() => resources.dispose());
</script>

<div class="copilot-dock" class:paused={docHidden}>
  <div class="cd-header">
    <span class="cd-logo" aria-hidden="true"><Icon name="copilot" size={19} /></span>
    <span class="cd-title">Copilot</span>
    <button class="cd-new" type="button" onclick={newTab} title="新しいタブ" aria-label="新しいタブ" disabled={busy}>
      <Icon name="plus" size={14} />
    </button>
  </div>

  {#if tabs.length}
    <div class="cd-tabs">
      {#each visibleTabs as tab (tab.id)}
        {@const fav = tab.type === "browser" ? tabFaviconUrl(tab.url) : ""}
        <div class="cd-tab-row" class:active={tab.active}>
          <button
            class="cd-tab"
            type="button"
            title={tab.title || tab.url}
            disabled={busy}
            onclick={() => revealTab(tab.id)}
          >
            {#if tab.loading}
              <span class="cd-ico cd-spinner" aria-hidden="true"></span>
            {:else if tab.type === "detective"}
              <img class="cd-ico cd-img" src="/naruhodo.png" alt="" aria-hidden="true" draggable="false" />
            {:else if fav && !faviconFailed.has(fav)}
              <img class="cd-ico cd-img" src={fav} alt="" aria-hidden="true" draggable="false" onerror={() => onFaviconError(fav)} />
            {:else}
              <span class="cd-ico cd-dot" data-kind={tab.type} aria-hidden="true"></span>
            {/if}
            <span class="cd-tab-title">{shortTitle(tab)}</span>
          </button>
          <button
            class="cd-close"
            type="button"
            title="タブを閉じる"
            aria-label={`${shortTitle(tab)}を閉じる`}
            disabled={busy}
            onclick={() => closeTab(tab.id)}
          >
            <Icon name="xmark" size={11} />
          </button>
        </div>
      {/each}
    </div>
  {/if}

  {#if tabs.length > MAX_VISIBLE}
    <button class="cd-more" type="button" onclick={revealWindow} disabled={busy} title="Copilot ですべてのタブを表示">
      他に {tabs.length - MAX_VISIBLE} 件のタブ
    </button>
  {/if}
</div>

<style>
  /* Animated angle drives the flowing conic-gradient border. */
  @property --cd-angle {
    syntax: "<angle>";
    inherits: false;
    initial-value: 0deg;
  }

  /* Inset glass module docked at the sidebar foot. */
  .copilot-dock {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 5px 8px 8px;
    padding: 6px;
    border-radius: 12px;
    border: 0.5px solid var(--glass-border);
    background: var(--bg-card);
    box-shadow:
      0 4px 16px rgba(0, 0, 0, 0.08),
      inset 0 0.5px 0 rgba(255, 255, 255, 0.45);
  }

  @keyframes cd-rotate {
    to { --cd-angle: 360deg; }
  }

  .cd-header {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 4px 0 6px;
  }

  .cd-logo {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    color: var(--text-primary);
  }

  .cd-title {
    flex: 1 1 auto;
    min-width: 0;
    text-align: left;
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.2px;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cd-more {
    align-self: stretch;
    margin-top: 2px;
    padding: 5px 8px;
    border: none;
    border-radius: 7px;
    background: transparent;
    color: var(--text-tertiary);
    font-size: 11.5px;
    font-weight: 600;
    text-align: center;
    cursor: pointer;
    transition: background 0.14s ease, color 0.14s ease;
  }

  .cd-more:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--accent);
  }

  .cd-more:disabled {
    cursor: default;
  }

  .cd-new {
    position: relative;
    flex: 0 0 auto;
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    transition: background 0.14s ease, color 0.14s ease, transform 0.1s ease;
  }

  /* Flowing AI-gradient border lives on the add button. */
  .cd-new::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    padding: 1.5px;
    background: conic-gradient(
      from var(--cd-angle),
      #af52de,
      #007aff,
      color-mix(in srgb, #007aff 55%, #ffffff),
      #007aff,
      #af52de
    );
    -webkit-mask:
      linear-gradient(#000 0 0) content-box,
      linear-gradient(#000 0 0);
    mask:
      linear-gradient(#000 0 0) content-box,
      linear-gradient(#000 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
    pointer-events: none;
    animation: cd-rotate 6s linear infinite;
  }

  /* Off-screen: stop the per-frame gradient repaint. */
  .copilot-dock.paused .cd-new::before {
    animation-play-state: paused;
  }

  .cd-new :global(.icon) {
    display: block;
    position: relative;
    z-index: 1;
  }

  .cd-new:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-primary) 8%, transparent);
    color: var(--text-primary);
  }

  .cd-new:active:not(:disabled) {
    transform: scale(0.9);
  }

  .cd-new:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .cd-tabs {
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: 168px;
    overflow-y: auto;
    scrollbar-width: thin;
  }

  .cd-tab-row {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    height: 27px;
    border: none;
    border-radius: 7px;
    background: transparent;
    color: var(--text-secondary);
    transition: background 0.15s ease, color 0.15s ease;
  }

  .cd-tab-row:hover,
  .cd-tab-row:focus-within {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
    color: var(--text-primary);
  }

  .cd-tab-row.active {
    background: color-mix(in srgb, var(--text-primary) 10%, transparent);
    color: var(--text-primary);
    box-shadow: inset 0 0 0 0.5px color-mix(in srgb, var(--text-primary) 18%, transparent);
  }

  .cd-tab-row.active .cd-tab-title {
    font-weight: 650;
  }

  .cd-tab {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    padding: 0 30px 0 10px;
    border: none;
    border-radius: inherit;
    background: transparent;
    color: inherit;
    cursor: pointer;
    text-align: left;
  }

  .cd-tab:disabled {
    cursor: default;
  }

  .cd-close {
    position: absolute;
    right: 4px;
    top: 50%;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    padding: 0;
    border: none;
    border-radius: 5px;
    background: color-mix(in srgb, var(--bg-card) 88%, transparent);
    color: var(--text-tertiary);
    cursor: pointer;
    opacity: 0;
    pointer-events: none;
    transform: translateY(-50%) scale(0.88);
    transition: opacity 0.12s ease, transform 0.12s ease, background 0.12s ease, color 0.12s ease;
  }

  .cd-tab-row:hover .cd-close,
  .cd-close:focus-visible {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(-50%) scale(1);
  }

  .cd-close:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-primary) 12%, var(--bg-card));
    color: var(--text-primary);
  }

  .cd-close:disabled {
    cursor: default;
  }

  .cd-ico {
    flex: 0 0 auto;
    width: 15px;
    height: 15px;
  }

  .cd-img {
    border-radius: 4px;
    object-fit: cover;
    box-shadow: 0 0 0 0.5px rgba(0, 0, 0, 0.06);
  }

  .cd-dot {
    width: 8px;
    height: 8px;
    margin: 0 3.5px;
    border-radius: 50%;
    background: #64748b;
    box-shadow: 0 0 0 3px color-mix(in srgb, #64748b 16%, transparent);
  }

  .cd-dot[data-kind="reader"] { background: #0f766e; box-shadow: 0 0 0 3px rgba(15,118,110,0.16); }
  .cd-dot[data-kind="browser"] { background: #2563eb; box-shadow: 0 0 0 3px rgba(37,99,235,0.16); }
  .cd-dot[data-kind="kwic"] { background: #b45309; box-shadow: 0 0 0 3px rgba(180,83,9,0.16); }
  .cd-dot[data-kind="kgc"] { background: #7c3aed; box-shadow: 0 0 0 3px rgba(124,58,237,0.16); }
  .cd-dot[data-kind="files"] { background: #0891b2; box-shadow: 0 0 0 3px rgba(8,145,178,0.16); }
  .cd-dot[data-kind="home"] { background: #475569; box-shadow: 0 0 0 3px rgba(71,85,105,0.16); }
  .cd-dot[data-kind="detective"] { background: #b91c1c; box-shadow: 0 0 0 3px rgba(185,28,28,0.16); }
  .cd-dot[data-kind="paper-check"] { background: #173b68; box-shadow: 0 0 0 3px rgba(23,59,104,0.16); }

  .cd-tab-title {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
  }

  .cd-spinner {
    width: 12px;
    height: 12px;
    margin: 1.5px;
    border-radius: 50%;
    border: 1.6px solid var(--border-strong, rgba(0,0,0,0.18));
    border-top-color: var(--accent);
    animation: cd-spin 0.7s linear infinite;
  }

  @keyframes cd-spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .cd-new::before { animation: none; }
  }
</style>
