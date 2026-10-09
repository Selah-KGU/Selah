import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { build } from "esbuild";

let instance = 0;

// Execute the component's actual TypeScript logic with IPC and DOM boundaries
// replaced. This checks async conversation ownership, not Svelte DOM rendering.
export async function loadAgentPanel({ hash = "" } = {}) {
  const instanceId = ++instance;
  const path = resolve("src/lib/AgentPanel.svelte");
  const source = await readFile(path, "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error("AgentPanel TypeScript script was not found");
  const result = await build({
    stdin: {
      contents: `
        const window = { location: { search: "", hash: ${JSON.stringify(hash)} } };
        const document = { title: "", hidden: false };
        const requestAnimationFrame = () => 0;
        const cancelAnimationFrame = () => {};
        const $state = (value) => value;
        const $derived = Object.assign((value) => value, { by: (read) => read() });
        const $effect = () => {};
        ${script}
        import { configure, calls, listeners } from "test-panel-backend";
        export { configure, calls, listeners };
        export const panel = {
          load: loadActiveConversation,
          deleted: conversationDeleted,
          send,
          stop,
          finish: finishTurn,
          dispose: () => resources.dispose(),
          setDraft(value) { draft = value; attachments = [{ mime: "image/png", data_base64: "AA==" }]; sttCommittedText = value; },
          addAttachment(part) { attachments = [...attachments, part]; },
          addFiles, pick: onPickFiles,
          get attachmentState() { return { attachments, error: typeof attachmentError === "undefined" ? error : attachmentError }; },
          streaming(value) { sending = true; streamText = value; },
          get state() { return { convId, messages, sending, streamText, draft, attachments, sttCommittedText, conversationReady, preparing, error }; },
        };
      `,
      loader: "ts", resolveDir: dirname(path), sourcefile: path,
    },
    bundle: true, write: false, platform: "node", format: "esm",
    plugins: [{
      name: "panel-boundaries",
      setup(plugin) {
        plugin.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|test-panel-backend)$/ }, () => ({ path: "backend", namespace: "panel-test" }));
        plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: "svelte", namespace: "panel-test" }));
        plugin.onResolve({ filter: /\.svelte$|\.png$|\.css$|^dompurify$/ }, () => ({ path: "presentation", namespace: "panel-test" }));
        plugin.onLoad({ filter: /.*/, namespace: "panel-test" }, ({ path }) => ({ contents: path === "backend" ? `
          export const calls = [], listeners = [];
          let run, subscribe;
          export function configure(invoke, listen) { run = invoke; subscribe = listen; }
          export const emit = async () => {};
          export async function invoke(command, args) { calls.push({ command, args }); return run(command, args); }
          export async function listen(name, receive) {
            const listener = { name, receive, releases: 0 };
            listeners.push(listener);
            const release = () => { listener.releases++; };
            return subscribe ? subscribe(listener, release) : release;
          }
        ` : path === "svelte" ? `export const onMount = () => {}, onDestroy = () => {}, tick = async () => {};`
          : `export default { sanitize: (value) => value };`, loader: "js" }));
      },
    }],
  });
  // URL fragments isolate identical compiled modules between component instances.
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}#panel-${instanceId}`);
}
