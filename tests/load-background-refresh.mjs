import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
export async function loadBackgroundRefresh() {
  const source = await readFile(process.env.SELAH_BACKGROUND_REFRESH_BEFORE || 'src/lib/api.ts', 'utf8');
  const fn = name => {
    const body = source.match(new RegExp(`(?:export )?(?:async )?function ${name}\\([^]*?\\n}\\n`))?.[0];
    if (!body) throw new Error(`Production function missing: ${name}`);
    return body;
  };
  const owners = source.match(/const backendAiStatusRead =[^]*?\n\);/)?.[0] || '';
  const sessionCooldown = source.match(/let lastForegroundSessionSyncAt[^]*?const FOREGROUND_SESSION_SYNC_COOLDOWN_MS[^\n]+;/)?.[0];
  const background = source.split('const TASK_LABELS:')[1]?.split('// ============ Backend AI Refresh')[0];
  if (!background || !sessionCooldown) throw new Error('Production background refresh section missing');
  const events = ['backend-ai-refresh-status', 'backend-session-status', 'backend-cache-updated'].map(name => {
    const handler = source.match(new RegExp(`  listen(?:<[^\\n]*>)?\\("${name}", \\(event\\) => \\{[^]*?\\n  \\}\\);`))?.[0];
    if (!handler) throw new Error(`Production event missing: ${name}`);
    return handler;
  }).join('\n');
  const result = await build({ stdin: { loader: 'ts', resolveDir: process.cwd(), contents: `
    import { BackgroundRefresh } from ${JSON.stringify(resolve('src/lib/backgroundRefresh.ts'))};
    import { CoalescedStatusRead } from ${JSON.stringify(resolve('src/lib/coalescedStatusRead.ts'))};
    import { BackendTaskStatusReader } from ${JSON.stringify(resolve('src/lib/backendTaskStatus.ts'))};
    import { configure, calls, requests, callbacks, state, warnings, document, localStorage, listen, invoke,
      _isDemo, getScheduleSnapshot, syncBackendManagedKeys, BACKEND_CACHE_DB_KEY, registerTask, updateTask,
      updateTaskInterval, aiRefreshing, lunaAuthState, kwicAuthState, sessionExpired, mailAuthState,
      setAuthFromSession } from 'test-background-boundaries';
    ${owners}
    ${['applyBackendAiRefreshStatus', 'applyBackendSessionStatus', 'getBackendAiRefreshStatus', 'backendAiRefreshNow',
      'refreshBackendAiTaskStatus', 'getDataCacheUpdatedAt', 'refreshVisibleBackendCaches',
      'syncBackendSessionStatusNow', 'syncForegroundSessionStatus'].map(fn).join('\n')}
    ${sessionCooldown}
    const TASK_LABELS:${background}
    ${events}
    export { configure, calls, requests, callbacks, state, warnings, document, syncBackendSessionStatusNow };
  ` }, bundle: true, write: false, platform: 'node', format: 'esm',
    define: { 'console.warn': 'backgroundWarning', 'Date.now': 'foregroundNow' }, inject: ['test-background-console'],
    plugins: [{ name: 'background-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^test-background-(boundaries|console)$/ }, ({ path }) => ({ path, namespace: 'background-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'background-test' }, ({ path }) => ({ loader: 'js', contents: path === 'test-background-console'
        ? `import { warnings, foregroundNow } from 'test-background-boundaries'; export { foregroundNow }; export const backgroundWarning = (...args) => warnings.push(args.map(v => v instanceof Error ? v.message : String(v)).join(' '));`
        : `
          export const calls = [], requests = [], callbacks = new Map(), warnings = [];
          export const state = { tasks: new Map(), updates: [], ai: null, session: {}, identity: null };
          let config = {};
          export const configure = value => { config = value; };
          export const foregroundNow = () => config.now ?? 100000;
          export const _isDemo = () => !!config.demo;
          export const localStorage = { getItem: () => config.demo ? '1' : null };
          export const document = {
            visibilityState: 'visible', listeners: new Set(), registrations: [],
            addEventListener(name, callback) { calls.push(['add',name]); this.listeners.add(callback); this.registrations.push(callback); },
            removeEventListener(name, callback) { calls.push(['remove',name]); this.listeners.delete(callback); },
            emit() { for (const callback of [...this.listeners]) callback(); }
          };
          export function listen(name, callback) { callbacks.set(name, callback); return Promise.resolve(() => {}); }
          export function invoke(name, args) {
            let resolve, reject;
            const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
            requests.push({ name, args, resolve, reject });
            return promise;
          }
          export const getScheduleSnapshot = () => config.demo
            ? Promise.resolve({snapshot_updated_at:foregroundNow()}) : invoke('get_schedule_snapshot');
          export const BACKEND_CACHE_DB_KEY = { exams: 'exam_timetable' };
          export const syncBackendManagedKeys = (keys, stale) => { calls.push(['cache',keys,stale]); return Promise.resolve(); };
          export function registerTask(key, label, tier, intervalMs) {
            calls.push(['register',key]);
            if (!state.tasks.has(key)) state.tasks.set(key,{key,label,tier,intervalMs});
          }
          export function updateTask(key, patch) { state.updates.push([key,patch]); Object.assign(state.tasks.get(key) || {},patch); }
          export function updateTaskInterval(key, intervalMs) { Object.assign(state.tasks.get(key) || {},{intervalMs}); }
          export const aiRefreshing = { set(value) { state.ai = value; config.ai?.(value); } };
          const store = key => ({set(value) {state.session[key] = value;}});
          export const lunaAuthState = store('luna'), kwicAuthState = store('kwic'), sessionExpired = store('expired'), mailAuthState = store('mail');
          export const setAuthFromSession = value => { state.identity = value; };
        ` }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#background-${++instance}`);
}
