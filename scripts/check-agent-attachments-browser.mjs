// Mount production Svelte components with native IPC replaced by synthetic
// in-memory documents. Real FileReader and rendered DOM are exercised. This
// never accesses user files, cloud APIs, microphone or the installed app.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { createServer } from 'node:http';

const transport = `
export const calls = [], listeners = [], pending = [];
export let rows = [];
export const body = '授業の資料 🌕 & <script>unsafe</script>\\n' + '🌕'.repeat(2500);
export function reset() { calls.length = 0; rows = []; pending.length = 0; }
export function complete() {
  const item = pending.shift();
  if (item.name === 'scan.pdf') item.reject('PDFに読み取れる文字がありません');
  else item.resolve({ name: item.name, mime: 'text/plain', size: 8, text: body, truncated: item.name === 'partial.txt' });
}
export async function invoke(command, args) {
  // Native IPC serializes reactive proxies rather than cloning proxy objects.
  args = args ? JSON.parse(JSON.stringify(args)) : args;
  calls.push({ command, args });
  if (command === 'get_app_theme') return 'light';
  if (command === 'agent_active_conversation' || command === 'agent_create_conversation') return 'fixture';
  if (command === 'agent_list_conversations') return [{ id: 'fixture', title: '添付の検証', created_at: 1, updated_at: 1 }];
  if (command === 'agent_load_display_messages') return JSON.parse(JSON.stringify(rows));
  if (command === 'agent_set_active_conversation' || command === 'agent_cancel') return;
  if (command === 'agent_read_document_attachment') return new Promise((resolve, reject) => pending.push({ ...args, resolve, reject }));
  if (command === 'agent_send' || command === 'agent_send_with_context') {
    rows.push({ id: rows.length + 1, conv_id: 'fixture', role: 'user', content: args.content, documents: args.documents, images: args.images, created_at: 1 });
    return;
  }
  throw new Error('Unexpected fixture command: ' + command);
}
export async function listen(name, receive) {
  const item = { name, receive, active: true }; listeners.push(item);
  return () => { item.active = false; };
}
export async function emit() {}
`;
const source = `
import { mount, unmount, tick } from 'svelte';
import Chat from ${JSON.stringify(resolve('src/lib/views/AgentChat.svelte'))};
import Panel from ${JSON.stringify(resolve('src/lib/AgentPanel.svelte'))};
import { calls, pending, complete, reset, body, listeners } from 'attachment-probe-transport';
const target = document.getElementById('probe'), checks = [];
let app = null;
const surfaces = [
  { name: 'main', component: Chat, composer: '.composer-bottom', action: '.action-capsule', cards: '.chat-attachment', history: '.msg-list', width: 960 },
  { name: 'sidebar', component: Panel, composer: '.agent-composer-wrap', action: '.agent-action-capsule', cards: '.agent-attachment', history: '.agent-messages', width: 380 },
];
function check(ok, label) { if (!ok) throw new Error(label); checks.push(label); }
async function until(ready) {
  for (let i = 0; i < 150; i++) { await tick(); if (ready()) return; await new Promise(requestAnimationFrame); }
  throw new Error('DOM condition timed out');
}
async function show(surface) {
  if (app) { await unmount(app); app = null; }
  target.style.width = surface.width + 'px';
  app = mount(surface.component, { target });
  await until(() => calls.some(c => c.command === 'agent_load_display_messages') && target.querySelector('textarea'));
  await tick();
}
function pick(files) {
  const transfer = new DataTransfer(); files.forEach(file => transfer.items.add(file));
  const input = target.querySelector('input[type=file]');
  input.files = transfer.files; input.dispatchEvent(new Event('change', { bubbles: true }));
}
function file(name, data = '日本語') { return new File([data], name); }
document.getElementById('run').onclick = async () => {
  document.getElementById('run').disabled = true;
  let error = null;
  try {
    for (const surface of surfaces) {
      reset(); await show(surface);
      const composer = () => target.querySelector(surface.composer);
      const action = () => target.querySelector(surface.action);
      const status = () => composer().querySelector('.attachment-status');
      const cards = () => [...composer().querySelectorAll(surface.cards)];
      const label = text => surface.name + ': ' + text;
      check(composer().querySelector('.attachment-help').textContent.includes('PDF') && composer().querySelector('.attachment-help').textContent.includes('最大4件・1件10MB'), label('persistent formats and limits'));
      check(target.querySelector('input[type=file]').accept.includes('.xlsx'), label('expanded native picker accept'));
      pick([file('資料.txt')]);
      await until(() => pending.length === 1);
      check(status().textContent.includes('読み込み中') && !status().textContent.includes('準備できました'), label('pending parse shown in actual DOM'));
      check(action().disabled, label('pending parse disables composer action'));
      composer().querySelector('textarea').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); await tick();
      check(!calls.some(c => c.command === 'agent_send'), label('Enter cannot send before extraction'));
      check(target.querySelector('input[type=file]').value === '', label('input reset allows same-file retry'));
      complete(); await until(() => cards().length === 1);
      check(status().textContent.includes('添付1件を準備できました') && !action().disabled, label('ready count and send action'));
      check(cards()[0].textContent.includes('資料.txt') && cards()[0].textContent.includes('本文の読み取り完了'), label('filename and parsed status'));
      cards()[0].querySelector('summary').click(); await tick();
      const preview = cards()[0].querySelector('pre').textContent;
      check(preview === Array.from(body).slice(0, 2000).join('') + '\\n…' && !preview.includes('�'), label('preview caps Unicode scalars without splitting emoji'));
      check(cards()[0].querySelector('.preview-note').textContent.includes('プレビューは先頭2,000文字'), label('preview clipping distinguished from parsing limit'));
      check(preview.includes('<script>unsafe</script>') && !cards()[0].querySelector('script'), label('document markup rendered as text'));
      check(cards()[0].scrollWidth <= cards()[0].clientWidth + 1 && composer().scrollWidth <= composer().clientWidth + 1, label('expanded preview fits composer width'));
      cards()[0].querySelector('[aria-label="添付を削除"]').click(); await tick();
      check(cards().length === 0 && !status().textContent.includes('準備できました'), label('removal clears ready count'));
      const png = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lC8AAAAASUVORK5CYII='), c => c.charCodeAt(0));
      pick([new File([png], 'image.PNG')]);
      await until(() => cards()[0]?.querySelector('img')?.complete);
      check(cards()[0].querySelector('img').naturalWidth === 1 && !pending.length, label('PNG with empty MIME renders an actual image thumbnail'));
      check(status().textContent.includes('添付1件を準備できました') && !action().disabled, label('image readiness uses the same UI feedback'));
      cards()[0].querySelector('[aria-label="添付を削除"]').click(); await tick();

      pick([file('scan.pdf')]); await until(() => pending.length === 1); complete();
      await until(() => status().textContent.includes('読み取れる文字'));
      check(cards().length === 0 && status().textContent.includes('scan.pdf') && !status().textContent.includes('読み込み中'), label('native extraction failure names the file and releases gate'));
      pick([file('unsupported.exe'), file('good.txt')]); await until(() => pending.length === 1);
      check(cards().length === 0 && status().textContent.includes('unsupported.exe') && status().textContent.includes('読み込み中'), label('mixed selection shows both failed filename and pending valid file'));
      complete(); await until(() => cards().length === 1);
      check(status().textContent.includes('対応形式') && status().textContent.includes('添付1件を準備できました'), label('mixed selection retains ready file alongside the failure hint'));
      cards()[0].querySelector('[aria-label="添付を削除"]').click(); await tick();
      pick([file('empty.txt', '')]); await until(() => status().textContent.includes('空です'));
      check(pending.length === 0, label('empty file rejected visibly before native work'));
      pick([file('large.txt', new Uint8Array(10 * 1024 * 1024 + 1))]); await until(() => status().textContent.includes('大きすぎ'));
      check(pending.length === 0, label('10MB limit visible before native work'));

      pick([file('partial.txt')]); await until(() => pending.length === 1); complete(); await until(() => cards().length === 1);
      check(cards()[0].textContent.includes('内容の一部を使用') && !status().textContent.includes('大きすぎ'), label('partial-document warning and successful retry'));
      pick([file('two.txt'), file('three.txt'), file('four.txt'), file('five.txt')]);
      for (let i = 0; i < 3; i++) { await until(() => pending.length === 1); complete(); }
      await until(() => status().textContent.includes('最大4件まで'));
      check(cards().length === 4 && status().textContent.includes('添付4件を準備できました'), label('capacity limit retains successful files and ready count'));
      while (cards().length > 1) { cards().at(-1).querySelector('[aria-label="添付を削除"]').click(); await tick(); }
      check(!status().textContent.includes('最大4件まで'), label('removal clears stale capacity error'));
      action().click(); await until(() => calls.some(c => c.command === 'agent_send'));
      const sent = calls.find(c => c.command === 'agent_send').args;
      check(sent.content === '' && sent.images.length === 0 && sent.documents[0].text === body && sent.documents[0].truncated, label('doc-only send carries complete extracted body and truncation'));
      await until(() => target.querySelector('.document-preview') && cards().length === 0 && action().title !== '停止');
      check(!status().textContent.includes('準備できました'), label('submitted attachments clear composer feedback'));
      await unmount(app); app = null;
      calls.length = 0; await show(surface);
      await until(() => target.querySelector('.document-preview'));
      check(target.querySelector('.document-preview').textContent.includes('partial.txt') && target.querySelector('.document-preview').textContent.includes('内容の一部を使用'), label('reloaded history restores filename and partial warning'));
      for (const theme of ['light', 'dark']) {
        document.documentElement.dataset.theme = theme; await tick();
        const summary = target.querySelector('.document-preview summary');
        check(getComputedStyle(summary.querySelector('span')).color === getComputedStyle(summary).color, label(theme + ' history warning inherits readable bubble text'));
      }
      document.documentElement.dataset.theme = 'light';
      pick([file('資料.txt')]); await until(() => pending.length === 1); complete(); await until(() => cards().length === 1);
      check(!calls.some(c => /stt.*start|start.*stt/.test(c.command)), label('no microphone started'));
    }
    const activeStreams = listeners.filter(l => l.active && l.name.startsWith('agent_stream:'));
    check(activeStreams.length === 1 && activeStreams[0].name === 'agent_stream:fixture', 'unmounted views release their streams; exactly one current stream remains');
  } catch (e) { error = String(e); }
  const result = { passed: checks.length, checks, error };
  document.getElementById('result').textContent = JSON.stringify(result, null, 2);
  await fetch('/result', { method: 'POST', body: JSON.stringify(result) });
};
`;
const bundle = await build({ stdin: { contents: source, loader: 'js', resolveDir: process.cwd() }, bundle: true, write: false, platform: 'browser', format: 'esm', conditions: ['browser'], loader: { '.png': 'dataurl' }, outfile: 'probe.js', plugins: [{ name: 'attachments-dom-probe', setup(builder) {
  builder.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|attachment-probe-transport)$/ }, () => ({ path: 'transport', namespace: 'attachments-probe' }));
  builder.onResolve({ filter: /^\.\.\/api$/ }, () => ({ path: 'api', namespace: 'attachments-probe' }));
  builder.onResolve({ filter: /^\.\.\/stores$/ }, () => ({ path: 'stores', namespace: 'attachments-probe' }));
  builder.onLoad({ filter: /.*/, namespace: 'attachments-probe' }, ({ path }) => ({ contents: path === 'transport' ? transport : path === 'stores' ? `import { writable } from 'svelte/store'; export const agentConversations = writable([]), agentActiveConvId = writable(null), agentReady = writable(true);` : `export * from ${JSON.stringify(resolve('src/lib/agentApi.ts'))}; export const isDemoActive = () => false, isAiReady = async () => true, getAiConfig = async () => ({});`, loader: 'js', resolveDir: resolve('.') }));
  builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({ contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client', css: 'injected' }).js.code, loader: 'js', resolveDir: dirname(path) }));
} }] });
const server = createServer(async (request, response) => {
  if (request.url === '/probe.js' || request.url === '/probe.css') {
    const file = bundle.outputFiles.find(file => file.path.endsWith(request.url));
    response.setHeader('Content-Type', request.url.endsWith('.css') ? 'text/css' : 'text/javascript'); response.end(file?.text ?? '');
  } else if (request.url === '/result' && request.method === 'POST') {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    console.log(Buffer.concat(chunks).toString()); response.end('ok');
  } else {
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><meta charset="utf-8"><title>Agent attachment UI regression</title><link rel="stylesheet" href="/probe.css"><style>:root{--bg-primary:#fff;--bg-secondary:#f6f6f8;--bg-card:#fff;--text-primary:#222;--text-secondary:#666;--text-tertiary:#888;--border:#ddd;--accent:#6752a3}body{margin:0;background:#f3f3f6;font:14px system-ui;color:#222}#toolbar{padding:12px}#probe{height:650px;margin:0 12px;max-width:calc(100vw - 24px);border:1px solid #ddd;background:#fff;position:relative;overflow:hidden}#result{padding:12px;white-space:pre-wrap}</style><div id="toolbar"><button id="run">Run attachment UI checks</button> Synthetic files · native/cloud/microphone disabled</div><div id="probe"></div><pre id="result"></pre><script type="module" src="/probe.js"></script>');
  }
});
server.listen(0, '127.0.0.1', () => console.log('http://127.0.0.1:' + server.address().port + '/?owner=agent-popup&target=agent-popup&kind=agent'));
