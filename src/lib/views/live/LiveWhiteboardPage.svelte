<script lang="ts">
  import type { WhiteboardLayoutResult } from "../../whiteboardLayout";
  import type { BoardHighlight, TermFloatLabels, WhiteboardStagePreset } from "./liveTypes";

  type WhiteboardDragStart = { x: number; y: number; panX: number; panY: number } | null;

  interface Props {
    activeWhiteboardLayout: WhiteboardLayoutResult;
    activeWhiteboardStage: WhiteboardStagePreset;
    termFloatLabels: TermFloatLabels;
    whiteboardZoom: number;
    whiteboardPanX: number;
    whiteboardPanY: number;
    whiteboardDragStart: WhiteboardDragStart;
    selectedBoardNodeId: string | null;
    boardHighlight: BoardHighlight;
    boardCanvasWidth: number;
    boardCanvasHeight: number;
    bindWhiteboardOverlayDismiss: (node: HTMLElement) => { destroy?: () => void } | void;
    onClose: () => void;
    onZoomOut: () => void;
    onResetZoom: () => void;
    onZoomIn: () => void;
    onWheel: (event: WheelEvent) => void;
    onPointerDown: (event: PointerEvent) => void;
    onPointerMove: (event: PointerEvent) => void;
    onPointerUp: (event: PointerEvent) => void;
    onClearSelection: () => void;
    onToggleNodeSelection: (id: string, event: MouseEvent | KeyboardEvent) => void;
  }

  let {
    activeWhiteboardLayout,
    activeWhiteboardStage,
    termFloatLabels,
    whiteboardZoom,
    whiteboardPanX,
    whiteboardPanY,
    whiteboardDragStart,
    selectedBoardNodeId,
    boardHighlight,
    boardCanvasWidth = $bindable(),
    boardCanvasHeight = $bindable(),
    bindWhiteboardOverlayDismiss,
    onClose,
    onZoomOut,
    onResetZoom,
    onZoomIn,
    onWheel,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onClearSelection,
    onToggleNodeSelection,
  }: Props = $props();

  function handleNodeKeydown(id: string, event: KeyboardEvent) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onToggleNodeSelection(id, event);
  }
</script>

<section
  class="board-page"
  class:dense={activeWhiteboardLayout.nodes.length > 8}
  class:very-dense={activeWhiteboardLayout.nodes.length > 14}
  use:bindWhiteboardOverlayDismiss
  aria-label={termFloatLabels.boardTitle}
>
  <button
    type="button"
    class="board-page-back"
    onclick={onClose}
    aria-label={termFloatLabels.collapse}
    title={termFloatLabels.collapse}
  >
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
  </button>
  <div class="board-zoom-controls" aria-label={termFloatLabels.boardTitle}>
    <button type="button" onclick={onZoomOut} title="Zoom out" aria-label="Zoom out">−</button>
    <button type="button" onclick={onResetZoom} title="Reset zoom" aria-label="Reset zoom">{Math.round(whiteboardZoom * 100)}%</button>
    <button type="button" onclick={onZoomIn} title="Zoom in" aria-label="Zoom in">＋</button>
  </div>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="visual-board-canvas"
    class:dragging={!!whiteboardDragStart}
    class:has-selection={selectedBoardNodeId !== null}
    role="application"
    aria-label={termFloatLabels.boardTitle}
    bind:clientWidth={boardCanvasWidth}
    bind:clientHeight={boardCanvasHeight}
    onwheel={onWheel}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onclick={onClearSelection}
  >
    <div
      class="visual-board-stage"
      style="width: {activeWhiteboardStage.width}px; height: {activeWhiteboardStage.height}px; transform: translate(-50%, -50%) translate({whiteboardPanX}px, {whiteboardPanY}px) scale({whiteboardZoom});"
    >
      <svg class="visual-board-links" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        {#each activeWhiteboardLayout.edges as edge (edge.id)}
          <path
            class="visual-board-edge edge-kind-{edge.colorKind} edge-source-{edge.colorSourceType}"
            class:redundant={edge.redundant}
            class:term-edge={edge.termEdge}
            class:is-highlighted={boardHighlight?.edges.has(edge.id)}
            d="M {edge.x1} {edge.y1} Q {edge.cx} {edge.cy} {edge.x2} {edge.y2}"
          />
        {/each}
      </svg>
      {#each activeWhiteboardLayout.edges as edge (edge.id + "-label")}
        {#if edge.label}
          <span
            class="visual-board-edge-label edge-kind-{edge.colorKind} edge-source-{edge.colorSourceType}"
            class:is-highlighted={boardHighlight?.edges.has(edge.id)}
            style="left: {edge.lx}%; top: {edge.ly}%;"
          >{edge.label}</span>
        {/if}
      {/each}
      {#each activeWhiteboardLayout.nodes as node (node.id)}
        <div
          class="visual-board-node kind-{node.kind} source-{node.sourceType}"
          class:role-main={node.role === "main"}
          class:role-branch={node.role !== "main"}
          class:node-term={node.nodeType === "term"}
          class:is-highlighted={boardHighlight?.nodes.has(node.id)}
          class:is-selected={selectedBoardNodeId === node.id}
          style="left: {node.x}%; top: {node.y}%;"
          title={node.sourceType === "external" ? `${termFloatLabels.externalSource}: ${node.sourceLabel}` : ""}
          onclick={(e) => onToggleNodeSelection(node.id, e)}
          onkeydown={(e) => handleNodeKeydown(node.id, e)}
          role="button"
          tabindex="0"
        >
          {#if node.sourceType === "external"}
            <span class="visual-board-source-badge">{termFloatLabels.externalNode}</span>
          {/if}
          <span class="visual-board-node-label">{node.label}</span>
          {#if node.detail}
            <span class="visual-board-node-detail">{node.detail}</span>
          {/if}
        </div>
      {/each}
    </div>
  </div>
</section>

<style>
  .board-page {
    position: absolute;
    inset: -24px;
    z-index: 60;
    padding: 0;
    background: var(--bg-primary);
    display: flex;
    flex-direction: column;
    animation: board-page-in 0.26s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .board-page .visual-board-canvas {
    flex: 1 1 auto;
    height: auto;
    min-height: 0;
    border-radius: 0;
  }
  .board-page-back {
    position: absolute;
    top: 20px;
    left: 20px;
    z-index: 5;
    width: 36px;
    height: 36px;
    padding: 0;
    border: 0.5px solid var(--glass-border);
    border-radius: 12px;
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    color: var(--text-secondary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    box-shadow: var(--shadow-glass), 0 4px 16px rgba(0, 0, 0, 0.06);
    transition: background 0.15s, color 0.15s, transform 0.15s;
  }
  .board-page-back:hover {
    background: color-mix(in srgb, var(--text-primary) 8%, var(--glass-bg, rgba(255, 255, 255, 0.5)));
    color: var(--text-primary);
  }
  .board-page-back:active {
    transform: scale(0.94);
  }
  .board-zoom-controls {
    position: absolute;
    top: 20px;
    right: 20px;
    z-index: 6;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: 14px;
    border: 0.5px solid var(--glass-border);
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    box-shadow: var(--shadow-glass), 0 4px 16px rgba(0, 0, 0, 0.06);
  }
  .board-zoom-controls button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 34px;
    height: 30px;
    padding: 0 10px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .board-zoom-controls button:hover {
    background: color-mix(in srgb, var(--text-primary) 8%, transparent);
    color: var(--text-primary);
  }

  @keyframes board-page-in {
    from { transform: translateY(10px); }
    to { transform: translateY(0); }
  }

  .visual-board-canvas {
    position: relative;
    height: 380px;
    border-radius: 8px;
    overflow: hidden;
    background:
      linear-gradient(color-mix(in srgb, var(--text-tertiary) 9%, transparent) 1px, transparent 1px),
      linear-gradient(90deg, color-mix(in srgb, var(--text-tertiary) 9%, transparent) 1px, transparent 1px),
      color-mix(in srgb, var(--bg-secondary) 72%, transparent);
    background-size: 22px 22px;
    cursor: grab;
    touch-action: none;
  }
  .visual-board-canvas.dragging {
    cursor: grabbing;
  }
  .visual-board-stage {
    position: absolute;
    left: 50%;
    top: 50%;
    transform-origin: 50% 50%;
  }
  .visual-board-links {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    color: color-mix(in srgb, var(--blue) 52%, var(--text-tertiary));
    overflow: visible;
  }
  .visual-board-links path {
    fill: none;
    stroke: var(--edge-color, currentColor);
    stroke-width: 0.75;
    stroke-linecap: round;
    opacity: 0.74;
  }
  .visual-board-links path.edge-kind-core {
    --edge-color: color-mix(in srgb, var(--text-tertiary) 76%, var(--text-secondary));
  }
  .visual-board-links path.edge-kind-result {
    --edge-color: color-mix(in srgb, #34c759 62%, var(--text-tertiary));
  }
  .visual-board-links path.edge-kind-question {
    --edge-color: color-mix(in srgb, var(--orange, #e67700) 64%, var(--text-tertiary));
  }
  .visual-board-links path.edge-kind-support {
    --edge-color: color-mix(in srgb, var(--text-tertiary) 76%, var(--text-secondary));
  }
  .visual-board-links path.edge-source-external {
    stroke-dasharray: 3 2.2;
    opacity: 0.64;
  }
  .visual-board-links path.term-edge {
    stroke-width: 0.95;
    opacity: 0.86;
    stroke-dasharray: none;
  }
  .visual-board-links path.redundant {
    stroke-dasharray: 1.6 1.4;
    opacity: 0.42;
  }
  .visual-board-canvas.has-selection .visual-board-links path,
  .visual-board-canvas.has-selection .visual-board-edge-label,
  .visual-board-canvas.has-selection .visual-board-node {
    transition: opacity 0.16s ease, box-shadow 0.16s ease, border-color 0.16s ease;
  }
  .visual-board-canvas.has-selection .visual-board-links path {
    opacity: 0.12;
  }
  .visual-board-canvas.has-selection .visual-board-links path.is-highlighted {
    opacity: 0.95;
    stroke-width: 1.1;
  }
  .visual-board-canvas.has-selection .visual-board-edge-label {
    opacity: 0.14;
  }
  .visual-board-canvas.has-selection .visual-board-edge-label.is-highlighted {
    opacity: 1;
  }
  .visual-board-canvas.has-selection .visual-board-node {
    opacity: 0.24;
  }
  .visual-board-canvas.has-selection .visual-board-node.is-highlighted {
    opacity: 1;
  }
  .visual-board-canvas.has-selection .visual-board-node.is-selected {
    opacity: 1;
    box-shadow:
      0 0 0 2px color-mix(in srgb, var(--blue) 65%, transparent),
      0 6px 16px rgba(33, 116, 223, 0.22);
  }
  .visual-board-node {
    cursor: pointer;
  }
  .visual-board-edge-label {
    position: absolute;
    transform: translate(-50%, -50%);
    transform-origin: 50% 50%;
    z-index: 2;
    max-width: 132px;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--bg-primary) 96%, transparent);
    border: 0.5px solid var(--edge-label-border, color-mix(in srgb, var(--blue) 24%, transparent));
    color: var(--edge-label-color, color-mix(in srgb, var(--blue) 78%, var(--text-secondary)));
    font-size: 10px;
    font-weight: 800;
    line-height: 1.15;
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .visual-board-edge-label.edge-kind-core {
    --edge-label-border: color-mix(in srgb, var(--text-tertiary) 30%, transparent);
    --edge-label-color: color-mix(in srgb, var(--text-tertiary) 82%, var(--text-secondary));
  }
  .visual-board-edge-label.edge-kind-result {
    --edge-label-border: color-mix(in srgb, #34c759 30%, transparent);
    --edge-label-color: color-mix(in srgb, #34c759 76%, var(--text-secondary));
  }
  .visual-board-edge-label.edge-kind-question {
    --edge-label-border: color-mix(in srgb, var(--orange, #e67700) 32%, transparent);
    --edge-label-color: color-mix(in srgb, var(--orange, #e67700) 76%, var(--text-secondary));
  }
  .visual-board-edge-label.edge-kind-support {
    --edge-label-border: color-mix(in srgb, var(--text-tertiary) 28%, transparent);
    --edge-label-color: color-mix(in srgb, var(--text-tertiary) 82%, var(--text-secondary));
  }
  .visual-board-edge-label.edge-source-external {
    border-style: dashed;
  }
  .visual-board-node {
    position: absolute;
    transform: translate(-50%, -50%);
    z-index: 3;
    width: 122px;
    min-height: 66px;
    padding: 8px 9px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    border-radius: 8px;
    border: 0.5px solid color-mix(in srgb, var(--blue) 22%, var(--glass-border));
    background: color-mix(in srgb, var(--bg-primary) 96%, transparent);
    box-shadow: 0 3px 8px rgba(0, 0, 0, 0.08);
    text-align: center;
  }
  .board-page.dense .visual-board-node {
    width: 106px;
    min-height: 58px;
    padding: 6px 7px;
    gap: 2px;
  }
  .board-page.very-dense .visual-board-node {
    width: 94px;
    min-height: 50px;
    padding: 5px 6px;
  }
  .visual-board-node.role-main {
    width: 142px;
    min-height: 74px;
    border-width: 1px;
    box-shadow: 0 5px 14px rgba(33, 116, 223, 0.14);
  }
  .visual-board-node.role-branch {
    width: 114px;
    min-height: 62px;
    opacity: 0.94;
  }
  .visual-board-node.node-term {
    width: auto;
    min-width: 64px;
    max-width: 96px;
    min-height: 0;
    padding: 5px 8px;
    gap: 0;
    border-radius: 999px;
    opacity: 0.86;
    box-shadow: 0 2px 5px rgba(0, 0, 0, 0.06);
  }
  .board-page.dense .visual-board-node.role-main {
    width: 124px;
    min-height: 66px;
  }
  .board-page.very-dense .visual-board-node.role-main {
    width: 110px;
    min-height: 58px;
  }
  .board-page.dense .visual-board-node.node-term,
  .board-page.very-dense .visual-board-node.node-term {
    min-width: 54px;
    max-width: 80px;
    padding: 4px 7px;
  }
  .visual-board-node.kind-core {
    background: color-mix(in srgb, var(--blue) 14%, var(--bg-primary));
    border-color: color-mix(in srgb, var(--blue) 38%, var(--glass-border));
  }
  .visual-board-node.kind-result {
    background: color-mix(in srgb, #34c759 13%, var(--bg-primary));
    border-color: color-mix(in srgb, #34c759 34%, var(--glass-border));
  }
  .visual-board-node.kind-question {
    background: color-mix(in srgb, var(--orange, #e67700) 13%, var(--bg-primary));
    border-color: color-mix(in srgb, var(--orange, #e67700) 32%, var(--glass-border));
  }
  .visual-board-node.source-external {
    border-style: dashed;
    border-color: color-mix(in srgb, var(--accent) 38%, var(--glass-border));
    background:
      linear-gradient(135deg, color-mix(in srgb, var(--accent) 9%, transparent), transparent 52%),
      color-mix(in srgb, var(--bg-primary) 96%, transparent);
  }
  .visual-board-source-badge {
    position: absolute;
    top: -7px;
    right: -7px;
    max-width: 44px;
    padding: 1px 5px;
    border-radius: 999px;
    border: 0.5px solid color-mix(in srgb, var(--accent) 34%, var(--glass-border));
    background: color-mix(in srgb, var(--bg-primary) 96%, transparent);
    color: color-mix(in srgb, var(--accent) 82%, var(--text-secondary));
    font-size: 8.5px;
    font-weight: 800;
    line-height: 1.35;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: none;
  }
  .visual-board-node-label {
    max-width: 100%;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 800;
    line-height: 1.25;
    overflow-wrap: anywhere;
  }
  .board-page.dense .visual-board-node-label {
    font-size: 10.8px;
  }
  .board-page.very-dense .visual-board-node-label {
    font-size: 10px;
  }
  .visual-board-node-detail {
    max-width: 100%;
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 600;
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .visual-board-node.node-term .visual-board-node-label {
    font-size: 10.5px;
    line-height: 1.18;
  }
  .visual-board-node.node-term .visual-board-node-detail {
    display: none;
  }
  .visual-board-node.node-term:is(:hover, :focus-visible, .is-selected) {
    max-width: 142px;
    padding: 7px 9px;
    border-radius: 8px;
    opacity: 0.96;
    z-index: 5;
  }
  .visual-board-node.node-term:is(:hover, :focus-visible, .is-selected) .visual-board-node-detail {
    display: -webkit-box;
    margin-top: 3px;
    font-size: 9px;
    -webkit-line-clamp: 3;
    line-clamp: 3;
  }
  .board-page.dense .visual-board-node-detail {
    font-size: 9px;
    -webkit-line-clamp: 3;
    line-clamp: 3;
  }
  .board-page.very-dense .visual-board-node-detail {
    display: -webkit-box;
    font-size: 8.5px;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  @media (max-width: 700px) {
    .board-page {
      padding: 52px 12px 12px;
    }
    .board-page-back {
      top: 10px;
      left: 10px;
    }
    .board-zoom-controls {
      top: 10px;
      right: 10px;
    }
    .visual-board-node {
      width: 108px;
      min-height: 58px;
    }
    .board-page.dense .visual-board-node {
      width: 94px;
      min-height: 50px;
    }
  }
</style>
