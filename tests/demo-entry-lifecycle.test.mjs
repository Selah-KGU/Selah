import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';

let instance = 0;
async function loadDemoEntry() {
  const source = await readFile(process.env.SELAH_LOGIN_API_BEFORE || 'src/lib/api.ts', 'utf8');
  const entry = source.match(/export async function enterDemoMode\([\s\S]*?\n}\n/)?.[0];
  if (!entry) throw new Error('Production demo entry missing');
  const result = await build({ stdin: { contents: `
    import { calls, stopBackgroundPolling, stopTrayStatus, sessionExpired, startBackgroundPolling, startTrayStatus } from 'test-demo-state';
    ${entry}
    export { calls };
  `, loader: 'ts' }, bundle: true, write: false, platform: 'node', format: 'esm',
    plugins: [{ name: 'demo-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^(test-demo-state|\.\/demo)$/ }, args => ({ path: args.path, namespace: 'demo-test' }));
      plugin.onLoad({ filter: /.*/, namespace: 'demo-test' }, ({ path }) => ({ contents: path === './demo'
        ? `import { calls } from 'test-demo-state'; export const activateDemo = () => calls.push('activateDemo');`
        : `export const calls = [];
          export const stopBackgroundPolling = () => calls.push('stopPolling'), stopTrayStatus = () => calls.push('stopTray');
          export const startBackgroundPolling = () => calls.push('startPolling'), startTrayStatus = () => calls.push('startTray');
          export const sessionExpired = { set: value => calls.push(['expired', value]) };`, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#demo-entry-${++instance}`);
}

test('demo entry from an already closed page performs no actions', async () => {
  const h = await loadDemoEntry(); await h.enterDemoMode(() => false); assert.deepEqual(h.calls, []);
});

test('closing while the demo module is loading prevents activation and background restarts', async () => {
  const h = await loadDemoEntry(); let current = true;
  const run = h.enterDemoMode(() => current); current = false; await run;
  assert.deepEqual(h.calls, ['stopPolling','stopTray',['expired',false]]);
});

test('default and current demo entry preserve the existing order of session, activation and tray updates', async () => {
  for (const guard of [undefined, () => true]) {
    const h = await loadDemoEntry(); await h.enterDemoMode(guard);
    assert.deepEqual(h.calls, ['stopPolling','stopTray',['expired',false],'activateDemo','startPolling','startTray']);
  }
});
