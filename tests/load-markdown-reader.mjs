import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';
let instance = 0;
// Actual reader script and whiteboard splitter. Native transports, DOMPurify,
// Svelte DOM/tick, clipboard and timers are boundaries; default parsing is Marked.
export async function loadMarkdownReader({ query = "?tabLabel=reader-A&ownerLabel=document-tabs" } = {}) {
  const path = resolve('src/lib/MarkdownReaderSurface.svelte');
  let script = (await readFile(path, 'utf8')).match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('Reader script missing');
  const dirty = 'const dirty = $derived(editing && editorValue !== savedMarkdown);';
  if (!script.includes(dirty)) throw new Error('Dirty rendering boundary changed');
  script = script.replace(dirty, 'function isDirty() { return editing && editorValue !== savedMarkdown; }').replace(/\bdirty\b/g, 'isDirty()');
  const result = await build({ stdin: { contents: `
    import { clock, setTimeout, clearTimeout, setInterval, clearInterval } from 'test-reader-clock';
    import { document, window, localStorage, writes } from 'test-reader-environment';
    const $state = value => value, $derived = value => value, $effect = () => {};
    ${script}
    import { configure, calls, listeners, emits, configureParser, parses } from 'test-reader-backend';
    import { mount, destroy } from 'svelte';
    export { configure, calls, listeners, emits, clock, writes, configureParser, parses };
    export const reader = { mount, dispose: destroy, apply: applyPayload, initial: fetchInitialPayload, save, share, enterEdit, cancelEdit, render: renderMarkdown,
      edit(value) { editorValue = value; },
      get state() { return { path, filename, markdown, savedMarkdown, renderedSegments, error, loading, editing, saving, editorValue, toastText }; },
    };
  `, loader: 'ts', sourcefile: path, resolveDir: dirname(path) }, bundle: true, write: false, format: 'esm', platform: 'node',
    plugins: [{ name: 'reader-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|test-reader-backend|marked)$/ }, () => ({ path: 'backend', namespace: 'reader-test' }));
      plugin.onResolve({ filter: /^test-reader-environment$/ }, () => ({ path: 'environment', namespace: 'reader-test' }));
      plugin.onResolve({ filter: /^test-reader-clock$/ }, () => ({ path: 'clock', namespace: 'reader-test' }));
      plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'svelte', namespace: 'reader-test' }));
      plugin.onResolve({ filter: /\.svelte$|^dompurify$/ }, () => ({ path: 'presentation', namespace: 'reader-test' }));
      plugin.onLoad({ filter: /auxiliarySurfaceTheme\.ts$/ }, async ({ path }) => ({ contents: `import { document, localStorage } from 'test-reader-environment';\n${await readFile(path, 'utf8')}`, loader: 'ts' }));
      plugin.onLoad({ filter: /resourceScope\.ts$/ }, async ({ path }) => ({ contents: `import { setTimeout, clearTimeout, setInterval, clearInterval } from 'test-reader-clock';\n${await readFile(path, 'utf8')}`, loader: 'ts' }));
      plugin.onLoad({ filter: /.*/, namespace: 'reader-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        import { marked as real } from ${JSON.stringify(resolve('node_modules/marked/lib/marked.esm.js'))};
        export const calls = [], listeners = [], emits = [], parses = [];
        let run, subscribe, parse = source => real.parse(source);
        export const configureParser = fn => { parse = fn; };
        export const marked = { parse(source) { parses.push(source); return parse(source); } };
        export function configure(invoke, listen) { run = invoke; subscribe = listen; }
        export const emit = async () => {};
        export async function emitTo(target, name, payload) { emits.push({ target, name, payload }); }
        export async function invoke(command, args) { calls.push({ command, args }); return run(command, args); }
        export async function listen(name, receive, options) { const l = { name, receive, options, releases: 0 }; listeners.push(l); const release = () => l.releases++; return subscribe ? subscribe(l, release) : release; }
      ` : path === 'environment' ? `
        import { setTimeout, clearTimeout } from 'test-reader-clock';
        export const localStorage = { getItem: () => '', setItem() {} };
        export const writes = [], elements = name => ({ setAttribute: (key, value) => writes.push({ name, key, value }), removeAttribute: key => writes.push({ name, key, value: '' }) });
        export const document = { title: '', documentElement: elements('root'), body: elements('body') };
        export const window = { location: { search: ${JSON.stringify(query)}, hash: '' }, confirm: () => true, setTimeout, clearTimeout, localStorage };
      ` : path === 'clock' ? `
        let id = 0; export const clock = { active: new Map(), created: [] };
        const add = (callback, delay) => { const t = { id: ++id, callback, delay }; clock.active.set(t.id, t); clock.created.push(t); return t.id; };
        export const setTimeout = add, clearTimeout = id => clock.active.delete(id), setInterval = add, clearInterval = clearTimeout;
      ` : path === 'svelte' ? `
        let start; const cleanups = []; export const onMount = fn => { start = fn; }, onDestroy = fn => cleanups.push(fn), tick = async () => {};
        export const mount = () => start(), destroy = () => cleanups.forEach(fn => fn());
      ` : `export default { sanitize: raw => raw };`, loader: 'js', resolveDir: resolve('.') }));
    } }] });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#reader-${++instance}`);
}
