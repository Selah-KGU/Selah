import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
// Execute the actual component script and resource/read controllers. Native IPC,
// Svelte rendering, the clipboard and browser timers are isolated boundaries.
export async function loadDocumentTabs() {
  const path = resolve('src/lib/DocumentTabs.svelte');
  let script = (await readFile(path, 'utf8')).match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('DocumentTabs script missing');
  // Selection and effects are rendering boundaries. Tests explicitly supply
  // the derived selection and run the component's own effects after a change.
  for (const [before, after] of [
    ['const activeTab = $derived(tabs.find((tab) => tab.active) || null);', 'let activeTab: DocumentTab | null = null;'],
    ['const isHomeTab = $derived(activeTab?.type === "home");', 'let isHomeTab = false;'],
  ]) {
    if (!script.includes(before)) throw new Error('Selection boundary changed');
    script = script.replace(before, after);
  }
  const result = await build({
    stdin: { contents: `
      import { clock, setTimeout, clearTimeout, setInterval, clearInterval } from 'test-tabs-clock';
      const window = Object.assign(new EventTarget(), {
        location: { search: '', hash: '' }, localStorage: { getItem: () => 'light' },
        setTimeout, clearTimeout, setInterval, clearInterval,
      });
      const themeWrites = [];
      const element = name => ({ setAttribute: (key, value) => themeWrites.push({ name, key, value }), removeAttribute: key => themeWrites.push({ name, key, value: '' }) });
      const document = Object.assign(new EventTarget(), { hidden: false, documentElement: element('root'), body: element('body') });
      const navigator = { userAgent: 'test', clipboard: { writeText: value => clipboard(value) } };
      const $state = value => value;
      const $derived = Object.assign(value => value, { by: read => read() });
      const effects = [];
      const $effect = fn => effects.push(fn);
      ${script}
      import { configure, configureClipboard, clipboard, calls, listeners } from 'test-tabs-backend';
      import { mount, destroy } from 'svelte';
      export { configure, configureClipboard, calls, listeners, clock, themeWrites };
      export const panel = {
        mount, dispose: destroy, refresh: refreshTabs, run, search: onFilesSearchInput,
        theme: syncThemeFromApp, toggleAgent,
        control: sendControl, external: browserExternal, copy: copyActiveUrl,
        navigate: submitAddress,
        setAddress(value) { address = value; },
        select(row) { activeTab = row; isHomeTab = row?.type === 'home'; effects.forEach(fn => fn()); },
        searchFor(value) { filesSearch = value; onFilesSearchInput(); },
        pointerDown: onTabPointerDown, pointerMove: onTabPointerMove, pointerUp: onTabPointerUp, pointerCancel: onTabPointerCancel,
        hide(value) { document.hidden = value; document.dispatchEvent(new Event('visibilitychange')); },
        get state() { return { tabs, error, busy, agentOpen, titleHints, copied }; },
      };
    `, loader: 'ts', resolveDir: dirname(path), sourcefile: path },
    bundle: true, write: false, platform: 'node', format: 'esm',
    plugins: [{ name: 'tabs-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|test-tabs-backend)$/ }, () => ({ path: 'backend', namespace: 'tabs-test' }));
      plugin.onResolve({ filter: /^test-tabs-clock$/ }, () => ({ path: 'clock', namespace: 'tabs-test' }));
      plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'svelte', namespace: 'tabs-test' }));
      plugin.onResolve({ filter: /^@tauri-apps\/api\/window$/ }, () => ({ path: 'window', namespace: 'tabs-test' }));
      plugin.onResolve({ filter: /^\.\/bookmarks$/ }, () => ({ path: 'bookmarks', namespace: 'tabs-test' }));
      plugin.onResolve({ filter: /^\.\/documentTabs$/ }, () => ({ path: 'favicon', namespace: 'tabs-test' }));
      plugin.onResolve({ filter: /\.svelte$|\.png$/ }, () => ({ path: 'presentation', namespace: 'tabs-test' }));
      plugin.onLoad({ filter: /resourceScope\.ts$/ }, async ({ path }) => ({ contents: `import { setTimeout, clearTimeout, setInterval, clearInterval } from 'test-tabs-clock';\n${await readFile(path, 'utf8')}`, loader: 'ts' }));
      plugin.onLoad({ filter: /.*/, namespace: 'tabs-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        export const calls = [], listeners = [];
        let run, subscribe;
        let copy = async () => {};
        export const clipboard = value => copy(value);
        export const configureClipboard = fn => { copy = fn; };
        export function configure(invoke, listen) { run = invoke; subscribe = listen; }
        export const emit = async () => {};
        export async function invoke(command, args) { calls.push({ command, args }); return run(command, args); }
        export async function listen(name, receive) {
          const listener = { name, receive, releases: 0 }; listeners.push(listener);
          const release = () => listener.releases++;
          return subscribe ? subscribe(listener, release) : release;
        }
      ` : path === 'clock' ? `
        let id = 0;
        export const clock = { active: new Map(), created: [], cleared: [] };
        const add = (kind, callback, delay) => { const timer = { id: ++id, kind, callback, delay }; clock.active.set(id, timer); clock.created.push(timer); return id; };
        const remove = id => { clock.active.delete(id); clock.cleared.push(id); };
        export const setTimeout = (fn, delay) => add('timeout', fn, delay), clearTimeout = remove;
        export const setInterval = (fn, delay) => add('interval', fn, delay), clearInterval = remove;
      ` : path === 'svelte' ? `
        let start; const cleanups = [];
        export const onMount = fn => { start = fn; }, onDestroy = fn => cleanups.push(fn);
        export const mount = () => start(), destroy = () => cleanups.forEach(fn => fn());
      ` : path === 'window' ? `export const getCurrentWindow = () => ({});`
        : path === 'bookmarks' ? `export const listBookmarks = () => [], toggleBookmark = () => {};`
        : path === 'favicon' ? `export const tabFaviconUrl = () => '';`
        : `export default {};`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#tabs-${++instance}`);
}
