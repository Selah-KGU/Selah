<script lang="ts">
  import { tick } from "svelte";
  import type { LiveWhiteboard } from "./api";
  import {
    computeWhiteboardLayout,
    prepareWhiteboardLayout,
    whiteboardLayoutReady,
    whiteboardTopics,
    type WhiteboardLayoutResult,
  } from "./whiteboardLayout";

  interface Props {
    board: LiveWhiteboard;
  }

  let { board }: Props = $props();
  let selectedTopicIds = $state<string[]>([]);
  let topicsCollapsed = $state(false);
  let fullscreen = $state(false);
  let selectedNodeId = $state<string | null>(null);
  let zoom = $state(0.8);
  let panX = $state(0);
  let panY = $state(0);
  let canvasWidth = $state(0);
  let canvasHeight = $state(0);
  let fitFingerprint = $state("");
  let drag = $state<{ x: number; y: number; panX: number; panY: number } | null>(null);
  let dragged = false;

  $effect(() => {
    if (!board) return;
    void prepareWhiteboardLayout().catch((error) => {
      console.warn("[MarkdownWhiteboard] layout failed to load:", error);
    });
  });

  const topics = $derived.by(() => {
    void $whiteboardLayoutReady;
    return whiteboardTopics(board);
  });
  const topicFingerprint = $derived(topics.map((topic) => topic.id).join("|"));
  const layout = $derived.by((): WhiteboardLayoutResult | null => {
    void $whiteboardLayoutReady;
    return computeWhiteboardLayout(board, {
      fallbackBoardTitle: "知識整理ボード",
      externalNodeLabel: "外部",
      topicIds: topics.length > 1 && selectedTopicIds.length ? selectedTopicIds : undefined,
    });
  });
  const stage = $derived(layout?.stage || { width: 1040, height: 660 });
  const allTopicsSelected = $derived(topics.length > 0 && selectedTopicIds.length >= topics.length);
  const highlighted = $derived.by(() => {
    if (!layout || !selectedNodeId) return null;
    const nodes = new Set<string>([selectedNodeId]);
    const edges = new Set<string>();
    for (const edge of layout.edges) {
      if (edge.from === selectedNodeId) {
        nodes.add(edge.to);
        edges.add(edge.id);
      } else if (edge.to === selectedNodeId) {
        nodes.add(edge.from);
        edges.add(edge.id);
      }
    }
    return { nodes, edges };
  });

  $effect(() => {
    topicFingerprint;
    const ids = topics.map((topic) => topic.id);
    const kept = selectedTopicIds.filter((id) => ids.includes(id));
    const next = kept.length ? kept : ids.slice(0, 1);
    const unchanged = next.length === selectedTopicIds.length
      && next.every((id, index) => id === selectedTopicIds[index]);
    if (!unchanged) selectedTopicIds = next;
  });

  $effect(() => {
    const fingerprint = `${stage.width}x${stage.height}:${canvasWidth}x${canvasHeight}:${selectedTopicIds.join("|")}`;
    if (!canvasWidth || !canvasHeight || fingerprint === fitFingerprint) return;
    fitFingerprint = fingerprint;
    zoom = Math.min(1, Math.max(0.06, Math.min((canvasWidth - 28) / stage.width, (canvasHeight - 28) / stage.height)));
    panX = 0;
    panY = 0;
  });

  function setZoom(value: number): void {
    zoom = Math.max(0.06, Math.min(2.2, value));
  }

  async function resetViewport(): Promise<void> {
    fitFingerprint = "";
    panX = 0;
    panY = 0;
    await tick();
    fitFingerprint = "";
  }

  async function toggleFullscreen(): Promise<void> {
    fullscreen = !fullscreen;
    await resetViewport();
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (fullscreen && event.key === "Escape") {
      event.preventDefault();
      void toggleFullscreen();
    }
  }

  function toggleTopic(id: string): void {
    if (selectedTopicIds.includes(id)) {
      if (selectedTopicIds.length > 1) selectedTopicIds = selectedTopicIds.filter((item) => item !== id);
    } else {
      selectedTopicIds = [...selectedTopicIds, id];
    }
    selectedNodeId = null;
    fitFingerprint = "";
  }

  function toggleAllTopics(): void {
    const ids = topics.map((topic) => topic.id);
    selectedTopicIds = allTopicsSelected ? ids.slice(0, 1) : ids;
    selectedNodeId = null;
    fitFingerprint = "";
  }

  function handleWheel(event: WheelEvent): void {
    event.preventDefault();
    setZoom(zoom + (event.deltaY < 0 ? 0.1 : -0.1));
  }

  function handlePointerDown(event: PointerEvent): void {
    if (event.button !== 0 || (event.target as Element)?.closest(".wb-node,button")) return;
    drag = { x: event.clientX, y: event.clientY, panX, panY };
    dragged = false;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function handlePointerMove(event: PointerEvent): void {
    if (!drag) return;
    const dx = event.clientX - drag.x;
    const dy = event.clientY - drag.y;
    if (Math.abs(dx) + Math.abs(dy) > 4) dragged = true;
    panX = drag.panX + dx;
    panY = drag.panY + dy;
  }

  function handlePointerUp(event: PointerEvent): void {
    drag = null;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
  }

  function selectNode(id: string, event: MouseEvent | KeyboardEvent): void {
    event.stopPropagation();
    selectedNodeId = selectedNodeId === id ? null : id;
  }

  function handleNodeKeydown(id: string, event: KeyboardEvent): void {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    selectNode(id, event);
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if layout}
  <section class:fullscreen class="whiteboard" aria-label={layout.title || "知識整理ボード"}>
    {#if topics.length > 1}
      <aside class="topics" class:collapsed={topicsCollapsed}>
        <div class="topic-head">
          <button
            class="topic-fold"
            type="button"
            aria-label={topicsCollapsed ? "展開" : "折りたたむ"}
            title={topicsCollapsed ? "展開" : "折りたたむ"}
            onclick={() => topicsCollapsed = !topicsCollapsed}
          >
            {#if topicsCollapsed}
              <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
                <line x1="4" y1="7" x2="20" y2="7" />
                <line x1="4" y1="12" x2="20" y2="12" />
                <line x1="4" y1="17" x2="20" y2="17" />
              </svg>
            {:else}
              <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
                <polyline points="11 7 6 12 11 17" />
                <polyline points="18 7 13 12 18 17" />
              </svg>
              <span>折りたたむ</span>
            {/if}
          </button>
          {#if !topicsCollapsed}
            <button class="topic-all" type="button" onclick={toggleAllTopics}>
              <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
                <rect x="3.5" y="3.5" width="17" height="17" rx="4" />
                {#if allTopicsSelected}
                  <path d="M8.2 12h7.6" />
                {:else}
                  <path d="M8 12.4l2.9 2.9L16.5 9" />
                {/if}
              </svg>
              <span>{allTopicsSelected ? "選択解除" : "すべて選択"}</span>
            </button>
          {/if}
        </div>
        {#if !topicsCollapsed}
          <div class="topic-list">
            {#each topics as topic (topic.id)}
              <button class:active={selectedTopicIds.includes(topic.id)} type="button" onclick={() => toggleTopic(topic.id)}>
                {topic.label}
              </button>
            {/each}
          </div>
        {/if}
      </aside>
    {/if}

    <div class="zoom-controls">
      <button class="zoom-step" type="button" title="縮小" aria-label="縮小" onclick={() => setZoom(zoom - 0.15)}>−</button>
      <button class="zoom-value" type="button" title="全体を表示" onclick={resetViewport}>{Math.round(zoom * 100)}%</button>
      <button class="zoom-step" type="button" title="拡大" aria-label="拡大" onclick={() => setZoom(zoom + 0.15)}>＋</button>
      <span class="control-separator" aria-hidden="true"></span>
      <button
        class="fullscreen-toggle"
        type="button"
        title={fullscreen ? "全画面表示を終了" : "全画面表示"}
        aria-label={fullscreen ? "全画面表示を終了" : "全画面表示"}
        aria-pressed={fullscreen}
        onclick={toggleFullscreen}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
          {#if fullscreen}
            <polyline points="9 3 9 9 3 9" />
            <line x1="9" y1="9" x2="3" y2="3" />
            <polyline points="15 21 15 15 21 15" />
            <line x1="15" y1="15" x2="21" y2="21" />
          {:else}
            <polyline points="9 3 3 3 3 9" />
            <line x1="3" y1="3" x2="9" y2="9" />
            <polyline points="15 21 21 21 21 15" />
            <line x1="21" y1="21" x2="15" y2="15" />
          {/if}
        </svg>
      </button>
    </div>

    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="canvas"
      class:dragging={!!drag}
      class:has-selection={selectedNodeId !== null}
      role="application"
      aria-label={layout.title || "知識整理ボード"}
      tabindex="0"
      bind:clientWidth={canvasWidth}
      bind:clientHeight={canvasHeight}
      onwheel={handleWheel}
      onpointerdown={handlePointerDown}
      onpointermove={handlePointerMove}
      onpointerup={handlePointerUp}
      onpointercancel={handlePointerUp}
      onclick={() => {
        if (!dragged) selectedNodeId = null;
        dragged = false;
      }}
    >
      <div
        class="stage"
        style={`width:${stage.width}px;height:${stage.height}px;transform:translate(-50%,-50%) translate(${panX}px,${panY}px) scale(${zoom});`}
      >
        <svg class="links" viewBox={`0 0 ${stage.width} ${stage.height}`} preserveAspectRatio="none" aria-hidden="true">
          {#each layout.edges as edge (edge.id)}
            <path
              class:trunk={edge.trunk}
              class:redundant={edge.redundant}
              class:highlighted={highlighted?.edges.has(edge.id)}
              class="edge kind-{edge.colorKind} source-{edge.colorSourceType}"
              d={`M ${edge.x1} ${edge.y1} Q ${edge.cx} ${edge.cy} ${edge.x2} ${edge.y2}`}
            />
          {/each}
        </svg>
        {#each layout.edges as edge (`${edge.id}-label`)}
          {#if edge.label}
            <span
              class:highlighted={highlighted?.edges.has(edge.id)}
              class="edge-label kind-{edge.colorKind} source-{edge.colorSourceType}"
              style={`left:${edge.lx}%;top:${edge.ly}%;`}
            >{edge.label}</span>
          {/if}
        {/each}
        {#each layout.nodes as node (node.id)}
          <div
            class:highlighted={highlighted?.nodes.has(node.id)}
            class:selected={selectedNodeId === node.id}
            class:main={node.role === "main"}
            class:branch={node.role === "branch"}
            class="wb-node kind-{node.kind} source-{node.sourceType}"
            style={`left:${node.x}%;top:${node.y}%;`}
            role="button"
            tabindex="0"
            title={node.sourceType === "external" ? `外部: ${node.sourceLabel}` : ""}
            onclick={(event) => selectNode(node.id, event)}
            onkeydown={(event) => handleNodeKeydown(node.id, event)}
          >
            {#if node.sourceType === "external"}<span class="source-badge">外部</span>{/if}
            <strong>{node.label}</strong>
            {#if node.detail}<span class="detail">{node.detail}</span>{/if}
            {#if node.chips?.length}
              <span class="chips">
                {#each node.chips as chip}
                  <span title={chip.detail}>{chip.label}</span>
                {/each}
              </span>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </section>
{/if}

<style>
  .whiteboard {
    position: relative;
    margin: 14px 0 10px;
    padding: 10px;
    border: 0.5px solid color-mix(in srgb, var(--reader-accent) 20%, var(--reader-border));
    border-radius: 12px;
    background: color-mix(in srgb, var(--reader-bg) 92%, transparent);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.06);
  }

  .whiteboard.fullscreen {
    position: fixed;
    inset: 0;
    z-index: 1000;
    box-sizing: border-box;
    width: 100vw;
    height: 100vh;
    margin: 0;
    padding: 10px;
    overflow: hidden;
    border: 0;
    border-radius: 0;
    background: var(--reader-bg);
    box-shadow: none;
  }

  .canvas {
    position: relative;
    width: 100%;
    height: 390px;
    overflow: hidden;
    border-radius: 8px;
    background:
      linear-gradient(color-mix(in srgb, var(--reader-faint) 9%, transparent) 1px, transparent 1px),
      linear-gradient(90deg, color-mix(in srgb, var(--reader-faint) 9%, transparent) 1px, transparent 1px),
      color-mix(in srgb, var(--reader-sidebar) 70%, transparent);
    background-size: 22px 22px;
    cursor: grab;
    touch-action: none;
  }

  .whiteboard.fullscreen .canvas {
    height: 100%;
    border-radius: 0;
  }

  .canvas.dragging { cursor: grabbing; }
  .stage { position: absolute; left: 50%; top: 50%; transform-origin: 50% 50%; }
  .links { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; color: color-mix(in srgb, #007aff 52%, var(--reader-faint)); }
  .edge { fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; opacity: 0.32; }
  .edge.trunk { stroke-width: 1.8; opacity: 0.7; }
  .edge.redundant { stroke-dasharray: 4 4; opacity: 0.2; }
  .edge.source-external { stroke-dasharray: 5 4; }
  .edge.kind-core,
  .edge.kind-support { stroke: color-mix(in srgb, var(--reader-faint) 76%, var(--reader-muted)); }
  .edge.kind-result { stroke: color-mix(in srgb, #34c759 62%, var(--reader-faint)); }
  .edge.kind-question { stroke: color-mix(in srgb, #ff9500 64%, var(--reader-faint)); }

  .edge-label {
    position: absolute;
    z-index: 2;
    max-width: 132px;
    overflow: hidden;
    transform: translate(-50%, -50%);
    padding: 2px 8px;
    border: 0.5px solid color-mix(in srgb, #007aff 24%, transparent);
    border-radius: 999px;
    color: color-mix(in srgb, #007aff 78%, var(--reader-muted));
    background: color-mix(in srgb, var(--reader-bg) 96%, transparent);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 10px;
    font-weight: 800;
    line-height: 1.15;
    text-align: center;
  }
  .edge-label.kind-core,
  .edge-label.kind-support {
    border-color: color-mix(in srgb, var(--reader-faint) 30%, transparent);
    color: color-mix(in srgb, var(--reader-faint) 82%, var(--reader-muted));
  }
  .edge-label.kind-result {
    border-color: color-mix(in srgb, #34c759 30%, transparent);
    color: color-mix(in srgb, #34c759 76%, var(--reader-muted));
  }
  .edge-label.kind-question {
    border-color: color-mix(in srgb, #ff9500 32%, transparent);
    color: color-mix(in srgb, #ff9500 76%, var(--reader-muted));
  }
  .edge-label.source-external { border-style: dashed; }

  .wb-node {
    position: absolute;
    z-index: 3;
    width: 124px;
    min-height: 66px;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    padding: 8px 10px;
    border: 0.5px solid color-mix(in srgb, #007aff 24%, var(--reader-border));
    border-radius: 8px;
    color: var(--reader-text);
    background: color-mix(in srgb, var(--reader-bg) 96%, transparent);
    box-shadow: 0 3px 8px rgba(0, 0, 0, 0.08);
    text-align: center;
    cursor: pointer;
  }

  .wb-node.main {
    width: 144px;
    min-height: 74px;
    border-width: 1px;
    box-shadow: 0 5px 14px rgba(33, 116, 223, 0.14);
  }
  .wb-node.branch { width: 116px; min-height: 62px; opacity: 0.94; }
  .wb-node.kind-core {
    border-color: color-mix(in srgb, #007aff 38%, var(--reader-border));
    background: color-mix(in srgb, #007aff 14%, var(--reader-bg));
  }
  .wb-node.kind-result { background: color-mix(in srgb, #34c759 13%, var(--reader-bg)); border-color: color-mix(in srgb, #34c759 34%, var(--reader-border)); }
  .wb-node.kind-question { background: color-mix(in srgb, #ff9500 13%, var(--reader-bg)); border-color: color-mix(in srgb, #ff9500 32%, var(--reader-border)); }
  .wb-node.source-external {
    border-style: dashed;
    border-color: color-mix(in srgb, var(--reader-accent) 38%, var(--reader-border));
    background:
      linear-gradient(135deg, color-mix(in srgb, var(--reader-accent) 9%, transparent), transparent 52%),
      color-mix(in srgb, var(--reader-bg) 96%, transparent);
  }
  .wb-node strong { max-width: 100%; overflow-wrap: anywhere; font-size: 12px; font-weight: 800; line-height: 1.25; }
  .detail { max-width: 100%; overflow: hidden; overflow-wrap: anywhere; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 4; line-clamp: 4; color: var(--reader-muted); font-size: 10.5px; font-weight: 600; line-height: 1.25; }
  .chips { max-width: 100%; display: flex; flex-wrap: wrap; justify-content: center; gap: 3px; margin-top: 4px; }
  .chips span { max-width: 100%; overflow: hidden; padding: 1px 6px; border-radius: 999px; color: var(--reader-muted); background: color-mix(in srgb, var(--reader-faint) 15%, transparent); text-overflow: ellipsis; white-space: nowrap; font-size: 8.5px; font-weight: 700; line-height: 1.55; }
  .source-badge { position: absolute; top: -7px; right: -7px; max-width: 44px; overflow: hidden; padding: 1px 6px; border: 0.5px solid color-mix(in srgb, var(--reader-accent) 36%, var(--reader-border)); border-radius: 999px; color: color-mix(in srgb, var(--reader-accent) 82%, var(--reader-muted)); background: color-mix(in srgb, var(--reader-bg) 96%, transparent); text-overflow: ellipsis; white-space: nowrap; pointer-events: none; font-size: 9px; font-weight: 800; line-height: 1.35; }

  .canvas.has-selection .edge,
  .canvas.has-selection .edge-label,
  .canvas.has-selection .wb-node {
    transition: opacity 0.16s ease, box-shadow 0.16s ease, border-color 0.16s ease;
  }
  .canvas.has-selection .edge { opacity: 0.08; }
  .canvas.has-selection .edge.highlighted { opacity: 0.96; stroke-width: 2.6; }
  .canvas.has-selection .edge-label { opacity: 0.14; }
  .canvas.has-selection .edge-label.highlighted,
  .canvas.has-selection .wb-node.highlighted,
  .canvas.has-selection .wb-node.selected { opacity: 1; }
  .canvas.has-selection .wb-node { opacity: 0.24; }
  .wb-node.selected { box-shadow: 0 0 0 2px color-mix(in srgb, #007aff 65%, transparent), 0 6px 16px rgba(33, 116, 223, 0.22); }

  .topics,
  .zoom-controls {
    position: absolute;
    z-index: 8;
    box-sizing: border-box;
    border: 0.5px solid var(--reader-border);
    background: color-mix(in srgb, var(--reader-bg) 78%, transparent);
    backdrop-filter: blur(22px) saturate(1.5);
    -webkit-backdrop-filter: blur(22px) saturate(1.5);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
  }

  .topics { left: 18px; top: 18px; width: 220px; max-height: calc(100% - 56px); display: flex; flex-direction: column; gap: 6px; overflow-y: auto; padding: 7px; border-radius: 12px; }
  .topics.collapsed { width: auto; padding: 5px; }
  .topic-head { display: flex; align-items: center; gap: 5px; }
  .topics.collapsed .topic-head { justify-content: center; }
  .topic-head button,
  .topic-list button,
  .zoom-controls button {
    border: 0.5px solid transparent;
    color: var(--reader-muted);
    background: transparent;
    font: inherit;
    cursor: pointer;
  }
  .topic-head button { box-sizing: border-box; min-width: 32px; height: 30px; flex: 1 1 0; display: inline-flex; align-items: center; justify-content: center; gap: 5px; padding: 0 8px; border-radius: 8px; white-space: nowrap; font-size: 10.5px; font-weight: 700; transition: background 0.14s, color 0.14s; }
  .topic-head button svg { flex: 0 0 auto; fill: none; stroke: currentColor; stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; }
  .topic-head button span { overflow: hidden; text-overflow: ellipsis; }
  .topic-list { display: flex; flex-direction: column; gap: 5px; }
  .topic-list button { box-sizing: border-box; width: 100%; padding: 7px 10px; border-radius: 8px; background: color-mix(in srgb, var(--reader-text) 5%, transparent); text-align: left; font-size: 11.5px; font-weight: 700; line-height: 1.3; transition: background 0.14s, color 0.14s, border-color 0.14s; }
  .topic-list button.active { border-color: color-mix(in srgb, var(--reader-accent) 45%, transparent); color: var(--reader-text); background: color-mix(in srgb, var(--reader-accent) 13%, transparent); font-weight: 800; }
  .topic-head button:hover,
  .topic-list button:hover,
  .zoom-controls button:hover { color: var(--reader-text); background: color-mix(in srgb, var(--reader-text) 9%, transparent); }
  .topic-list button.active:hover { color: var(--reader-text); background: color-mix(in srgb, var(--reader-accent) 18%, transparent); }

  .zoom-controls { top: 20px; right: 20px; display: inline-flex; align-items: center; gap: 2px; padding: 4px; border-radius: 14px; }
  .zoom-controls button { min-width: 34px; height: 30px; display: inline-flex; align-items: center; justify-content: center; padding: 0 10px; border: 0; border-radius: 10px; font-size: 13px; font-weight: 700; transition: background 0.15s, color 0.15s; }
  .control-separator { width: 0.5px; height: 16px; margin: 0 1px; background: var(--reader-border); }
  .zoom-controls .fullscreen-toggle { width: 30px; min-width: 30px; padding: 0; }
  .fullscreen-toggle svg { fill: none; stroke: currentColor; stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; }

  @media (max-width: 700px) {
    .canvas { height: 320px; }
    .topics { width: 180px; }
    .wb-node { width: 108px; min-height: 58px; }
    .whiteboard.fullscreen .canvas { height: 100%; }
  }
</style>
