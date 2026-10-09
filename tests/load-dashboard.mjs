import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
// Execute the production script; native subscriptions, stores, lifecycle hooks
// and component imports are the isolated boundaries. No application is started.
export async function loadDashboard() {
  const path = resolve('src/lib/Dashboard.svelte');
  const source = await readFile(process.env.SELAH_DASHBOARD_BEFORE || path, 'utf8');
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('Dashboard script missing');
  const result = await build({
    stdin: { contents: `
      const $state = value => value;
      const $effect = () => {};
      const $readIdsStore = { kgc: [], luna: [], kwic: [] };
      ${script}
      import { configure, calls, listeners, caches, state, warnings, readIdsCompletion } from 'test-dashboard-backend';
      import { mount, destroy } from 'svelte';
      export { configure, calls, listeners, caches, state, warnings, readIdsCompletion };
      export const dashboard = { mount, dispose: destroy, ensureViewLoaded,
        loader(tab, loader) { viewLoaders[tab] = loader; },
        get views() { return { lazyViews, lazyErrors }; },
      };
    `, loader: 'ts', resolveDir: dirname(path), sourcefile: path },
    bundle: true, write: false, platform: 'node', format: 'esm',
    define: { 'console.warn': 'dashboardWarning' }, inject: ['test-dashboard-console'],
    plugins: [{ name: 'dashboard-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(test-dashboard-backend|@tauri-apps\/api\/event|\.\/stores|\.\/api|\.\/onboarding\/onboardingState)$/ }, () => ({ path: 'backend', namespace: 'dashboard-test' }));
      plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'lifecycle', namespace: 'dashboard-test' }));
      plugin.onResolve({ filter: /^svelte\/store$/ }, () => ({ path: 'store', namespace: 'dashboard-test' }));
      plugin.onResolve({ filter: /^test-dashboard-console$/ }, () => ({ path: 'console', namespace: 'dashboard-test' }));
      plugin.onResolve({ filter: /\.svelte$/ }, () => ({ path: 'presentation', namespace: 'dashboard-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'dashboard-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        export const calls = [], listeners = [], caches = [], warnings = [], readIdsCompletion = [];
        export const state = { activeTab: 'home', onboardingVisible: false, liveTodoPending: true, liveTodoDrafts: null,
          unreadNotifCount: 0, unreadMailCount: 0, readIdsStore: { kgc: [], luna: [], kwic: [] } };
        let config = {};
        export function configure(value) { config = value; }
        const store = key => ({ get value() { return state[key]; }, set(value) { calls.push({ key, value }); state[key] = value; } });
        export const activeTab = store('activeTab'), onboardingVisible = store('onboardingVisible'),
          liveTodoPending = store('liveTodoPending'), liveTodoDrafts = store('liveTodoDrafts'),
          unreadNotifCount = store('unreadNotifCount'), unreadMailCount = store('unreadMailCount'), readIdsStore = store('readIdsStore');
        export async function listen(name, receive) {
          const listener = { name, receive, releases: 0 }; listeners.push(listener);
          const release = () => { listener.releases++; if (config.release) config.release(listener); };
          return config.listen ? config.listen(listener, release) : release;
        }
        export function onCacheUpdate(name, receive) { const cache = { name, receive, releases: 0 }; caches.push(cache); return () => cache.releases++; }
        export const getCached = key => config.cache?.[key], notifKey = (title, date) => title + date;
        export function loadReadIds() { return new Promise(resolve => readIdsCompletion.push(resolve)); }
        export function updateAiReadiness() { calls.push({ key: 'ai-readiness' }); return Promise.resolve(); }
        export function shouldAutoShow() { return config.onboarding ? config.onboarding() : Promise.resolve(false); }
        export const hasResume = () => false;
      ` : path === 'lifecycle' ? `
        let start; const cleanups = [];
        export const onMount = fn => { start = fn; }, onDestroy = fn => cleanups.push(fn);
        export const mount = () => { const pending = start(); if (pending) pending.catch(error => console.warn(error)); }, destroy = () => cleanups.forEach(fn => fn());
      ` : path === 'store' ? `export const get = store => store.value;`
        : path === 'console' ? `import { warnings } from 'test-dashboard-backend';
          export function dashboardWarning(...args) { warnings.push(args.map(arg => arg instanceof Error ? arg.message : String(arg)).join(' ')); }`
        : `export default {};`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#dashboard-${++instance}`);
}
