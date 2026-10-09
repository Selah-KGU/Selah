import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
// Run the actual dock script. Svelte rendering, native transports and the
// browser visibility surface are isolated; no real window is opened or closed.
export async function loadCopilotDock() {
  const path = resolve('src/lib/CopilotDock.svelte');
  const script = (await readFile(path, 'utf8')).match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('CopilotDock script missing');
  const result = await build({
    stdin: { contents: `
      const document = Object.assign(new EventTarget(), { hidden: false });
      const $state = value => value, $derived = value => value;
      ${script}
      import { configure, calls, listeners, steps } from 'test-dock-backend';
      import { mount, destroy } from 'svelte';
      export { configure, calls, listeners, steps };
      export const dock = { mount, dispose: destroy, refresh, run, closeTab, revealTab, newTab,
        hide(value) { document.hidden = value; document.dispatchEvent(new Event('visibilitychange')); },
        get state() { return { tabs, busy, docHidden }; },
      };
    `, loader: 'ts', resolveDir: dirname(path), sourcefile: path },
    bundle: true, write: false, platform: 'node', format: 'esm',
    plugins: [{ name: 'dock-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|test-dock-backend)$/ }, () => ({ path: 'backend', namespace: 'dock-test' }));
      plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'svelte', namespace: 'dock-test' }));
      plugin.onResolve({ filter: /\.svelte$/ }, () => ({ path: 'presentation', namespace: 'dock-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'dock-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        export const calls = [], listeners = [], steps = [];
        let run, subscribe;
        export function configure(invoke, listen) { run = invoke; subscribe = listen; }
        export async function invoke(command, args) { steps.push(command); calls.push({ command, args }); return run(command, args); }
        export async function listen(name, receive) {
          steps.push(name); const listener = { name, receive, releases: 0 }; listeners.push(listener);
          const release = () => listener.releases++;
          return subscribe ? subscribe(listener, release) : release;
        }
      ` : path === 'svelte' ? `
        let start; const cleanups = [];
        export const onMount = fn => { start = fn; }, onDestroy = fn => cleanups.push(fn);
        export const mount = () => start(), destroy = () => cleanups.forEach(fn => fn());
      ` : `export default {};`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#dock-${++instance}`);
}
