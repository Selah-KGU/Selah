import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
async function fixture() {
  const source = await readFile('src/lib/api.ts', 'utf8');
  const readiness = source.slice(source.indexOf('let _aiReadyCache:'), source.indexOf('export async function aiChat('));
  const result = await build({
    stdin: { contents: `
      import { requests, state, invoke } from 'readiness-boundary';
      const _isDemo = () => state.demo;
      const getAiConfig = () => invoke('get_ai_config');
      const aiReady = { set(value) { state.ai = value; } };
      const agentReady = { set(value) { state.agent = value; } };
      ${readiness}
      export { requests, state };
    `, loader: 'ts', resolveDir: resolve('src/lib') },
    bundle: true, write: false, platform: 'node', format: 'esm',
    plugins: [{ name: 'readiness-boundaries', setup(builder) {
      builder.onResolve({ filter: /^(readiness-boundary|@tauri-apps\/api\/core)$/ }, () => ({ path: 'boundary', namespace: 'readiness-test' }));
      builder.onLoad({ filter: /.*/, namespace: 'readiness-test' }, () => ({ loader: 'js', contents: `
        export const requests = [], state = { demo: false, ai: null, agent: null };
        export function invoke(command) {
          let resolve, reject;
          const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
          requests.push({ command, resolve, reject }); return promise;
        }
      ` }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#readiness-${++instance}`);
}

async function until(fn) {
  for (let i = 0; i < 30; i++) { if (fn()) return; await new Promise(setImmediate); }
  assert.fail('Expected native request was not issued');
}
const connected = [{ active: true, connected: true, plan_enabled: true }];
const config = { ai_enabled: true, provider: 'chatgpt', api_key: 'unrelated-api-key' };

test('late account reads cannot restore readiness after logout', async () => {
  const f = await fixture();
  const old = f.isAiReady();
  f.requests[0].resolve(config);
  await until(() => f.requests.length === 2);
  const update = f.updateAiReadiness();
  f.requests[2].resolve(config);
  await until(() => f.requests.length === 4);
  f.requests[3].resolve([]);
  await update;
  assert.equal(f.state.ai, false);
  f.requests[1].resolve(connected);
  await old;
  assert.equal(await f.isAiReady(), false);
});

test('obsolete settings reads cannot overwrite a newer disabled state', async () => {
  const f = await fixture();
  const old = f.updateAiReadiness();
  const latest = f.updateAiReadiness();
  f.requests[1].resolve({ ai_enabled: false });
  await latest;
  f.requests[0].resolve({ ai_enabled: true, provider: 'openai', api_key: 'saved-key' });
  await old;
  assert.equal(f.state.ai, false);
  assert.equal(f.state.agent, false);
});

test('ChatGPT readiness requires an active account with plan consent, regardless of API key', async () => {
  for (const [accounts, expected] of [
    [[], false],
    [[{ active: true, connected: true, plan_enabled: false }], false],
    [[{ active: false, connected: true, plan_enabled: true }], false],
    [[{ active: true, connected: false, plan_enabled: true }], false],
    [connected, true],
  ]) {
    const f = await fixture();
    const pending = f.isAiReady();
    f.requests[0].resolve(config);
    await until(() => f.requests.length === 2);
    assert.equal(f.requests[1].command, 'chatgpt_accounts');
    f.requests[1].resolve(accounts);
    assert.equal(await pending, expected);
    assert.equal(await f.isAiReady(), expected);
    assert.equal(f.requests.length, 2);
  }
});

test('demo readiness preserves the disabled agent behavior', async () => {
  const f = await fixture();
  f.state.demo = true;
  const update = f.updateAiReadiness();
  f.requests[0].resolve({ ai_enabled: true });
  await update;
  assert.equal(f.state.ai, true);
  assert.equal(f.state.agent, false);
});
