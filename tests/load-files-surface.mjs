import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build } from 'esbuild';
let instance = 0;
export async function loadFilesSurface() {
  const path = resolve('src/lib/FilesSurface.svelte');
  let script = (await readFile(path, 'utf8')).match(/<script lang="ts">([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error('Files script missing');
  const selected = 'let selectedRecords = $derived(allRecords.filter((r) => selectedIds.has(r.id)));';
  const existing = 'let selectedExisting = $derived(selectedRecords.filter((r) => r.file_exists !== false && r.path));';
  if (!script.includes(selected) || !script.includes(existing)) throw new Error('Selection boundary changed');
  script = script.replace(selected, 'function getSelectedRecords() { return allRecords.filter((r) => selectedIds.has(r.id)); }').replace(existing, 'function getSelectedExisting() { return getSelectedRecords().filter((r) => r.file_exists !== false && r.path); }').replace(/\bselectedExisting\b/g, 'getSelectedExisting()');
  const pathsStart = script.indexOf('  let dupCleanupPaths = $derived.by<string[]>(() => {');
  const pathsEnd = script.indexOf('  let dupWasteBytes', pathsStart);
  if (pathsStart < 0 || pathsEnd < 0) throw new Error('Duplicate paths boundary changed');
  const paths = script.slice(pathsStart, pathsEnd).replace('let dupCleanupPaths = $derived.by<string[]>(() => {', 'function getDupCleanupPaths(): string[] {').replace(/\}\);\s*$/, '}\n');
  script = (script.slice(0, pathsStart) + paths + script.slice(pathsEnd)).replace(/\bdupCleanupPaths\b/g, 'getDupCleanupPaths()');
  const result = await build({ stdin: { contents: `
    import { document, window, location, localStorage, writes, IntersectionObserver, observers } from 'test-files-env';
    import { clock, setTimeout, clearTimeout, setInterval, clearInterval } from 'test-files-clock';
    const $state = value => value, $derived = Object.assign(value => value, { by: () => undefined }), $effect = () => {};
    ${script}
    import { configure, calls, listeners } from 'test-files-backend';
    import { mount, destroy } from 'svelte';
    export { configure, calls, listeners, clock, writes, observers };
    export const files = { mount, dispose: destroy, load: loadDownloads, remove: removeRecord, removeSelected: deleteSelectedRecords, deleteEntry, deleteSelected: deleteSelectedFiles, clear: clearHistory, scan: scanDir,
      share: shareSelectedFiles, scanDuplicates, cleanupDuplicates, closeDuplicateModal, preview(path) { if (resources.active && viewMode === 'icons') return previewController.retain({ path, key: path }); }, lazyPreview(node, path) { return lazyPreview(node, typeof path === 'string' ? { path, key: path } : path); }, setViewMode, hint: pinDeleteHint,
      select: toggleSelect, records(value) { applyRecords(value); },
      get state() { return { allRecords, loading, selectedIds: [...selectedIds], statusMessage, previewMap, deleteHintId, scanning, sharing, deletingFiles, duplicateGroups, dupModalOpen, dupScanning, dupError, dupFootOverride, previewStats: previewController.stats, courseFilters: [...courseFilters] }; },
    };
  `, loader: 'ts', sourcefile: path, resolveDir: dirname(path) }, bundle: true, write: false, platform: 'node', format: 'esm', plugins: [{ name: 'files-boundaries', setup(plugin) {
    plugin.onResolve({ filter: /^(@tauri-apps\/api\/(core|event)|test-files-backend)$/ }, () => ({ path: 'backend', namespace: 'files-test' }));
    plugin.onResolve({ filter: /^test-files-env$/ }, () => ({ path: 'env', namespace: 'files-test' }));
    plugin.onResolve({ filter: /^test-files-clock$/ }, () => ({ path: 'clock', namespace: 'files-test' }));
    plugin.onResolve({ filter: /^svelte$/ }, () => ({ path: 'svelte', namespace: 'files-test' }));
    plugin.onLoad({ filter: /auxiliarySurfaceTheme\.ts$/ }, async ({ path }) => ({ contents: `import { document, localStorage } from 'test-files-env';\n${await readFile(path, 'utf8')}`, loader: 'ts' }));
    plugin.onLoad({ filter: /resourceScope\.ts$/ }, async ({ path }) => ({ contents: `import { setTimeout, clearTimeout, setInterval, clearInterval } from 'test-files-clock';\n${await readFile(path, 'utf8')}`, loader: 'ts' }));
    plugin.onLoad({ filter: /.*/, namespace: 'files-test' }, ({ path }) => ({ contents: path === 'backend' ? `
      export const calls = [], listeners = []; let run, subscribe;
      export const configure = (invoke, listen) => { run = invoke; subscribe = listen; };
      export async function invoke(command, args) { calls.push({ command, args }); return run(command, args); }
      export async function listen(name, receive, options) { const l = { name, receive, options, releases: 0 }; listeners.push(l); const release = () => l.releases++; return subscribe ? subscribe(l, release) : release; }
      export async function emit() {}
    ` : path === 'env' ? `
      export const writes = [], element = name => ({ setAttribute: (key, value) => writes.push({ name, key, value }), removeAttribute: key => writes.push({ name, key, value: '' }) });
      export const document = { documentElement: element('root'), body: element('body') };
      export const location = { search: '?tabLabel=files-A&ownerLabel=document-tabs', hash: '' }, window = { location, innerWidth: 1000 }, localStorage = { getItem: () => '', setItem() {} };
      export const observers = []; export class IntersectionObserver { constructor(receive) { this.receive = receive; this.disconnected = false; this.nodes = new Set(); this.pending = []; observers.push(this); } observe(n) { this.nodes.add(n); } unobserve(n) { this.nodes.delete(n); } takeRecords() { return this.pending.splice(0); } disconnect() { this.nodes.clear(); this.disconnected = true; } }
    ` : path === 'clock' ? `
      let id = 0; export const clock = { active: new Map(), created: [] };
      export function setTimeout(callback, delay) { const timer = { id: ++id, callback, delay }; clock.active.set(timer.id, timer); clock.created.push(timer); return timer.id; }
      export const clearTimeout = id => clock.active.delete(id), setInterval = setTimeout, clearInterval = clearTimeout;
    ` : `
      let start; const stops = []; export const onMount = fn => { start = fn; }, onDestroy = fn => stops.push(fn), mount = () => start(), destroy = () => stops.forEach(fn => fn());
    `, loader: 'js' }));
  } }] });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#files-${++instance}`);
}
