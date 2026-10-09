import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
// Execute the real script with native/UI/store and timer boundaries isolated.
export async function loadLogin() {
  const path = resolve('src/lib/Login.svelte');
  const source = await readFile(process.env.SELAH_LOGIN_BEFORE || path, 'utf8');
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('Login script missing');
  const result = await build({
    stdin: { contents: `
      const $state = value => value, navigator = { userAgent: 'macOS' };
      ${script}
      import { configure, calls, listeners, state, warnings, timers, liveTimers, queuedTimer, clearTimer } from 'test-login-backend';
      import { mount, destroy } from 'svelte';
      export { configure, calls, listeners, state, warnings, timers, liveTimers };
      export const login = { mount, dispose: destroy, handleLogin, handleLogoClick, startDemoMode, cancelDemoMode,
        minimizeWindow, toggleMaximize, closeWindow,
        get state() { return { showDemoConfirm, logoTapCount }; },
      };
    `, loader: 'ts', resolveDir: dirname(path), sourcefile: path },
    bundle: true, write: false, platform: 'node', format: 'esm',
    define: { 'console.warn': 'loginWarning', 'setTimeout': 'queuedTimer', 'clearTimeout': 'clearTimer' },
    inject: ['test-login-inject'],
    plugins: [{ name: 'login-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(test-login-backend|@tauri-apps\/api\/(event|window)|\.\/(api|stores))$/ }, () => ({ path: 'backend', namespace: 'login-test' }));
      plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'lifecycle', namespace: 'login-test' }));
      plugin.onResolve({ filter: /^test-login-inject$/ }, () => ({ path: 'inject', namespace: 'login-test' }));
      plugin.onResolve({ filter: /\.png$/ }, () => ({ path: 'image', namespace: 'login-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'login-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        export const calls = [], listeners = [], warnings = [], timers = [], liveTimers = new Set();
        export const state = { auth: { authenticated: false, loading: false, error: '', retained: 'unchanged' } };
        let config = {};
        export const configure = value => { config = value; };
        export const authState = { update(fn) { state.auth = fn(state.auth); calls.push({ name: 'auth', value: state.auth }); } };
        export function queuedTimer(callback, delay) { const timer = { callback, delay, clears: 0 }; timers.push(timer); liveTimers.add(timer); return timer; }
        export function clearTimer(timer) { timer.clears++; liveTimers.delete(timer); }
        export async function listen(name, receive) {
          const listener = { name, receive, releases: 0 }; listeners.push(listener);
          const release = () => { listener.releases++; if (config.release) config.release(listener); };
          return config.listen ? config.listen(listener, release) : release;
        }
        export function setAuthFromSession(session) { state.auth = { ...state.auth, ...session, authenticated: true, loading: false }; calls.push({ name: 'session', value: session }); }
        export const startBackgroundPolling = refresh => { calls.push({ name: 'startPolling' }); config.poll?.(refresh); };
        export async function openLoginWindow() { calls.push({ name: 'openLogin' }); return config.open ? config.open() : undefined; }
        export async function enterDemoMode(current) { calls.push({ name: 'enterDemo', current }); return config.demo ? config.demo() : undefined; }
        export function getCurrentWindow() { return Object.fromEntries(['minimize','toggleMaximize','close'].map(name => [name, async () => { calls.push({ name }); }])); }
      ` : path === 'lifecycle' ? `
        let start; const cleanups = [];
        export const onMount = fn => { start = fn; }, onDestroy = fn => cleanups.push(fn);
        export const mount = () => { const pending = start(); if (pending) pending.catch(error => console.warn(error)); }, destroy = () => cleanups.forEach(fn => fn());
      ` : path === 'inject' ? `import { warnings, queuedTimer, clearTimer } from 'test-login-backend';
          export { queuedTimer, clearTimer };
          export function loginWarning(...args) { warnings.push(args.map(arg => arg instanceof Error ? arg.message : String(arg)).join(' ')); }`
        : `export default 'logo';`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#login-${++instance}`);
}
