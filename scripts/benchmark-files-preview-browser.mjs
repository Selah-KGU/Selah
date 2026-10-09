// Full FilesSurface, real scrolling/IntersectionObserver/image decode; native
// IO only is replaced by generated PNG fixtures. Compare --before and current.
// Retained bytes estimate strings, not heap/RSS, decoded images or GPU memory.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { createServer } from 'node:http';

const before = process.argv.includes('--before');
let surface = await readFile('src/lib/FilesSurface.svelte', 'utf8');
if (before) {
  const previous = JSON.parse(await readFile('tests/fixtures/files-preview-before.json', 'utf8'));
  const start = surface.indexOf('  // ── Lazy preview loading (icons view)');
  const end = surface.indexOf('  // ── Event wiring', start);
  surface = surface.slice(0, start) + previous.preview + '  function reconcilePreviewRecords() {}\n\n' + surface.slice(end);
  const viewStart = surface.indexOf('  function setViewMode('), viewEnd = surface.indexOf('  function setSortKey', viewStart);
  surface = surface.slice(0, viewStart) + previous.viewMode + surface.slice(viewEnd);
  const closeStart = surface.indexOf('  onDestroy(() => {'), closeEnd = surface.indexOf('\n  });', closeStart) + '\n  });'.length;
  surface = surface.slice(0, closeStart) + previous.destroy + surface.slice(closeEnd);
  surface = surface.replace('{@const request = filePreviewRequest(r)}\n                        {@const preview = previewMap[request.key]}', '{@const preview = previewMap[r.path]}')
    .replace('use:lazyPreview={request}', 'use:lazyPreview={r.path}');
}
const diagnostics = before ? `
  export function previewProbe() {
    const bytes = [...previewCache.entries()].reduce((n, [key, value]) => n + previewBytes(key, value), 0);
    return { cacheEntries: previewCache.size, cacheBytes: bytes, retainedBytes: bytes, active: previewActive, queued: previewQueue.length,
      consumers: Object.keys(previewMap).length, displayEntries: Object.keys(previewMap).length,
      uncachedDisplays: Object.keys(previewMap).filter(key => !previewCache.has(key)).length };
  }
` : `
  export function previewProbe() { return { ...previewController.stats, displayEntries: Object.keys(previewMap).length,
    uncachedDisplays: Object.keys(previewMap).filter(key => !previewController.hasCached(key)).length }; }
`;
surface = surface.replace('</script>', `\nimport { previewBytes } from './filePreviewController';\n${diagnostics}</script>`);

// Browser-decodable 1×1 PNG with unused trailing bytes: large transport/string retention
// without allocating giant decoded surfaces in this development-only probe.
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lC8AAAAASUVORK5CYII=', 'base64');
const base64 = Buffer.concat([png, Buffer.alloc(512 * 1024)]).toString('base64');
const transport = `
export const calls = [], listeners = [], imageData = ${JSON.stringify(base64)};
const now = Date.now();
export const records = Array.from({ length: 96 }, (_, i) => ({ id: String(i), filename: 'image-' + String(i).padStart(3, '0') + '.png', path: '/fixture/image-' + i + '.png', course_name: '画像',
  source: 'scan', size_bytes: ${png.length + 512 * 1024}, downloaded_at: now - i, file_exists: true }));
export async function invoke(command, args) {
  calls.push({ command, args });
  if (command === 'get_app_theme') return 'light';
  if (command === 'list_downloads') return records.slice();
  if (command === 'get_download_preview') return { kind: 'image', mime: 'image/png', data_url: 'data:image/png;name=' + args.path.match(/(\\d+)/)[1] + ';base64,' + imageData };
  if (command === 'document_tabs_set_controls') return null;
  throw new Error('Unexpected preview probe command: ' + command);
}
export async function listen(name, receive, options) { const l = { name, receive, options, releases: 0 }; listeners.push(l); return () => l.releases++; }
export async function emit() {}
export function control(action) { listeners.filter(l => !l.releases && l.name === 'document-tab-control').forEach(l => l.receive({ payload: { owner: 'document-tabs', target: 'fixture-files', action } })); }
`;
const source = `
import { mount, unmount, tick } from 'svelte';
import Files from ${JSON.stringify(resolve('src/lib/FilesSurface.svelte'))};
import { calls, listeners, control, imageData } from 'preview-probe-native';
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
async function until(ready) { for (let i = 0; i < 240; i++) { if (ready()) return; await frames(); } throw new Error('Preview probe timed out'); }
const reads = () => calls.filter(c => c.command === 'get_download_preview').length;
const samples = [], checks = [];
function check(ok, name) { if (!ok) throw new Error(name); checks.push(name); }
document.getElementById('run').onclick = async () => {
  document.getElementById('run').disabled = true;
  let app, error = null;
  try {
    app = mount(Files, { target: document.getElementById('probe') });
    await until(() => document.querySelectorAll('.file-row').length === 96);
    check(reads() === 0, 'list mode requests no image data');
    control('files.toggleView');
    await until(() => document.querySelector('.file-card-preview img'));
    const list = document.querySelector('.file-list');
    async function settle() {
      let images;
      for (let attempt = 0; attempt < 10; attempt++) {
        await frames();
        await until(() => app.previewProbe().active === 0 && app.previewProbe().queued === 0 && document.querySelector('.file-card-preview img'));
        const startReads = reads(), current = [...document.querySelectorAll('.file-card-preview img')];
        await Promise.all(current.map(img => img.decode()));
        await frames();
        if (reads() === startReads && app.previewProbe().active === 0 && app.previewProbe().queued === 0 && current.length === document.querySelectorAll('.file-card-preview img').length) {
          images = current; break;
        }
      }
      if (!images) throw new Error('Preview state did not settle');
      check(images.every(img => img.naturalWidth === 1 && img.src.endsWith(imageData)), 'full PNG data decodes at scroll sample ' + samples.length);
      samples.push({ scrollTop: list.scrollTop, imageElements: images.length, ...app.previewProbe() });
    }
    await settle();
    const viewport = { width: list.clientWidth, height: list.clientHeight };
    while (list.scrollTop + list.clientHeight < list.scrollHeight - 1) {
      list.scrollTop += Math.max(150, list.clientHeight - 160);
      await settle();
    }
    const bottom = { ...samples.at(-1), requested: reads() };
    check(new Set(calls.filter(c => c.command === 'get_download_preview').map(c => c.args.path)).size === 96, 'every file remains reachable and previewable');
    const oldReads = reads();
    list.scrollTop = Math.max(0, list.scrollTop - 100); await settle();
    list.scrollTop = list.scrollHeight; await settle();
    check(reads() === oldReads, 'recent visible previews are reused on a short scroll back');
    control('files.toggleView'); await frames();
    const listMode = app.previewProbe();
    check(document.querySelectorAll('.file-row').length === 96 && !document.querySelector('.file-card-preview img'), 'list view retains all records and removes image DOM');
    if (!${before}) {
      check(bottom.cacheEntries <= 128 && bottom.cacheBytes <= 32 * 1024 * 1024, 'completed reuse cache stays within both budgets');
      check(bottom.imageElements < 96 && bottom.displayEntries === bottom.imageElements, 'offscreen previews are released from DOM and display state');
      check(listMode.displayEntries === 0 && listMode.consumers === 0, 'list mode releases every display consumer');
    }
    control('files.toggleView'); await until(() => document.querySelector('.file-card-preview img')); await settle();
    check(reads() === oldReads + bottom.uncachedDisplays, 'view switch reuses cached previews and reloads only evicted displays');
    await unmount(app); app = null;
    check(listeners.every(l => l.releases === 1), 'unmount releases every native listener');
    const result = { mode: ${JSON.stringify(before ? 'before' : 'current')}, viewport, passed: checks.length, checks, samples, bottom, listMode, error: null };
    document.getElementById('result').textContent = JSON.stringify(result, null, 2);
    await fetch('/result', { method: 'POST', body: JSON.stringify(result) });
  } catch (e) {
    error = String(e); const result = { mode: ${JSON.stringify(before ? 'before' : 'current')}, passed: checks.length, checks, samples, error };
    document.getElementById('result').textContent = JSON.stringify(result, null, 2);
    await fetch('/result', { method: 'POST', body: JSON.stringify(result) });
  } finally { if (app) await unmount(app); }
};
`;
const bundle = await build({ stdin: { contents: source, loader: 'js', resolveDir: process.cwd() }, bundle: true, write: false, platform: 'browser', format: 'esm', conditions: ['browser'], plugins: [{ name: 'preview-retention-probe', setup(builder) {
  builder.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|preview-probe-native)$/ }, () => ({ path: 'native', namespace: 'preview-probe' }));
  builder.onLoad({ filter: /.*/, namespace: 'preview-probe' }, () => ({ contents: transport, loader: 'js' }));
  builder.onLoad({ filter: /FilesSurface\.svelte$/ }, ({ path }) => ({ contents: compile(surface, { filename: path, generate: 'client', css: 'injected' }).js.code, loader: 'js', resolveDir: dirname(path) }));
} }] });
const server = createServer(async (request, response) => {
  if (request.url === '/probe.js') { response.setHeader('Content-Type', 'text/javascript'); response.end(bundle.outputFiles[0].text); }
  else if (request.url === '/result' && request.method === 'POST') { const chunks = []; for await (const chunk of request) chunks.push(chunk); console.log(Buffer.concat(chunks).toString()); response.end('ok'); }
  else { response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><meta charset="utf-8"><title>File preview retention ' + (before ? 'before' : 'current') + '</title><style>body{margin:0}#run{position:fixed;top:0;right:0;z-index:1000}</style><button id="run">Run preview checks</button><div id="probe"></div><pre id="result"></pre><script type="module" src="/probe.js"></script>'); }
});
server.listen(0, '127.0.0.1', () => console.log('http://127.0.0.1:' + server.address().port + '/?tabLabel=fixture-files&ownerLabel=document-tabs'));
