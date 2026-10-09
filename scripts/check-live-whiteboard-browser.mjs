// Isolated client probe of the actual LIVE whiteboard state and page. Open the
// printed localhost URL; Ctrl-C stops it. No app, native IPC, model or audio.
import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { createServer } from "node:http";

const live = await readFile("src/lib/views/Live.svelte", "utf8");
function section(start, end) {
  const from = live.indexOf(start), to = live.indexOf(end, from + start.length);
  if (from < 0 || to < 0) throw new Error(`Actual LIVE section missing: ${start}`);
  return live.slice(from, to);
}
const state = section("  let whiteboardExpanded = $state(false);", "  let sessionEventVersion = 0;");
const markup = section("  {#if activeWhiteboardLayout && whiteboardExpanded}", "  {#if summaryDetailOpen");
const component = `<script lang="ts">
  import { untrack } from "svelte";
  import { computeWhiteboardLayout, prepareWhiteboardLayout, whiteboardLayoutReady, whiteboardTopics } from ${JSON.stringify(resolve("src/lib/whiteboardLayout.ts"))};
  import LiveWhiteboardPage from ${JSON.stringify(resolve("src/lib/views/live/LiveWhiteboardPage.svelte"))};
  import type { WhiteboardStagePreset } from ${JSON.stringify(resolve("src/lib/views/live/liveTypes.ts"))};
  let { snapshot, activeSummaryIdx = 0, aiReplyLanguage = "ja" } = $props();
  const summaries = $derived(snapshot.summaries);
  ${state}
  export function inspect() {
    return { expanded: whiteboardExpanded, zoom: whiteboardZoom,
      panX: whiteboardPanX, panY: whiteboardPanY,
      width: boardCanvasWidth, height: boardCanvasHeight,
      stage: activeWhiteboardStage, selectedTopicIds: [...selectedTopicIds],
      nodeIds: activeWhiteboardLayout?.nodes.map(node => node.id) ?? [],
      selectedNode: selectedBoardNodeId, fit: initialFitDone };
  }
  export function nudgePan() { whiteboardPanX = 17; whiteboardPanY = -9; }
  export function layouts() { return { active: activeWhiteboardLayout, preview: previewWhiteboardLayout }; }
</script>
<div class="whiteboard-probe" style="position:relative;width:900px;height:600px">
  <button type="button" class="whiteboard-probe-open" onclick={openWhiteboardOverlay}>Open board</button>
  <p class="whiteboard-probe-preview">{previewWhiteboardLayout?.title}:{previewWhiteboardLayout?.nodes.length ?? 0}</p>
  ${markup}
</div>`;

const labelBoundBefore = await readFile("tests/fixtures/whiteboard-label-bound-before.js", "utf8");
const source = `
import { mount, unmount } from "svelte";
import Harness from ${JSON.stringify(resolve("tests/fixtures/live-whiteboard-browser.svelte"))};
const previousWindow = {};
(function(window) { ${labelBoundBefore} })(previousWindow);
const engine = window.WhiteboardLayout, calls = []; let app;
window.WhiteboardLayout = { ...engine, compute(board, options) {
  calls.push({ id: board.probe_id, topics: [...(options?.topicIds ?? [])] });
  return engine.compute(board, options);
}};
window.__whiteboardProbe = {
  before: previousWindow.WhiteboardLayout,
  calls: () => calls.map(call => ({ ...call, topics: [...call.topics] })),
  async finish(result) {
    await unmount(app);
    window.WhiteboardLayout = engine;
    if (document.querySelector(".board-page")) result.error ||= "whiteboard page retained after unmount";
    else result.checks.push("whiteboard page removed at unmount");
    if (document.querySelector(".reader-probe")) result.error ||= "reader whiteboard retained after unmount";
    else result.checks.push("reader whiteboard removed at unmount");
    const heading = document.createElement("h1");
    heading.textContent = result.error ? "Whiteboard checks failed" : result.checks.length + " whiteboard checks passed";
    const description = document.createElement("p");
    description.textContent = "Actual LIVE and Markdown components: exact dense-board geometry, label positions, notifications, gap recovery, duplicate IDs, topics and unmount.";
    const details = document.createElement("details"), summary = document.createElement("summary"), output = document.createElement("pre");
    summary.textContent = "Full check results";
    output.textContent = JSON.stringify(result, null, 2);
    details.append(summary, output); document.body.append(heading, description, details);
    await fetch("/result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(result) });
  }
};
app = mount(Harness, { target: document.getElementById("probe") });
`;
const bundle = await build({
  stdin: { contents: source, resolveDir: process.cwd(), loader: "js" },
  bundle: true, write: false, platform: "browser", format: "esm", conditions: ["browser"],
  plugins: [{ name: "actual-live-whiteboard-client", setup(builder) {
    builder.onResolve({ filter: /^test-live-whiteboard$/ }, () => ({ path: "whiteboard", namespace: "live-whiteboard" }));
    builder.onLoad({ filter: /.*/, namespace: "live-whiteboard" }, () => ({
      contents: compile(component, { filename: "live-whiteboard-probe.svelte", generate: "client" }).js.code,
      loader: "js", resolveDir: process.cwd(),
    }));
    builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
      contents: compile(await readFile(path, "utf8"), { filename: path, generate: "client", css: "injected" }).js.code,
      loader: "js", resolveDir: dirname(path),
    }));
  } }],
});
const layout = await readFile("static/whiteboard-layout.js", "utf8");
const server = createServer(async (request, response) => {
  if (request.url === "/probe.js" || request.url === "/whiteboard-layout.js") {
    response.setHeader("Content-Type", "text/javascript");
    response.end(request.url === "/probe.js" ? bundle.outputFiles[0].text : layout);
  } else if (request.url === "/result" && request.method === "POST") {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    const result = JSON.parse(Buffer.concat(chunks).toString());
    console.log(JSON.stringify({ passed: result.checks.length, ...result })); response.end("ok");
  } else if (request.url === "/") {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><meta charset="utf-8"><title>LIVE whiteboard verification</title><div id="probe"></div><script src="/whiteboard-layout.js"></script><script type="module" src="/probe.js"></script>');
  } else { response.statusCode = 404; response.end(); }
});
server.listen(0, "127.0.0.1", () => console.log(`http://127.0.0.1:${server.address().port}/`));
