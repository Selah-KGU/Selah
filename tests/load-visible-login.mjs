import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
export async function loadVisibleLogin() {
  const source = await readFile(process.env.SELAH_VISIBLE_LOGIN_BEFORE || 'src/lib/api.ts', 'utf8');
  const publicEntry = source.match(/export (?:async )?function initiateRelogin\([\s\S]*?\n}\n/)?.[0];
  const privateEntry = source.match(/function openVisibleLogin\([\s\S]*?\n}\n/)?.[0];
  const owner = source.match(/let pendingRelogin:[^\n]+;/)?.[0] || '';
  if (!publicEntry || !privateEntry) throw new Error('Production re-login entry points missing');
  const result = await build({ stdin: { contents: `
    import { waitForVisibleLogin, type UniversityLoginComplete } from ${JSON.stringify(resolve('src/lib/visibleLogin.ts'))};
    import { configure, calls, listeners, state, warnings, listen, reloginInProgress, sessionExpired,
      setAuthFromSession, startBackgroundPolling, openLoginWindow, _isDemo } from 'test-visible-login';
    ${owner}
    ${publicEntry}
    ${privateEntry}
    export { configure, calls, listeners, state, warnings, openVisibleLogin };
  `, loader: 'ts', resolveDir: process.cwd() }, bundle: true, write: false, platform: 'node', format: 'esm',
    define: { 'console.warn': 'loginWarning' }, inject: ['test-visible-login-console'],
    plugins: [{ name: 'visible-login-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^test-visible-login$/ }, () => ({ path: 'backend', namespace: 'visible-login-test' }));
      plugin.onResolve({ filter: /^test-visible-login-console$/ }, () => ({ path: 'console', namespace: 'visible-login-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'visible-login-test' }, ({ path }) => ({ contents: path === 'backend' ? `
        export const calls = [], listeners = [], warnings = [];
        export const state = { running: false, expired: true, identity: null };
        let config = {};
        export const configure = value => { config = value; };
        export const _isDemo = () => !!config.demo;
        export const reloginInProgress = { set(value) { calls.push(['progress',value]); state.running = value; config.progress?.(value); } };
        export const sessionExpired = { set(value) { calls.push(['expired',value]); state.expired = value; } };
        export function setAuthFromSession(value) { if (config.identity) config.identity(value); state.identity = value; calls.push(['identity',value]); }
        export function startBackgroundPolling(refresh) { calls.push(['poll',state.running]); config.poll?.(refresh); }
        export async function openLoginWindow() { calls.push(['open']); return config.open?.(); }
        export async function listen(name, receive) {
          const listener = { name, receive, releases: 0 }; listeners.push(listener);
          const release = () => { listener.releases++; config.release?.(listener); };
          return config.listen ? config.listen(listener, release) : release;
        }
      ` : `import { warnings } from 'test-visible-login';
        export function loginWarning(...args) { warnings.push(args.map(arg => arg instanceof Error ? arg.message : String(arg)).join(' ')); }`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#visible-login-${++instance}`);
}
