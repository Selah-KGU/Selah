import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { build } from "esbuild";

let instance = 0;
// Run the main component's actual request/stream logic. Rendering, microphone
// capture and Tauri transports are boundaries; this is not a DOM integration test.
export async function loadAgentChat() {
  const path = resolve("src/lib/views/AgentChat.svelte");
  const script = (await readFile(path, "utf8")).match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error("AgentChat script was not found");
  const result = await build({
    stdin: {
      contents: `
        const document = { hidden: false };
        const requestAnimationFrame = () => 0, cancelAnimationFrame = () => {};
        const $agentReady = false;
        const $state = value => value;
        const $derived = Object.assign(value => value, { by: read => read() });
        const $effect = () => {};
        ${script}
        import { configure, calls, listeners } from "test-chat-backend";
        import { destroy } from "svelte";
        export { configure, calls, listeners };
        export const messagesProbe = {
          set(rows) { messages = rows; }, append: appendAssistantText,
          quoteLast() { quoteReply(messages.at(-1)); },
          finalize() { finalizeTurn(false); },
          get rows() { return messages; }, get quote() { return quotedMessage; },
        };
        export const panel = {
          select: selectConversation, send, stop: async () => cancel(), dispose: destroy,
          setDraft(value) { inputText = value; },
          addAttachment(part) { attachments = [...attachments, part]; },
          addFiles, pick: onPickFiles,
          get attachmentState() { return { attachments, error: typeof attachmentError === "undefined" ? "" : attachmentError }; },
          get state() { return { convId: activeConvId, messages, sending, activeRequestId, attachments, draft: inputText }; },
        };
      `,
      loader: "ts", resolveDir: dirname(path), sourcefile: path,
    },
    bundle: true, write: false, platform: "node", format: "esm",
    plugins: [{
      name: "chat-boundaries",
      setup(plugin) {
        plugin.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|test-chat-backend)$/ }, () => ({ path: "backend", namespace: "chat-test" }));
        plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: "svelte", namespace: "chat-test" }));
        plugin.onResolve({ filter: /^svelte\/transition$/ }, () => ({ path: "transitions", namespace: "chat-test" }));
        plugin.onResolve({ filter: /^\.\.\/api$/ }, () => ({ path: "api", namespace: "chat-test" }));
        plugin.onResolve({ filter: /^\.\.\/stores$/ }, () => ({ path: "stores", namespace: "chat-test" }));
        plugin.onResolve({ filter: /\.svelte$|\.png$|^dompurify$/ }, () => ({ path: "presentation", namespace: "chat-test" }));
        plugin.onLoad({ filter: /.*/, namespace: "chat-test" }, ({ path }) => ({ contents: path === "backend" ? `
          export const calls = [], listeners = [];
          let run;
          export function configure(invoke) { run = invoke; }
          export async function invoke(command, args) { calls.push({ command, args }); return run(command, args); }
          export async function listen(name, receive) { const listener = { name, receive, releases: 0 }; listeners.push(listener); return () => listener.releases++; }
        ` : path === "svelte" ? `
          const cleanups = [];
          export const onMount = () => {}, onDestroy = fn => cleanups.push(fn), tick = async () => {};
          export const destroy = () => cleanups.forEach(fn => fn());
        ` : path === "api" ? `
          export * from ${JSON.stringify(resolve("src/lib/agentApi.ts"))};
          export const isDemoActive = () => false, isAiReady = async () => true, getAiConfig = async () => ({});
        ` : path === "stores" ? `
          const store = { set() {}, subscribe(fn) { fn([]); return () => {}; } };
          export const agentConversations = store, agentActiveConvId = store, agentReady = store;
        ` : path === "transitions" ? `export const fade = () => {}, scale = () => {};`
          : `export default { sanitize: value => value };`, loader: "js", resolveDir: resolve(".") }));
      },
    }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}#chat-${++instance}`);
}
