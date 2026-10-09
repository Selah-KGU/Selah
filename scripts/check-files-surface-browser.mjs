// Compile the actual FilesSurface with only native IPC replaced. All records,
// deletion, sharing and scan results are in-memory fixtures; no real app or
// downloads are touched. Open the printed URL; Ctrl-C stops the server.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { createServer } from 'node:http';

const transport = `
export const calls = [], listeners = [];
const record = (id, filename, course, exists = true) => ({ id, filename, path: '/fixture/' + filename, course_name: course, source: 'scan', size_bytes: 100, downloaded_at: Date.now(), file_exists: exists });
export const fixtures = [record('a', 'alpha.png', 'Course A'), record('b', 'beta.txt', 'Course B'), record('missing', 'missing.pdf', 'Course B', false)];
let records = fixtures.slice(), removeDone = null, duplicateDone = null;
let textPreview = '授業 👩🏽‍💻\\n完全な preview';
export function overwriteTextPreview() { textPreview = '更新後の全文 👩🏽‍💻'; records = records.map(r => r.id === 'b' ? { ...r, size_bytes: r.size_bytes + 1, downloaded_at: r.downloaded_at + 1 } : r); }
export const removing = () => !!removeDone, scanningDuplicates = () => !!duplicateDone;
export function finishRemove() { removeDone(); removeDone = null; }
export function finishDuplicates() { duplicateDone([]); duplicateDone = null; }
export async function invoke(command, args) {
  calls.push({ command, args });
  if (command === 'get_app_theme') return 'light';
  if (command === 'list_downloads') return records.slice();
  if (command === 'get_download_preview') return args.path.endsWith('.txt') ? { kind: 'text', mime: 'text/plain', text: textPreview } : { kind: 'image', mime: 'image/png', data_url: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lC8AAAAASUVORK5CYII=' };
  if (command === 'scan_download_dir') return records.slice();
  if (command === 'remove_download_records') return new Promise(resolve => { removeDone = () => { records = records.filter(r => !args.ids.includes(r.id)); resolve(); }; });
  if (command === 'scan_duplicate_downloads') return new Promise(resolve => { duplicateDone = resolve; });
  if (command === 'delete_downloaded_files') { records = records.filter(r => !args.paths.includes(r.path)); return { deleted_count: args.paths.length, failed_count: 0, errors: ['履歴の更新に失敗しました'] }; }
  if (command === 'open_downloaded_file' || command === 'luna_reveal_file' || command === 'document_tabs_set_controls') return null;
  throw new Error('Unexpected fixture command: ' + command);
}
export async function listen(name, receive, options) { const l = { name, receive, options, active: true, releases: 0 }; listeners.push(l); return () => { l.active = false; l.releases++; }; }
export async function emit() {}
export function push(name, payload) { listeners.filter(l => l.active && l.name === name).forEach(l => l.receive({ payload })); }
export const control = (action, payload) => push('document-tab-control', { owner: 'document-tabs', target: 'fixture-files', action, payload });
`;
const source = `
import { mount, unmount, tick } from 'svelte';
import Files from ${JSON.stringify(resolve('src/lib/FilesSurface.svelte'))};
import { calls, listeners, control, push, finishRemove, removing, finishDuplicates, scanningDuplicates, overwriteTextPreview } from 'files-probe-transport';
const checks = [];
function check(ok, label) { if (!ok) throw new Error(label); checks.push(label); }
async function until(ready) { for (let i = 0; i < 120; i++) { await tick(); if (ready()) return; await new Promise(requestAnimationFrame); } throw new Error('DOM condition timed out'); }
document.getElementById('run').onclick = async () => {
  document.getElementById('run').disabled = true;
  let app, error = null;
  try {
    app = mount(Files, { target: document.getElementById('probe') });
    await until(() => document.querySelectorAll('.file-row').length === 2);
    check(listeners.length === 4 && listeners.find(l => l.name === 'document-tab-control').options.target === 'fixture-files', 'owned targeted toolbar subscription');
    check(document.querySelector('.nav-count').textContent === '3', 'history still contains missing files');
    control('files.toggleView');
    await until(() => document.querySelector('.file-card-preview img')?.complete && document.querySelector('.file-card-preview-text'));
    check(document.querySelector('.file-card-preview img').naturalWidth === 1, 'actual image preview updates its reactive entry');
    check(document.querySelector('.file-card-preview-text').textContent.includes('授業 👩🏽‍💻'), 'text preview retains Unicode');
    check(calls.filter(c => c.command === 'get_download_preview').length === 2, 'visible preview paths fetched once');
    control('files.toggleView'); await tick(); control('files.toggleView');
    await until(() => document.querySelector('.file-card-preview img'));
    check(calls.filter(c => c.command === 'get_download_preview').length === 2, 'view switch reuses completed previews');
    control('files.toggleView'); await until(() => document.querySelectorAll('.file-row').length === 2);
    control('files.search', ' BeTa '); await tick();
    check(document.querySelectorAll('.file-row').length === 1 && document.querySelector('.file-name').textContent === 'beta.txt', 'toolbar search retains case and whitespace normalization');
    control('files.search', ''); await tick(); push('focus-course', 'COURSE b'); await tick();
    check(document.querySelectorAll('.file-row').length === 1 && document.querySelector('.file-name').textContent === 'beta.txt', 'course focus resolves the actual case-insensitive label');
    push('focus-course', ''); await tick(); control('files.toggleMissing'); await tick();
    check(document.querySelectorAll('.file-row').length === 3 && document.querySelectorAll('.file-row.missing').length === 1, 'missing file control preserves filtered list behavior');
    const a = document.querySelector('[data-id="a"] input'), b = document.querySelector('[data-id="b"] input');
    a.click(); await tick();
    [...document.querySelectorAll('.selection-actions button')].find(el => el.textContent === '履歴から削除').click(); await until(removing);
    b.click(); await tick();
    check([...document.querySelectorAll('.selection-actions button')].find(el => el.textContent === '履歴から削除').disabled && [...document.querySelectorAll('.action-btn.remove')].every(el => el.disabled), 'history and row removal are disabled while their request is pending');
    finishRemove(); await until(() => !document.querySelector('[data-id="a"]'));
    check(document.querySelector('[data-id="b"] input').checked && document.querySelectorAll('.file-row').length === 2, 'submitted removal keeps later selection and untouched row');
    check(calls.find(c => c.command === 'remove_download_records').args.ids.join('|') === 'a', 'removal transport contains only submitted IDs');
    control('files.duplicates'); await until(scanningDuplicates);
    check(document.querySelector('[role="dialog"][aria-label="重複ファイル整理"]'), 'duplicate scan modal opens');
    document.querySelector('.duplicate-modal button[aria-label="閉じる"]').click(); await tick(); finishDuplicates(); await tick();
    check(!document.querySelector('.duplicate-modal'), 'late duplicate result cannot reopen a dismissed modal');
    control('files.toggleView'); await until(() => document.querySelector('.file-card-preview-text'));
    const reusedPreviewNode = document.querySelector('[data-id="b"] .file-card-preview');
    overwriteTextPreview(); control('files.scan');
    await until(() => document.querySelector('.file-card-preview-text')?.textContent === '更新後の全文 👩🏽‍💻');
    check(document.querySelector('[data-id="b"] .file-card-preview') === reusedPreviewNode, 'same DOM preview target loads its changed record version');
    check(calls.filter(c => c.command === 'get_download_preview' && c.args.path.endsWith('.txt')).length === 2, 'changed version invalidates exactly one cached text preview');
    control('files.toggleView'); await until(() => document.querySelector('[data-id="b"] input'));
    check(document.querySelector('[data-id="b"] input').checked, 'preview version refresh preserves file selection');
    const deleteSelected = () => [...document.querySelectorAll('.selection-actions button')].find(el => ['ファイルを削除', 'もう一度押して削除'].includes(el.textContent.trim()));
    deleteSelected().click(); await tick();
    check(!calls.some(c => c.command === 'delete_downloaded_files'), 'physical deletion still needs its second confirmation');
    deleteSelected().click();
    await until(() => !document.querySelector('[data-id="b"]'));
    check(document.querySelector('.status-bar').textContent.includes('1件のファイルを削除しました') && document.querySelector('.status-bar').textContent.includes('履歴の更新に失敗しました'), 'deletion success and metadata failure both appear in actual DOM');
    push('theme-changed', 'dark'); await tick(); check(document.documentElement.dataset.theme === 'dark', 'theme push updates actual DOM');
    await unmount(app); app = null;
    check(!document.querySelector('.files-root'), 'files DOM removed at unmount');
    check(listeners.every(l => l.releases === 1), 'all native subscriptions released exactly once');
    const count = calls.length;
    listeners.find(l => l.name === 'document-tab-control').receive({ payload: { owner: 'document-tabs', target: 'fixture-files', action: 'files.scan' } }); await tick();
    check(calls.length === count, 'queued control cannot perform native work after unmount');
  } catch (e) { error = String(e); }
  finally { if (app) await unmount(app); }
  const result = { passed: checks.length, checks, error };
  document.getElementById('result').textContent = JSON.stringify(result, null, 2);
  await fetch('/result', { method: 'POST', body: JSON.stringify(result) });
};
`;
const bundle = await build({ stdin: { contents: source, loader: 'js', resolveDir: process.cwd() }, bundle: true, write: false, platform: 'browser', format: 'esm', conditions: ['browser'], plugins: [{ name: 'files-dom-probe', setup(builder) {
  builder.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|files-probe-transport)$/ }, () => ({ path: 'transport', namespace: 'files-probe' }));
  builder.onLoad({ filter: /.*/, namespace: 'files-probe' }, () => ({ contents: transport, loader: 'js' }));
  builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({ contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client', css: 'injected' }).js.code, loader: 'js', resolveDir: dirname(path) }));
} }] });
const server = createServer(async (request, response) => {
  if (request.url === '/probe.js') { response.setHeader('Content-Type', 'text/javascript'); response.end(bundle.outputFiles[0].text); }
  else if (request.url === '/result' && request.method === 'POST') { const chunks = []; for await (const chunk of request) chunks.push(chunk); console.log(Buffer.concat(chunks).toString()); response.end('ok'); }
  else { response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><meta charset="utf-8"><title>Files surface DOM verification</title><style>body{margin:0}#run{position:fixed;top:0;right:0;z-index:1000}</style><button id="run">Run files checks</button><div id="probe"></div><pre id="result"></pre><script type="module" src="/probe.js"></script>'); }
});
server.listen(0, '127.0.0.1', () => console.log('http://127.0.0.1:' + server.address().port + '/?tabLabel=fixture-files&ownerLabel=document-tabs'));
