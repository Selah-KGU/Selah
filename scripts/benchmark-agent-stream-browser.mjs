// Client rendering probe of the actual AgentChat append function and message
// markup. No application initialization, IPC, model, clipboard or microphone.
import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";

const live = await readFile("src/lib/views/AgentChat.svelte", "utf8");
const before = process.argv.includes("--before");
const frozen = before ? await readFile("tests/fixtures/agent-stream-before.txt", "utf8") : "";
const fn = name => {
  const source = before && (name === "appendAssistantText" || name === "quoteReply") ? frozen : live;
  const result = source.match(new RegExp(`  function ${name}\\([^]*?\\n  }\\n`))?.[0];
  if (!result) throw new Error(`Actual AgentChat function missing: ${name}`);
  return result;
};
const from = live.indexOf('        {#each messages as m (m.id)}');
const to = live.indexOf('      {#if showStatus}', from);
if (from < 0 || to < 0) throw new Error("Actual AgentChat message markup missing");
const markup = live.slice(from, to).replace(/\s*\{\/if\}\s*$/, "");
const component = `<script lang="ts">
  import { onMount, tick } from "svelte";
  import { createMarkdownRenderer } from "./src/lib/markdownRenderer.ts";
  import DOMPurify from "dompurify";
  type UIMessage = { id: number; role: string; content: string; conv_id: string; created_at: number; _streaming?: boolean; images?: any[] };
  let messages = $state<UIMessage[]>([]);
  const activeConvId = "probe";
  let copiedId = $state<number | null>(null);
  let quotedMessage = $state<UIMessage | null>(null);
  const composerTextarea = null;
  const conversationView = { capture: () => () => true };
  const metrics = { ids: 0, renders: 0, parses: 0 };
  const parser = createMarkdownRenderer(html => { metrics.parses++; return DOMPurify.sanitize(html); });
  const renderCache = { render(source: string) { metrics.renders++; return parser.render(source); } };
  const selahLogoUrl = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
  const TOOL_CALL_LEAK_FALLBACK = "内部ツール呼び出しの形式が崩れたため、そのまま表示せずに止めました。もう一度、必要な資料名や操作を指定してください。";
  const copyMessage = () => {};
  ${fn("render")}
  ${fn("looksLikePseudoToolCallLeak")}
  ${fn("displayContent")}
  ${fn("appendAssistantText")}
  ${fn("quoteReply")}
  ${fn("imageSrc")}
  onMount(() => { void run(); });
  async function run() {
    const results = [];
    try {
      for (const count of [0, 100, 1000]) {
        messages = Array.from({ length: count }, (_, i) => ({
          get id() { metrics.ids++; return i + 1; }, role: "assistant", content: "履歴 " + i + " **Unicode 🙂**", conv_id: "probe", created_at: 0
        }));
        appendAssistantText("応答 🙂"); await tick();
        for (let i = 0; i < 20; i++) { appendAssistantText("。"); await tick(); }
        const times = [], start = { ...metrics };
        for (let batch = 0; batch < 9; batch++) {
          const begin = performance.now();
          for (let i = 0; i < 100; i++) { appendAssistantText("字"); await tick(); }
          times.push(performance.now() - begin);
        }
        const delta = { idReads: metrics.ids - start.ids, renders: metrics.renders - start.renders, parses: metrics.parses - start.parses };
        times.sort((a, b) => a - b);
        const streamed = messages.at(-1)!;
        if (messages.length !== count + 1 || streamed.content !== "応答 🙂" + "。".repeat(20) + "字".repeat(900)) throw new Error("full message mismatch");
        if (document.querySelector(".streaming-md")?.textContent !== streamed.content) throw new Error("actual streaming DOM text mismatch");
        quoteReply(streamed); const quoted = quotedMessage!.content;
        appendAssistantText("後続"); await tick();
        if (quotedMessage!.content !== quoted) throw new Error("quoted reply changed after more streaming");
        results.push({ history: count, batches: 100, trials: 9, medianMs: times[4], ...delta });
        quotedMessage = null;
      }
      await (window as any).__agentRenderProbe.finish({ before: ${before}, results, error: null });
    } catch (error) { await (window as any).__agentRenderProbe.finish({ before: ${before}, results, error: String(error) }); }
  }
</script>
<div class="message-probe">${markup}</div>`;
const source = `
import { mount, unmount } from "svelte";
import Probe from "agent-message-probe";
let app;
window.__agentRenderProbe = { async finish(result) {
  await unmount(app);
  const output = document.createElement("pre"); output.textContent = JSON.stringify(result, null, 2); document.body.append(output);
  await fetch("/result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(result) });
}};
app = mount(Probe, { target: document.getElementById("probe") });`;
const bundle = await build({
  stdin: { contents: source, resolveDir: process.cwd(), loader: "js" },
  bundle: true, write: false, platform: "browser", format: "esm", conditions: ["browser"],
  plugins: [{ name: "actual-agent-messages", setup(builder) {
    builder.onResolve({ filter: /^agent-message-probe$/ }, () => ({ path: "probe", namespace: "agent-messages" }));
    builder.onLoad({ filter: /.*/, namespace: "agent-messages" }, () => ({
      contents: compile(component, { filename: "agent-message-probe.svelte", generate: "client" }).js.code,
      loader: "js", resolveDir: process.cwd(),
    }));
  } }],
});
const server = createServer(async (request, response) => {
  if (request.url === "/probe.js") { response.setHeader("Content-Type", "text/javascript"); response.end(bundle.outputFiles[0].text); }
  else if (request.url === "/result" && request.method === "POST") {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    console.log(Buffer.concat(chunks).toString()); response.end("ok");
  } else {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><meta charset="utf-8"><title>Agent stream rendering probe</title><div id="probe"></div><script type="module" src="/probe.js"></script>');
  }
});
server.listen(0, "127.0.0.1", () => console.log(`http://127.0.0.1:${server.address().port}/`));
