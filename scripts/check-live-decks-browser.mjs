// Serve an isolated actual-client probe. Open the printed localhost URL; Ctrl-C
// stops it. It never imports the app, Tauri, network models or audio APIs.
import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { createServer } from "node:http";

const liveSource = await readFile("src/lib/views/Live.svelte", "utf8");
const displayClock = liveSource.match(/  function bindLiveDisplayClock\(\) {[\s\S]*?\n  }\n/)?.[0];
if (!displayClock) throw new Error("Production LIVE display clock was not found");
const transcriptFollow = liveSource.match(/  function bindLiveTranscriptFollow\(\) {[\s\S]*?\n  }\n/)?.[0];
if (!transcriptFollow) throw new Error("Production LIVE transcript follow binding was not found");
const clockSource = `<script lang="ts">
  import { onDestroy } from "svelte";
  import { ResourceScope } from ${JSON.stringify(resolve("src/lib/resourceScope.ts"))};
  let { snapshot, visible }: { snapshot: { active: boolean }; visible: boolean } = $props();
  const liveSurfaceVisible = $derived(visible);
  const resources = new ResourceScope();
  let now = $state(new Date(0));
  ${displayClock}
  bindLiveDisplayClock();
  onDestroy(() => resources.dispose());
</script><span class="live-clock-probe">{now.getTime()}</span>`;
const followSource = `<script lang="ts">
  import { onDestroy } from "svelte";
  import { ResourceScope } from ${JSON.stringify(resolve("src/lib/resourceScope.ts"))};
  import { LiveTranscriptFollow } from ${JSON.stringify(resolve("src/lib/views/live/liveTranscriptFollow.ts"))};
  let { snapshot, visible, covered, listening, following } = $props();
  const liveSurfaceVisible = $derived(visible);
  const whiteboardExpanded = $derived(covered), summaryDetailOpen = false;
  const sttListening = $derived(listening), autoFollow = $derived(following);
  const resources = new ResourceScope();
  let scrollEl = $state<HTMLElement | null>(null);
  ${transcriptFollow}
  bindLiveTranscriptFollow();
  onDestroy(() => resources.dispose());
</script>
<div class="live-follow-probe" bind:this={scrollEl} style="height:80px;width:400px;overflow-y:auto;scroll-behavior:auto">
  {#each Array.from({ length: Math.min(snapshot.transcript_line_count, 120) }, (_, i) => snapshot.transcript_line_count - Math.min(snapshot.transcript_line_count, 120) + i) as line}
    <div style="height:24px">Transcript {line} 🙂</div>
  {/each}
</div>`;
const source = `
import { mount, unmount } from "svelte";
import Harness from ${JSON.stringify(resolve("tests/fixtures/live-deck-browser.svelte"))};
const savedSet = window.setInterval, savedClear = window.clearInterval;
const savedFrame = window.requestAnimationFrame, savedCancelFrame = window.cancelAnimationFrame;
const SavedDate = window.Date; let testTime = 1000000;
window.Date = class extends SavedDate {
  constructor(...args) { if (args.length) super(...args); else super(testTime); }
  static now() { return testTime; }
};
const timers = new Map(); let nextId = 0, app;
const frames = new Map(); let nextFrame = 0;
window.requestAnimationFrame = callback => { const id = ++nextFrame; frames.set(id, callback); return id; };
window.cancelAnimationFrame = id => frames.delete(id);
window.setInterval = (callback, delay) => { const id = ++nextId; timers.set(id, { callback, delay }); return id; };
window.clearInterval = id => timers.delete(id);
window.__deckProbe = {
  frameCount: () => frames.size, frameCallbacks: () => [...frames.values()],
  flushFrames: () => { for (const [id, callback] of [...frames]) { frames.delete(id); callback(testTime); } },
  timerCount: (delay = 4500) => [...timers.values()].filter(timer => timer.delay === delay).length,
  timerIds: (delay = 4500) => [...timers].filter(([, timer]) => timer.delay === delay).map(([id]) => id),
  timerCallbacks: (delay = 4500) => [...timers.values()].filter(timer => timer.delay === delay).map(timer => timer.callback),
  setTime: time => { testTime = time; },
  step: (delay = 4500) => { for (const timer of [...timers.values()]) if (timer.delay === delay) timer.callback(); },
  async finish(result) {
    await unmount(app);
    if (timers.size) result.error = result.error || "rotation timer retained after unmount";
    else result.checks.push("unmount removes all rotation timers");
    if (frames.size) result.error = result.error || "transcript frame retained after unmount";
    else result.checks.push("unmount removes all transcript frames");
    window.setInterval = savedSet; window.clearInterval = savedClear;
    window.Date = SavedDate;
    window.requestAnimationFrame = savedFrame; window.cancelAnimationFrame = savedCancelFrame;
    const output = document.createElement("pre");
    output.textContent = JSON.stringify(result, null, 2); document.body.append(output);
    await fetch("/result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(result) });
  }
};
app = mount(Harness, { target: document.getElementById("probe") });
`;
const bundle = await build({
  stdin: { contents: source, resolveDir: process.cwd(), loader: "js" },
  bundle: true, write: false, platform: "browser", format: "esm", conditions: ["browser"],
  plugins: [{ name: "actual-svelte-client", setup(builder) {
    builder.onResolve({ filter: /^test-live-display-clock$/ }, () => ({ path: "clock", namespace: "live-clock" }));
    builder.onLoad({ filter: /.*/, namespace: "live-clock" }, () => ({
      contents: compile(clockSource, { filename: "live-display-clock-probe.svelte", generate: "client" }).js.code,
      loader: "js", resolveDir: process.cwd(),
    }));
    builder.onResolve({ filter: /^test-live-transcript-follow$/ }, () => ({ path: "follow", namespace: "live-follow" }));
    builder.onLoad({ filter: /.*/, namespace: "live-follow" }, () => ({
      contents: compile(followSource, { filename: "live-transcript-follow-probe.svelte", generate: "client" }).js.code,
      loader: "js", resolveDir: process.cwd(),
    }));
    builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => {
      const { js } = compile(await readFile(path, "utf8"), { filename: path, generate: "client", css: "injected" });
      return { contents: js.code, loader: "js", resolveDir: dirname(path) };
    });
  } }],
});
const server = createServer(async (request, response) => {
  if (request.url === "/probe.js") {
    response.setHeader("Content-Type", "text/javascript"); response.end(bundle.outputFiles[0].text);
  } else if (request.url === "/result" && request.method === "POST") {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const result = JSON.parse(Buffer.concat(chunks).toString());
    console.log(JSON.stringify({ passed: result.checks.length, error: result.error, checks: result.checks }));
    response.end("ok");
  } else if (request.url === "/") {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><meta charset="utf-8"><title>LIVE deck verification</title><div id="probe"></div><script type="module" src="/probe.js"></script>');
  } else { response.statusCode = 404; response.end(); }
});
server.listen(0, "127.0.0.1", () => console.log(`http://127.0.0.1:${server.address().port}/`));
