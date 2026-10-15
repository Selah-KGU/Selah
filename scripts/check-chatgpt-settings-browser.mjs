// Exercise the production Svelte UI against synthetic native IPC. No account,
// credentials, installed Selah app, or external network service is accessed.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';

const transport = `
export let accounts = [], calls = [], pending = null, failModels = false;
export let demo = false;
export function setAccounts(next) { accounts = next; }
export function setDemo(next) { demo = next; }
export function setModelFailure(next) { failModels = next; }
export function finishLogin(next) { accounts = next; const resolve = pending; pending = null; resolve(); }
export function recordTest() { calls.push({ command: 'test-connection' }); }
export async function invoke(command, args) {
  calls.push({ command, args });
  if (command === 'chatgpt_accounts') return JSON.parse(JSON.stringify(accounts));
  if (command === 'chatgpt_sign_in') return new Promise(resolve => { pending = resolve; });
  if (command === 'chatgpt_cancel_login') { const resolve = pending; pending = null; resolve?.(); return; }
  if (command === 'chatgpt_select_account') { accounts = accounts.map(a => ({ ...a, active: a.client_id === args.clientId })); return; }
  if (command === 'chatgpt_disconnect') { accounts = accounts.map(a => a.client_id === args.clientId ? { ...a, active: false, connected: false, plan_enabled: false } : a); return false; }
  if (command === 'chatgpt_list_models') {
    if (failModels) throw new Error('Synthetic model-list outage');
    return accounts.some(a => a.active && a.client_id === 'oaiapp_second')
      ? [{ slug: 'second-model', display_name: 'Second account model' }]
      : [{ slug: 'first-model', display_name: 'First account model' }];
  }
  throw new Error('Unexpected native command: ' + command);
}
`;

const source = `
import { mount, unmount, tick } from 'svelte';
import Settings from ${JSON.stringify(resolve('src/lib/views/settings/ChatGptSettings.svelte'))};
import { calls, pending, setAccounts, setDemo, finishLogin, setModelFailure, recordTest } from 'chatgpt-ui-transport';
import { isChatGptReady } from ${JSON.stringify(resolve('src/lib/chatgptApi.ts'))};
const checks = [], target = document.getElementById('probe');
let app;
const account = (id, active, enabled = true) => ({ client_id: id, email: 'same@example.com', active, connected: true, plan_enabled: enabled });
function check(ok, label) { if (!ok) throw new Error(label); checks.push(label); }
async function until(fn) {
  for (let i = 0; i < 200; i++) { await tick(); if (fn()) return; await new Promise(resolve => setTimeout(resolve, 5)); }
  throw new Error('DOM condition timed out');
}
function button(text) { return [...target.querySelectorAll('button')].find(b => b.textContent.includes(text)); }
async function show() {
  if (app) await unmount(app);
  app = mount(Settings, { target, props: { model: 'unavailable-old-model', testConnection: async () => recordTest() } });
  await until(() => button('Continue with ChatGPT') && !button('Continue with ChatGPT').disabled);
}
try {
  setAccounts([]); await show();
  check(!target.querySelector('#chatgpt-model') && !target.querySelector('input[type=password]'), 'disconnected UI needs browser sign-in and never asks for a token');
  check(!(await isChatGptReady()), 'no ChatGPT account is not ready');
  button('Continue with ChatGPT').click();
  await until(() => pending && button('キャンセル'));
  check(calls.at(-1).args.clientId === null, 'first sign-in uses dynamic registration');
  button('キャンセル').click();
  await until(() => !button('Continue with ChatGPT').disabled);
  check(calls.some(c => c.command === 'chatgpt_cancel_login'), 'pending sign-in can be cancelled');
  button('Continue with ChatGPT').click(); await until(() => pending);
  finishLogin([account('oaiapp_first', true)]);
  await until(() => target.querySelector('#chatgpt-model')?.value === 'first-model' && !button('Continue with ChatGPT').disabled);
  check(await isChatGptReady(), 'active account with plan consent enables AI');
  check(target.textContent.includes('プラン利用が有効'), 'connected status confirms granted plan usage');
  check(target.querySelector('a').href === 'https://chatgpt.com/settings/usage', 'usage management points to ChatGPT settings');
  button('接続テスト').click(); await tick();
  check(calls.some(c => c.command === 'test-connection'), 'selected model can be tested');
  setAccounts([account('oaiapp_first', true), account('oaiapp_second', false)]);
  await unmount(app); app = null; await show();
  const picker = target.querySelector('#chatgpt-account');
  check(picker.options[0].textContent !== picker.options[1].textContent, 'same-email registrations have distinct labels');
  picker.value = 'oaiapp_second'; picker.dispatchEvent(new Event('change', { bubbles: true }));
  await until(() => target.querySelector('#chatgpt-model')?.value === 'second-model' && !button('Continue with ChatGPT').disabled);
  check(calls.some(c => c.command === 'chatgpt_select_account' && c.args.clientId === 'oaiapp_second'), 'saved-account switching refreshes the account-specific catalog');
  button('ログアウト').click();
  await until(() => target.textContent.includes('サーバー側の解除を確認できない'));
  check(!(await isChatGptReady()) && !target.querySelector('#chatgpt-model'), 'logout disables AI and explains unconfirmed remote revocation');
  setAccounts([account('oaiapp_first', true, false)]);
  await unmount(app); app = null; await show();
  check(!(await isChatGptReady()) && !target.querySelector('#chatgpt-model'), 'identity-only grant never enables inference');
  button('Continue with ChatGPT').click(); await until(() => pending);
  check(calls.at(-1).args.clientId === 'oaiapp_first', 'reauthorization reuses the issued registration');
  await unmount(app); app = null;
  check(calls.at(-1).command === 'chatgpt_cancel_login', 'leaving settings cancels its pending sign-in');
  await until(() => !pending);
  setAccounts([account('oaiapp_first', true)]); setModelFailure(true); await show();
  await until(() => target.querySelector('[role=alert]'));
  check(target.textContent.includes('Synthetic model-list outage'), 'model-list failure is visible and retryable');
  check(button('接続テスト').disabled, 'catalog failure cannot test a leftover model from another account');
  setModelFailure(false); button('更新').click();
  await until(() => target.querySelector('#chatgpt-model')?.value === 'first-model' && !button('更新').disabled);
  check(!target.querySelector('[role=alert]'), 'catalog retry clears the previous error');
  await unmount(app); app = null; setDemo(true);
  app = mount(Settings, { target, props: { model: '', testConnection: async () => recordTest() } });
  await until(() => target.textContent.includes('デスクトップ版'));
  check(button('Continue with ChatGPT').disabled, 'demo does not start native authentication');
  await unmount(app); app = null;
  await fetch('/result', { method: 'POST', body: JSON.stringify({ passed: checks.length, checks }) });
} catch (error) {
  await fetch('/result', { method: 'POST', body: JSON.stringify({ passed: checks.length, checks, error: String(error) }) });
}
`;

const bundle = await build({
  stdin: { contents: source, loader: 'js', resolveDir: process.cwd() },
  bundle: true, write: false, platform: 'browser', format: 'esm', conditions: ['browser'],
  plugins: [{ name: 'chatgpt-ui-fixture', setup(builder) {
    builder.onResolve({ filter: /^(@tauri-apps\/api\/core|chatgpt-ui-transport)$/ }, () => ({ path: 'transport', namespace: 'chatgpt-fixture' }));
    builder.onResolve({ filter: /^\.\.\/\.\.\/api$/ }, () => ({ path: 'api', namespace: 'chatgpt-fixture' }));
    builder.onResolve({ filter: /^\.\/system$/ }, () => ({ path: 'system', namespace: 'chatgpt-fixture' }));
    builder.onLoad({ filter: /.*/, namespace: 'chatgpt-fixture' }, ({ path }) => ({
      contents: path === 'transport' ? transport : path === 'system' ? 'export async function openExternalUrl() {}'
        : 'import { demo } from "chatgpt-ui-transport"; export const isDemoActive = () => demo; export async function updateAiReadiness() {}',
      loader: 'js', resolveDir: resolve('.'),
    }));
    builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
      contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client', css: 'injected' }).js.code,
      loader: 'js', resolveDir: dirname(path),
    }));
  } }],
});

let finish;
const result = new Promise(resolve => { finish = resolve; });
const server = createServer(async (request, response) => {
  if (request.url === '/probe.js') {
    response.setHeader('Content-Type', 'text/javascript'); response.end(bundle.outputFiles[0].text);
  } else if (request.url === '/result' && request.method === 'POST') {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    finish(JSON.parse(Buffer.concat(chunks).toString())); response.end('ok');
  } else {
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><meta charset="utf-8"><title>ChatGPT settings regression</title><div id="probe"></div><script type="module" src="/probe.js"></script>');
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const profile = await mkdtemp(resolve(tmpdir(), 'selah-chatgpt-ui-'));
const chrome = spawn(process.env.CHROME_BIN || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', [
  '--headless', '--disable-gpu', '--disable-extensions', '--disable-background-networking', '--no-first-run',
  '--user-data-dir=' + profile, 'http://127.0.0.1:' + server.address().port,
], { stdio: 'ignore' });
let timer;
try {
  const outcome = await Promise.race([
    result,
    new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Browser checks timed out')), 25000); chrome.once('error', reject); }),
  ]);
  console.log(JSON.stringify(outcome, null, 2));
  if (outcome.error) process.exitCode = 1;
} finally {
  clearTimeout(timer); chrome.kill();
  server.closeAllConnections(); server.close();
  await new Promise(resolve => chrome.once('exit', resolve));
  await rm(profile, { recursive: true, force: true });
}
