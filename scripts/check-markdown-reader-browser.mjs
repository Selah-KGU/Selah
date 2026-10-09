// Actual reader, Marked, DOMPurify, Svelte DOM and whiteboard renderer. Only the
// native transport is replaced. Open the printed URL and run the fixture; no
// installed app, file write, share sheet, model or microphone is used.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { createServer } from 'node:http';

const backend = `
export const calls = [], listeners = []; let pendingWrite = null;
export let initialPayload;
export const configure = payload => { initialPayload = payload; };
export async function invoke(command, args) {
  calls.push({ command, args });
  if (command === 'get_app_theme') return 'light';
  if (command === 'get_pending_markdown_payload') { const p = initialPayload; initialPayload = null; return p; }
  if (command === 'write_markdown_file') return new Promise(resolve => { pendingWrite = resolve; });
  return null;
}
export const finishWrite = () => { const done = pendingWrite; pendingWrite = null; done(); };
export const writing = () => !!pendingWrite;
export async function listen(name, receive, options) {
  const entry = { name, receive, options, active: true, releases: 0 }; listeners.push(entry);
  return () => { entry.active = false; entry.releases++; };
}
export async function emitTo() {}
export async function emit() {}
export function push(name, payload, target = 'fixture-reader') {
  for (const l of listeners) if (l.active && l.name === name && (!l.options?.target || l.options.target === target)) l.receive({ payload });
}
export const control = action => push('document-tab-control', { owner: 'document-tabs', target: 'fixture-reader', action });
`;
const source = `
import { mount, unmount, tick } from 'svelte';
import Reader from ${JSON.stringify(resolve('src/lib/MarkdownReaderSurface.svelte'))};
import { calls, listeners, configure, push, control, finishWrite, writing } from 'reader-probe-transport';
import { counts } from 'reader-probe-marked';
const board = { title: '板書', nodes: [{ id: 'a', label: '概念', role: 'main' }, { id: 'b', label: '根拠', role: 'branch' }], edges: [{ from: 'a', to: 'b', label: '支える' }] };
let markdown = '# 授業\\n\\n## 同じ見出し\\n\\n全文 👩🏽‍💻\\n\\n## 同じ見出し\\n\\n[公式サイト](https://example.com/) [章へ](#同じ見出し-2)\\n\\n<script>window.__unsafe=1</script>\\n<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lC8AAAAASUVORK5CYII=" onerror="window.__unsafe=1" alt="fixture">\\n\\n' + '\u0060\u0060\u0060live-whiteboard\\n' + JSON.stringify(board) + '\\n\u0060\u0060\u0060\\n\\n# 終わり';
markdown += '\\n\\n長い文書のスクロール確認 👩🏽‍💻'.repeat(100);
const payload = (revision, text = markdown, error = null) => ({ path: '/fixture/note.md', filename: 'note.md', markdown: text, error, deliveryRevision: String(revision) });
const checks = [];
function check(ok, name) { if (!ok) throw new Error(name); checks.push(name); }
async function until(ready) { for (let i = 0; i < 120; i++) { await tick(); if (ready()) return; await new Promise(requestAnimationFrame); } throw new Error('DOM condition timed out'); }
function edit(value) { const el = document.querySelector('textarea'); el.value = value; el.dispatchEvent(new Event('input', { bubbles: true })); }
const output = document.getElementById('result');
document.getElementById('run').onclick = async () => {
  document.getElementById('run').disabled = true;
  let app, error = null, diagnostics = null;
  try {
    configure(payload(1));
    app = mount(Reader, { target: document.getElementById('probe') });
    await until(() => document.querySelector('.whiteboard') && document.querySelector('.doc h1'));
    check(document.querySelector('.doc').textContent.includes('全文 👩🏽‍💻'), 'full Unicode markdown rendered');
    check(document.title === 'note.md', 'document title retained');
    check(!document.querySelector('.doc script,.doc [onerror]') && !window.__unsafe, 'DOMPurify strips script and inline handler');
    const ids = [...document.querySelectorAll('.doc h2')].map(h => h.id);
    check(ids.join('|') === '同じ見出し|同じ見出し-2', 'duplicate headings retain unique ToC ids');
    check(listeners.filter(l => ['markdown-content', 'document-tab-control'].includes(l.name)).every(l => l.options.target === 'fixture-reader'), 'reader transport is targeted');
    control('reader.toggleToc'); await tick();
    check(document.querySelectorAll('.toc-item').length === 4, 'ToC lists document headings');
    [...document.querySelectorAll('.toc-item')].at(-1).click();
    await until(() => document.querySelector('.toc-item.active')?.textContent === '終わり' && Math.abs([...document.querySelectorAll('.doc h1')].at(-1).getBoundingClientRect().top - document.querySelector('.scroll').getBoundingClientRect().top - 12) < 2);
    check(document.querySelector('.toc-item.active').textContent === '終わり', 'ToC reaches the heading after the whiteboard within 2px');
    document.querySelector('.doc a[href="https://example.com/"]').click(); await tick();
    check(calls.filter(c => c.command === 'open_in_system_browser').length === 1, 'external link uses one captured native call');
    document.querySelector('.doc img').click(); await tick();
    check(document.querySelector('[role="dialog"][aria-label="画像プレビュー"]'), 'image lightbox opens');
    document.querySelector('.lightbox button[aria-label="閉じる"]').click(); await tick();
    check(!document.querySelector('.lightbox'), 'image lightbox closes');
    const node = document.querySelector('.whiteboard'), heading = document.querySelector('.doc h1'), parses = counts.parses;
    push('markdown-content', payload(1)); await tick();
    check(counts.parses === parses && document.querySelector('.whiteboard') === node && document.querySelector('.doc h1') === heading, 'duplicate delivery preserves parsed DOM and board');
    control('reader.edit'); await tick(); check(document.querySelector('textarea').value === markdown, 'editor retains full source');
    edit('# submitted'); await tick(); control('reader.save'); await until(writing);
    edit('# later input'); await tick(); finishWrite(); await until(() => !calls.at(-1)?.args?.controls?.find(c => c.id === 'save')?.disabled);
    check(document.querySelector('textarea')?.value === '# later input', 'typing during save stays in editor');
    check(calls.find(c => c.command === 'write_markdown_file').args.contents === '# submitted', 'write uses submitted snapshot');
    edit('# submitted'); await tick(); control('reader.cancel'); await until(() => document.querySelector('.doc h1')?.textContent === 'submitted');
    check(!document.querySelector('textarea'), 'saved snapshot appears after leaving editor');
    push('markdown-content', payload(1)); await tick(); check(document.querySelector('.doc h1').textContent === 'submitted', 'old retry cannot undo saved content');
    push('markdown-content', payload(2)); await until(() => document.querySelector('.whiteboard'));
    check(document.querySelector('.doc h1').textContent === '授業', 'new revision can reopen the original file');
    push('markdown-content', payload(3, '', 'fixture read failed')); await tick();
    check(document.querySelector('.reader-error')?.textContent === 'fixture read failed', 'read error clears prior content');
    push('markdown-content', payload(4)); await until(() => document.querySelector('.whiteboard'));
    check(!document.querySelector('.reader-error'), 'subsequent delivery recovers display');
    await unmount(app); app = null;
    check(!document.querySelector('.reader'), 'reader DOM removed on unmount');
    check(listeners.every(l => l.releases === 1), 'all native subscriptions released once');
    const before = calls.length;
    listeners.find(l => l.name === 'markdown-content').receive({ payload: payload(5) }); await tick();
    check(calls.length === before && !document.querySelector('.reader'), 'queued callback cannot resurrect a disposed reader');
  } catch (e) { error = String(e); diagnostics = {
    scroll: [...document.querySelectorAll('.scroll')].map(el => ({ height: el.clientHeight, full: el.scrollHeight, top: el.scrollTop })),
    headings: [...document.querySelectorAll('.doc h1,.doc h2')].map(el => ({ id: el.id, offset: el.offsetTop })),
    active: [...document.querySelectorAll('.toc-item.active')].map(el => el.textContent),
  }; }
  finally { if (app) await unmount(app); }
  const result = { passed: checks.length, checks, error, diagnostics };
  output.textContent = JSON.stringify(result, null, 2);
  await fetch('/result', { method: 'POST', body: JSON.stringify(result) });
};
`;
const bundle = await build({ stdin: { contents: source, resolveDir: process.cwd(), loader: 'js' }, bundle: true, write: false, platform: 'browser', format: 'esm', conditions: ['browser'], plugins: [{ name: 'reader-dom-probe', setup(builder) {
  builder.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|reader-probe-transport)$/ }, () => ({ path: 'transport', namespace: 'reader-probe' }));
  builder.onResolve({ filter: /^(marked|reader-probe-marked)$/ }, () => ({ path: 'marked', namespace: 'reader-probe' }));
  builder.onLoad({ filter: /.*/, namespace: 'reader-probe' }, ({ path }) => ({ contents: path === 'transport' ? backend : `import { marked as real } from ${JSON.stringify(resolve('node_modules/marked/lib/marked.esm.js'))}; export const counts = { parses: 0 }; export const marked = { ...real, parse(...args) { counts.parses++; return real.parse(...args); } };`, loader: 'js', resolveDir: process.cwd() }));
  builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({ contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client', css: 'injected' }).js.code, loader: 'js', resolveDir: dirname(path) }));
} }] });
const layout = await readFile('static/whiteboard-layout.js');
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  if (pathname === '/probe.js' || pathname === '/whiteboard-layout.js') { response.setHeader('Content-Type', 'text/javascript'); response.end(pathname === '/probe.js' ? bundle.outputFiles[0].text : layout); }
  else if (pathname === '/result' && request.method === 'POST') { const chunks = []; for await (const chunk of request) chunks.push(chunk); console.log(Buffer.concat(chunks).toString()); response.end('ok'); }
  else if (pathname === '/') { response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><meta charset="utf-8"><title>Markdown reader DOM verification</title><style>body{margin:0}#run{position:fixed;top:0;right:0;z-index:100}</style><button id="run">Run reader checks</button><div id="probe"></div><pre id="result"></pre><script src="/whiteboard-layout.js"></script><script type="module" src="/probe.js"></script>'); }
  else { response.statusCode = 404; response.end(); }
});
server.listen(0, '127.0.0.1', () => console.log('http://127.0.0.1:' + server.address().port + '/?tabLabel=fixture-reader&ownerLabel=document-tabs'));
