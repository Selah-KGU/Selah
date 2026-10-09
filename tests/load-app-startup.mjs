import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
export async function loadAppStartup(boot = {}) {
  const path = resolve('src/App.svelte');
  const source = await readFile(process.env.SELAH_APP_STARTUP_BEFORE || path, 'utf8');
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('App script missing');
  const result = await build({
    stdin: { contents: `
      const storage = new Map(Object.entries(${JSON.stringify(boot)}));
      const localStorage = { getItem: key => storage.get(key) ?? null, removeItem: key => storage.delete(key) };
      const $state = value => value, $derived = value => value;
      const $demoMode = false, $authState = { authenticated: false }, $sessionExpired = false;
      ${script}
      import { configure, calls, listeners, state, warnings } from 'test-startup-backend';
      import { mount, destroy } from 'svelte';
      export { configure, calls, listeners, state, warnings, storage };
      export const app = { mount, dispose: destroy,
        get state() { return { demoBootFlag, everLoggedIn, restoring }; },
      };
    `, loader: 'ts', resolveDir: dirname(path), sourcefile: path },
    bundle: true, write: false, platform: 'node', format: 'esm',
    define: { 'console.warn': 'startupWarning' }, inject: ['test-startup-console'],
    plugins: [{ name: 'startup-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(test-startup-backend|@tauri-apps\/api\/event|\.\/lib\/(stores|api|demoStore|demo|trayStatus|updater))$/ }, () => ({ path: 'backend', namespace: 'startup-test' }));
      plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'lifecycle', namespace: 'startup-test' }));
      plugin.onResolve({ filter: /^svelte\/store$/ }, () => ({ path: 'store', namespace: 'startup-test' }));
      plugin.onResolve({ filter: /^test-startup-console$/ }, () => ({ path: 'console', namespace: 'startup-test' }));
      plugin.onResolve({ filter: /\.(svelte|css)$/ }, () => ({ path: 'presentation', namespace: 'startup-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'startup-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        export const calls = [], listeners = [], warnings = [];
        export const state = { activeTab: 'home', sessionExpired: false, authState: { authenticated: false }, demoMode: false };
        let config = {};
        export const configure = value => { config = value; };
        const store = key => ({ get value() { return state[key]; }, set(value) { state[key] = value; calls.push({ name: key, value }); } });
        export const activeTab = store('activeTab'), sessionExpired = store('sessionExpired'), authState = store('authState'), demoMode = store('demoMode');
        export async function listen(name, receive) {
          const listener = { name, receive, releases: 0 }; listeners.push(listener);
          const release = () => listener.releases++;
          return config.listen ? config.listen(listener, release) : release;
        }
        export function restoreDemo() { calls.push({ name: 'restoreDemo' }); state.demoMode = !!config.demo; return state.demoMode; }
        export const isDemoMode = () => state.demoMode;
        export function deactivateDemo() { calls.push({ name: 'deactivateDemo' }); state.demoMode = false; }
        export async function restoreAllSessions(current) { calls.push({ name: 'restoreAllSessions', current }); return config.restore ? config.restore() : { valid: true }; }
        export async function liveHasActiveSession() { calls.push({ name: 'live_has_active_session' }); return config.live ? config.live() : false; }
        export async function liveGetSession() { calls.push({ name: 'live_get_session' }); return { active: config.live ? await config.live() : false }; }
        export const serviceRegistry = Object.fromEntries(['kgc','luna','kwic','mail'].map(name => [name, { onReset() { calls.push({ name: 'reset:' + name }); state.authState = { authenticated: false }; } }]));
        export const invalidateCache = () => calls.push({ name: 'invalidateCache' });
        export const startBackgroundPolling = () => calls.push({ name: 'startPolling' });
        export const stopBackgroundPolling = () => calls.push({ name: 'stopPolling' });
        export const startTrayStatus = () => calls.push({ name: 'startTray' });
        export const stopTrayStatus = () => calls.push({ name: 'stopTray' });
        export const startSilentUpdateCheck = async () => { calls.push({ name: 'checkUpdate' }); };
      ` : path === 'lifecycle' ? `
        let start; const cleanups = [];
        export const onMount = fn => { start = fn; }, onDestroy = fn => cleanups.push(fn);
        export const mount = () => { const pending = start(); if (pending) pending.catch(error => console.warn(error)); }, destroy = () => cleanups.forEach(fn => fn());
      ` : path === 'store' ? `export const get = store => store.value;`
        : path === 'console' ? `import { warnings } from 'test-startup-backend'; export function startupWarning(...args) { warnings.push(args.map(arg => arg instanceof Error ? arg.message : String(arg)).join(' ')); }`
        : `export default {};`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#app-startup-${++instance}`);
}
