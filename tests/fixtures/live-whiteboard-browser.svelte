<script lang="ts">
  import { onMount, tick } from "svelte";
  import Whiteboard from "test-live-whiteboard";
  import MarkdownWhiteboard from "../../src/lib/MarkdownWhiteboard.svelte";
  import { expandLiveSurface } from "../../src/lib/liveBoardTransport";
  import { applyLiveSessionUpdate, mergeLiveSnapshot } from "../../src/lib/views/live/liveTranscript";
  import { applyLiveSessionNotification } from "../../src/lib/views/live/liveNotification";
  import { boardFixture } from "./whiteboard-layout-cases.mjs";
  import { duplicateBoard, reservedBoard, prototypeBoard, phantomBoard } from "./whiteboard-identity-cases.mjs";

  const board = (id: string, count = 12) => ({ ...boardFixture({ count, edges: count * 2 }), probe_id: id });
  let snapshot = $state.raw({ course: null, summaries: [{ whiteboard: board("initial") }], transcript_line_count: 0 });
  let component: any;
  let activeSummaryIdx = $state(0);
  let readerEnabled = $state(false);
  const readerBoard = $derived(readerEnabled ? snapshot.summaries[activeSummaryIdx]?.whiteboard : null);
  const settle = async () => {
    await tick();
    for (let i = 0; i < 2; i++) await new Promise(resolve => requestAnimationFrame(resolve));
    await tick();
  };
  const query = (selector: string) => document.querySelector<HTMLElement>(selector)!;

  onMount(() => { void run(); });
  async function run() {
    const probe = (window as any).__whiteboardProbe;
    const checks: string[] = [];
    const check = (condition: unknown, label: string) => { if (!condition) throw new Error(label); checks.push(label); };
    const calls = (id: string) => probe.calls().filter((call: any) => call.id === id);
    const read = () => component.inspect();
    const fitted = () => {
      const state = read();
      const expected = Math.max(0.05, Math.round(Math.min(state.width / state.stage.width, state.height / state.stage.height) * 0.94 * 100) / 100);
      return state.width > 0 && state.height > 0 && state.zoom === expected;
    };
    try {
      await settle();
      check(calls("initial").length === 1 && calls("initial")[0].topics.length === 0, "closed board computes only the full overview");
      check(!read().expanded && read().nodeIds.length === 0, "closed overlay has no selected-topic layout");
      check(read().selectedTopicIds.join() === "n0", "closed board still selects the first topic");
      const start = probe.calls().length;
      for (let i = 0; i < 100; i++) { snapshot = { ...snapshot, transcript_line_count: i + 1 }; await tick(); }
      check(probe.calls().length === start, "100 speech snapshots do not recompute either layout");
      for (let i = 0; i < 20; i++) {
        snapshot = { ...snapshot, summaries: [{ whiteboard: board(`closed-${i}`) }] }; await tick();
        check(calls(`closed-${i}`).length === 1 && calls(`closed-${i}`)[0].topics.length === 0, `closed summary ${i} computes no filtered forest`);
      }
      query(".whiteboard-probe-open").click();
      check(read().expanded && read().zoom === 0.8, "open reads the real stage preset before DOM measurement");
      await settle();
      check(calls("closed-19").length === 2 && calls("closed-19")[1].topics.join() === "n0", "first open computes the deferred selected-topic layout once");
      check(!!query(".board-page") && document.querySelectorAll(".visual-board-node").length === read().nodeIds.length, "actual page renders all selected-topic nodes");
      check(fitted() && read().fit, "first open fits the actual measured viewport");
      query('.board-zoom-controls button[aria-label="Zoom in"]').click(); component.nudgePan(); await settle();
      query(".visual-board-node").click(); await tick();
      const manual = read();
      check(manual.selectedNode !== null && manual.panX === 17 && manual.panY === -9, "node selection and manual view state are present");
      snapshot = { ...snapshot, transcript_line_count: 999 }; await settle();
      check(read().zoom === manual.zoom && read().panX === manual.panX && read().selectedNode === manual.selectedNode, "speech updates retain zoom pan and selected node");
      check(calls("closed-19").length === 2, "open speech updates also retain cached layout");
      query(".board-topic-chip:not(.is-active)").click(); await settle();
      check(read().selectedTopicIds.length === 2 && fitted() && read().panX === 0 && read().selectedNode === null, "topic change refits the stage and clears previous selection");
      query(".board-topic-all").click(); await settle();
      check(read().selectedTopicIds.length === 3 && fitted(), "select all renders and fits all topics");
      query(".board-topic-all").click(); await settle();
      check(read().selectedTopicIds.join() === "n0" && fitted(), "all-topic toggle returns to the first topic");
      query(".board-page-back").click(); await settle();
      check(!read().expanded && !query(".board-page"), "close unmounts the actual overlay");
      const closedCalls = probe.calls().length;
      query(".whiteboard-probe-open").click(); await settle();
      check(probe.calls().length === closedCalls && fitted(), "reopen uses the cached selected layout and fits again");
      snapshot = { ...snapshot, summaries: [{ whiteboard: board("replacement", 18) }] }; await settle();
      check(read().expanded && query(".board-page") && calls("replacement").length === 2, "a valid replacement board updates the open overlay in place");
      snapshot = { ...snapshot, summaries: [{ whiteboard: null }] } as any; await settle();
      check(!read().expanded && !query(".board-page"), "removing the board closes the overlay without a reactive loop");
      snapshot = { ...snapshot, summaries: [{ whiteboard: board("after-removal") }] }; await settle();
      check(calls("after-removal").length === 1 && !read().expanded, "a later board remains closed and only computes its overview");
      query(".whiteboard-probe-open").click(); await settle();
      snapshot = { ...snapshot, summaries: [...snapshot.summaries, { whiteboard: board("other-segment") }] };
      activeSummaryIdx = 1; await settle();
      check(read().expanded && calls("other-segment").length === 2 && query(".board-page"), "switching to a valid segment keeps the overlay open");
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); await settle();
      check(!read().expanded && !query(".board-page"), "Escape closes the overlay through its actual dismiss action");
      snapshot = { ...snapshot, summaries: [{ whiteboard: { probe_id: "invalid", nodes: [{ label: "one" }] } }] } as any;
      activeSummaryIdx = 0; await settle();
      check(!read().expanded && calls("invalid").length === 1, "invalid closed board attempts only its overview");
      query(".whiteboard-probe-open").click(); await settle();
      check(!read().expanded && !query(".board-page"), "invalid open returns to the closed state");
      readerEnabled = true;
      const carried = board("compact-carried"), carriedJSON = JSON.stringify(carried);
      snapshot = expandLiveSurface({ ...snapshot, whiteboard_table_version: 1,
        whiteboards: [carried], summaries: Array.from({ length: 32 }, (_, index) => ({
          title: `段 ${index}`, range_label: "10:00-10:05", body: `全文 ${index}`,
          line_count: index, whiteboard_ref: 0,
        })) } as any) as any;
      activeSummaryIdx = 0; await settle();
      query(".whiteboard-probe-open").click(); await settle();
      const carriedCalls = calls("compact-carried").length;
      const liveNode = query(".visual-board-node"), readerNode = query(".reader-probe .wb-node");
      for (let index = 1; index < 32; index++) { activeSummaryIdx = index; await tick(); }
      await settle();
      check(snapshot.summaries.every(chunk => chunk.whiteboard === carried), "compact reply shares one complete board across 32 chunks");
      check(calls("compact-carried").length === carriedCalls, "switching 32 carried chunks reuses actual LIVE and reader layouts");
      check(query(".visual-board-node") === liveNode && query(".reader-probe .wb-node") === readerNode, "carried chunk selection retains rendered nodes in both consumers");
      check(JSON.stringify(carried) === carriedJSON && fitted(), "compact restoration retains all board fields and the measured viewport");
      snapshot = { ...snapshot, active: true, session_id: "notification-probe", update_revision: 100,
        visible_lines: [], pending_from_line: 0 } as any;
      query('.board-zoom-controls button[aria-label="Zoom in"]').click(); component.nudgePan(); await settle();
      query(".visual-board-node").click(); await tick();
      const notificationView = read();
      for (let index = 0; index < 32; index++) {
        const latest = { title: `通知 ${index}`, range_label: "10:00-10:05", body: `全文 ${index}`,
          line_count: index, whiteboard: JSON.parse(carriedJSON) };
        snapshot = applyLiveSessionUpdate(snapshot as any, {
          update_revision: (snapshot as any).update_revision + 1, active: true, session_id: "notification-probe",
          course: null, started_at: null, transcript_line_count: snapshot.transcript_line_count, pending_line_count: 0,
          summary_count: snapshot.summaries.length + 1, latest_summary: latest,
          finish_phase: null, finish_revision: 0, summarizing: false, next_summary_at_ms: null,
        }).snapshot as any;
        activeSummaryIdx = snapshot.summaries.length - 1; await tick();
      }
      await settle();
      check(snapshot.summaries.every(chunk => chunk.whiteboard === carried), "32 independent summary notifications retain one complete board");
      check(calls("compact-carried").length === carriedCalls && query(".visual-board-node") === liveNode
        && query(".reader-probe .wb-node") === readerNode, "notifications reuse both cached layouts and rendered node trees");
      check(read().zoom === notificationView.zoom && read().panX === notificationView.panX
        && read().selectedNode === null, "carried notifications retain zoom and pan while segment changes clear node selection");
      const changed = { ...JSON.parse(carriedJSON), title: "Changed lecture board", probe_id: "notification-changed" };
      snapshot = { ...snapshot, summaries: [...snapshot.summaries, { whiteboard: changed }] } as any;
      activeSummaryIdx = snapshot.summaries.length - 1; await settle();
      check(calls("notification-changed").length >= 2 && read().expanded, "changed board recomputes the open overlay");
      const changedCalls = calls("notification-changed").length;
      const changedNode = query(".visual-board-node"), changedReaderNode = query(".reader-probe .wb-node");
      const recovery = JSON.parse(JSON.stringify(snapshot));
      recovery.summaries.push({ whiteboard: JSON.parse(JSON.stringify(changed)) });
      snapshot = mergeLiveSnapshot(snapshot as any, recovery) as any;
      activeSummaryIdx = snapshot.summaries.length - 1; await settle();
      check(snapshot.summaries.at(-1)?.whiteboard === changed && calls("notification-changed").length === changedCalls,
        "recovery read reuses the known board across an independently parsed history");
      check(query(".visual-board-node") === changedNode && query(".reader-probe .wb-node") === changedReaderNode
        && JSON.stringify(snapshot) === JSON.stringify(recovery), "recovery keeps both node trees and complete incoming JSON");
      const deltaWires: any[] = [];
      for (let index = 0; index < 32; index++) {
        const wire = JSON.parse(JSON.stringify({
          whiteboard_delta_version: 1, update_revision: (snapshot as any).update_revision + 1,
          active: true, session_id: "notification-probe", course: null, started_at: null,
          transcript_line_count: snapshot.transcript_line_count, pending_line_count: 0,
          summary_count: snapshot.summaries.length + 1, finish_phase: null, finish_revision: 0,
          summarizing: false, next_summary_at_ms: null,
          latest_summary: { title: `沿用通知 ${index}`, range_label: "10:00-10:05", body: `全文 ${index}`,
            line_count: index, whiteboard_from_summary: snapshot.summaries.length - 1 },
        }));
        deltaWires.push(wire);
        const result = applyLiveSessionNotification(snapshot as any, wire);
        if (result.needsResync) throw new Error("carried reference unexpectedly requested recovery");
        snapshot = result.snapshot as any;
        activeSummaryIdx = snapshot.summaries.length - 1; await tick();
      }
      await settle();
      check(deltaWires.every(wire => !("whiteboard" in wire.latest_summary)
        && !("whiteboard_delta_version" in snapshot)), "versioned carried notifications transmit no board and keep transport fields out of display state");
      check(snapshot.summaries.slice(-32).every(chunk => chunk.whiteboard === changed
        && !("whiteboard_from_summary" in chunk)), "32 versioned references resolve to the same complete known board");
      check(calls("notification-changed").length === changedCalls && query(".visual-board-node") === changedNode
        && query(".reader-probe .wb-node") === changedReaderNode, "carried references reuse both actual layout and DOM trees");
      const beforeGap = snapshot;
      const gap = { ...deltaWires.at(-1), update_revision: (snapshot as any).update_revision + 1,
        summary_count: snapshot.summaries.length + 2,
        latest_summary: { title: "Gap", range_label: "10:00-10:05", body: "Full gap summary", line_count: 2,
          whiteboard_from_summary: snapshot.summaries.length } };
      const gapResult = applyLiveSessionNotification(snapshot as any, gap);
      check(gapResult.needsResync && gapResult.snapshot.summaries === beforeGap.summaries
        && gapResult.snapshot.update_revision === gap.update_revision, "missing referenced summary requests recovery without inventing a whiteboard");
      const gapRecovery = JSON.parse(JSON.stringify(gapResult.snapshot));
      gapRecovery.summaries.push({ whiteboard: JSON.parse(JSON.stringify(changed)) }, { whiteboard: JSON.parse(JSON.stringify(changed)) });
      snapshot = mergeLiveSnapshot(gapResult.snapshot, gapRecovery) as any;
      activeSummaryIdx = snapshot.summaries.length - 1; await settle();
      check(snapshot.summaries.at(-1)?.whiteboard === changed && snapshot.summaries.length === gap.summary_count
        && calls("notification-changed").length === changedCalls && JSON.stringify(snapshot) === JSON.stringify(gapRecovery),
        "complete recovery fills both missing chunks and retains existing layouts and all JSON fields");
      const delayed = applyLiveSessionNotification(snapshot as any, { ...gap, update_revision: 1 });
      check(delayed.snapshot === snapshot && !delayed.needsResync, "delayed reference notifications neither change the display nor request recovery");
      query(".board-page-back").click(); activeSummaryIdx = 0; await settle();
      const dense = board("bounded-dense", 96), denseJSON = JSON.stringify(dense);
      snapshot = { ...snapshot, summaries: [{ whiteboard: dense }] };
      await settle(); query(".whiteboard-probe-open").click(); await settle();
      query(".board-topic-all").click(); query(".reader-probe .topic-all").click(); await settle();
      const denseOptions = { fallbackBoardTitle: "知識整理", externalNodeLabel: "外部" };
      const oldPreview = probe.before.compute(dense, denseOptions);
      const oldActive = probe.before.compute(dense, { ...denseOptions, topicIds: read().selectedTopicIds });
      const readerTopics = Array.from(document.querySelectorAll<HTMLElement>(".reader-probe .topic-list button.active"))
        .map(node => node.textContent?.trim());
      const readerTopicIds = probe.before.topics(dense).filter((topic: any) => readerTopics.includes(topic.label)).map((topic: any) => topic.id);
      const oldReader = probe.before.compute(dense, { fallbackBoardTitle: "知識整理ボード", externalNodeLabel: "外部", topicIds: readerTopicIds });
      const currentLayouts = component.layouts();
      check(JSON.stringify(currentLayouts.preview) === JSON.stringify(oldPreview)
        && JSON.stringify(currentLayouts.active) === JSON.stringify(oldActive), "dense overview and selected-topic layout preserve every pre-bound geometry byte");
      const labelsMatch = (selector: string, layout: any) => {
        const rendered = Array.from(document.querySelectorAll<HTMLElement>(selector));
        const labels = layout.edges.filter((edge: any) => edge.label);
        return rendered.length === labels.length && rendered.every((node, index) => node.textContent === labels[index].label
          && Number.parseFloat(node.style.left) === labels[index].lx && Number.parseFloat(node.style.top) === labels[index].ly);
      };
      check(document.querySelectorAll(".visual-board-node").length === oldActive.nodes.length
        && labelsMatch(".visual-board-edge-label", oldActive), "dense LIVE renders every selected node and exact original edge label position");
      check(document.querySelectorAll(".reader-probe .wb-node").length === oldReader.nodes.length
        && labelsMatch(".reader-probe .edge-label", oldReader), "dense Markdown reader renders every selected node and exact original edge label position");
      const pathsMatch = (selector: string, layout: any) => {
        const paths = Array.from(document.querySelectorAll(selector));
        return paths.length === layout.edges.length && paths.every((node, index) => {
          const edge = layout.edges[index];
          return node.getAttribute("d") === `M ${edge.x1} ${edge.y1} Q ${edge.cx} ${edge.cy} ${edge.x2} ${edge.y2}`;
        });
      };
      check(pathsMatch(".visual-board-edge", oldActive) && pathsMatch(".reader-probe .links path", oldReader)
        && JSON.stringify(dense) === denseJSON && fitted(), "dense curve paths, full input and measured viewport remain unchanged");
      query(".board-page-back").click(); await settle();
      for (const [name, factory, expectedEdges] of [
        ["duplicate", duplicateBoard, 1], ["reserved", reservedBoard, 1],
        ["prototype", prototypeBoard, 3], ["phantom", phantomBoard, 1],
      ] as const) {
        const saved = factory(), original = JSON.stringify(saved);
        snapshot = { ...snapshot, summaries: [{ whiteboard: saved }] } as any;
        await settle(); query(".whiteboard-probe-open").click(); await settle();
        const mainLabels = saved.nodes.filter(node => node.role === "main").map(node => node.label);
        for (const consumer of [
          { name: "LIVE", root: ".board-page", nodes: ".visual-board-node", label: ".visual-board-node-label", detail: ".visual-board-node-detail", chips: ".visual-board-chip", topics: ".board-topic-chip", all: ".board-topic-all", selected: ".is-selected", edges: ".visual-board-edge" },
          { name: "reader", root: ".reader-probe .whiteboard", nodes: ".wb-node", label: "strong", detail: ".detail", chips: ".chips > span", topics: ".topic-list button", all: ".topic-all", selected: ".selected", edges: ".links path" },
        ]) {
          const root = query(consumer.root);
          const all = () => Array.from(root.querySelectorAll<HTMLElement>(consumer.nodes));
          const labels = () => all().map(node => node.querySelector(consumer.label)?.textContent);
          const tag = `${name}/${consumer.name}`;
          if (mainLabels.length > 1) {
            const topicButtons = Array.from(root.querySelectorAll<HTMLButtonElement>(consumer.topics));
            check(JSON.stringify(topicButtons.map(button => button.textContent?.trim())) === JSON.stringify(mainLabels), `${tag} renders every topic with a distinct key`);
            root.querySelector<HTMLButtonElement>(consumer.all)!.click(); await settle();
            for (const main of mainLabels) {
              const button = topicButtons.find(button => button.textContent?.trim() === main)!;
              button.click(); await settle();
              check(!labels().includes(main) && mainLabels.filter(label => label !== main).every(label => labels().includes(label)), `${tag} deselects only ${main}`);
              button.click(); await settle();
            }
          }
          const structures = saved.nodes.filter(node => node.node_type !== "term");
          check(JSON.stringify(labels()) === JSON.stringify(structures.map(node => node.label)), `${tag} retains all structure labels and order`);
          check(JSON.stringify(all().map(node => node.querySelector(consumer.detail)?.textContent)) === JSON.stringify(structures.map(node => node.detail)), `${tag} retains full structure details`);
          const chips = Array.from(root.querySelectorAll<HTMLElement>(consumer.chips));
          check(JSON.stringify(chips.map(chip => [chip.textContent, chip.title])) === JSON.stringify(saved.nodes.filter(node => node.node_type === "term").map(node => [node.label, node.detail])), `${tag} retains all term labels and details`);
          const paths = Array.from(root.querySelectorAll<SVGPathElement>(consumer.edges));
          check(paths.length === expectedEdges && paths.every(path => !/NaN|Infinity/.test(path.getAttribute("d") || "")) && all().every(node => Number.isFinite(parseFloat(node.style.left)) && Number.isFinite(parseFloat(node.style.top))), `${tag} has only real edges and finite node geometry`);
          for (const node of all()) {
            node.click(); await settle();
            check(root.querySelectorAll(consumer.nodes + consumer.selected).length === 1 && node.matches(consumer.selected), `${tag} independently selects ${node.querySelector(consumer.label)?.textContent}`);
            node.click(); await settle();
          }
        }
        const stateBefore = read(), visibleBefore = query(".reader-probe").textContent;
        for (let i = 0; i < 20; i++) { snapshot = { ...snapshot, transcript_line_count: i + 1000 }; await tick(); }
        await settle();
        check(JSON.stringify(read()) === JSON.stringify(stateBefore) && query(".reader-probe").textContent === visibleBefore, `${name} speech snapshots preserve both displays and viewport`);
        check(JSON.stringify(saved) === original, `${name} saved input is unchanged by both consumers`);
        query(".board-page-back").click(); await settle();
      }
      snapshot = { ...snapshot, summaries: [{ whiteboard: board("unmount-open") }] }; await settle();
      query(".whiteboard-probe-open").click(); await settle();
      check(read().expanded, "open page is present before unmount");
      await probe.finish({ checks, error: null });
    } catch (error) { await probe.finish({ checks, error: String(error) }); }
  }
</script>

<p>Isolated LIVE whiteboard verification — transcript {snapshot.transcript_line_count}</p>
<Whiteboard {snapshot} {activeSummaryIdx} bind:this={component} />
<div class="reader-probe" style="width:900px">
  {#if readerBoard}<MarkdownWhiteboard board={readerBoard as any} />{/if}
</div>
