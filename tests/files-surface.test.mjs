import test from 'node:test';
import assert from 'node:assert/strict';
import { loadFilesSurface } from './load-files-surface.mjs';
import { loadTypeScript } from './load-typescript.mjs';
const { filePreviewRequest } = await loadTypeScript('src/lib/filePreviewController.ts');
function deferred() { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const record = id => ({ id, filename: id + '.txt', path: '/fixture/' + id + '.txt', course_name: '授業', source: 'scan', size_bytes: 10, downloaded_at: 1, file_exists: true });
const fallback = command => command === 'get_app_theme' ? 'light' : command === 'list_downloads' ? [] : null;

test('closing the files surface stops queued previews and prevents late image publication', async () => {
  const h = await loadFilesSurface(); const reads = [];
  h.configure(command => { if (command !== 'get_download_preview') return fallback(command); const d = deferred(); reads.push(d); return d.promise; });
  h.files.setViewMode('icons'); for (let i = 0; i < 20; i++) h.files.preview('/fixture/' + i); await flush();
  assert.equal(reads.length, 4); h.files.dispose();
  reads.forEach(d => d.resolve({ kind: 'image', data_url: 'data:image/png;base64,fixture' })); await flush();
  assert.equal(h.calls.filter(c => c.command === 'get_download_preview').length, 4); assert.deepEqual(h.files.state.previewMap, {});
});

test('removing selected history uses the submitted IDs and retains a selection changed during IO', async () => {
  const h = await loadFilesSurface(); const remove = deferred(); h.configure(c => c === 'remove_download_records' ? remove.promise : fallback(c));
  h.files.records([record('a'), record('b')]); h.files.select('a', true); const work = h.files.removeSelected();
  h.files.select('a', false); h.files.select('b', true); remove.resolve(); await work;
  assert.deepEqual(h.calls.find(c => c.command === 'remove_download_records').args.ids, ['a']);
  assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['b']); assert.deepEqual(h.files.state.selectedIds, ['b']); h.files.dispose();
});

test('a late initial download list cannot resurrect a record removed after that read began', async () => {
  const h = await loadFilesSurface(); const read = deferred(); h.configure(c => c === 'list_downloads' ? read.promise : fallback(c));
  h.files.records([record('a'), record('b')]); const loading = h.files.load(); await flush();
  await h.files.remove('a'); read.resolve([record('a'), record('b')]); await loading;
  assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['b']); h.files.dispose();
});

test('late registration after closing releases every handle once without starting reads or UI work', async () => {
  const h = await loadFilesSurface(), pending = [];
  h.configure(fallback, (l, release) => { const d = deferred(); pending.push({ d, release }); return d.promise; });
  h.files.mount(); await flush(); assert.equal(pending.length, 4); h.files.dispose();
  const calls = h.calls.length, writes = h.writes.length;
  pending.forEach(({ d, release }) => d.resolve(release)); await flush();
  h.listeners.forEach(l => l.receive({ payload: l.name === 'theme-changed' ? 'dark' : 'late' })); await flush();
  assert.deepEqual(h.listeners.map(l => l.releases), [1, 1, 1, 1]); assert.equal(h.calls.length, calls); assert.equal(h.writes.length, writes);
});

test('subscriptions precede blocked theme/list IO and queued course focus resolves against the actual first list', async () => {
  const h = await loadFilesSurface(), theme = deferred(), read = deferred();
  h.configure(c => {
    if (['get_app_theme', 'list_downloads'].includes(c)) assert.equal(h.listeners.length, 4);
    return c === 'get_app_theme' ? theme.promise : c === 'list_downloads' ? read.promise : null;
  });
  h.files.mount(); await flush();
  assert.deepEqual(h.listeners.find(l => l.name === 'document-tab-control').options, { target: 'files-A' });
  h.listeners.find(l => l.name === 'focus-course').receive({ payload: 'course NAME' });
  read.resolve([{ ...record('a'), course_name: 'Course Name' }]); theme.resolve('light'); await flush();
  assert.equal(h.files.state.loading, false); assert.deepEqual(h.files.state.courseFilters, ['Course Name']); h.files.dispose();
});

test('a pushed theme rejects the older theme read and close prevents late DOM writes', async () => {
  const h = await loadFilesSurface(), theme = deferred(); h.configure(c => c === 'get_app_theme' ? theme.promise : fallback(c));
  h.files.mount(); await flush(); h.listeners.find(l => l.name === 'theme-changed').receive({ payload: 'dark' });
  theme.resolve('light'); await flush(); assert.deepEqual(h.writes.filter(w => w.key === 'data-theme').map(w => w.value), ['dark', 'dark']); h.files.dispose();
});

test('download reads coalesce one hundred requests with at most one follow-up and preserve rows on a current failure', async () => {
  const h = await loadFilesSurface(), first = deferred(); let count = 0;
  h.configure(c => c === 'list_downloads' ? (++count === 1 ? first.promise : [record('new')]) : fallback(c));
  const initial = h.files.load(); await flush(); const requests = Array.from({ length: 100 }, () => h.files.load());
  assert.equal(count, 1); first.resolve([record('old')]); await Promise.all([initial, ...requests]);
  assert.equal(count, 2); assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['new']);
  h.configure(c => c === 'list_downloads' ? Promise.reject(new Error('read failed')) : fallback(c));
  await h.files.load(); assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['new']); assert.match(h.files.state.statusMessage, /read failed/); h.files.dispose();
});

test('history mutation serializes repeated clicks and clears only its captured IDs after IO', async () => {
  const h = await loadFilesSurface(), pending = deferred(); h.configure(c => c === 'remove_download_records' ? pending.promise : fallback(c));
  h.files.records([record('a'), record('b')]); h.files.select('a', true);
  const work = h.files.removeSelected(); h.files.select('b', true);
  await h.files.removeSelected(); await h.files.clear(); await h.files.scan();
  assert.deepEqual(h.calls.map(c => c.command), ['remove_download_records']); pending.resolve(); await work;
  assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['b']); assert.deepEqual(h.files.state.selectedIds, ['b']); h.files.dispose();
});

test('deleting selected files keeps the existing two-click confirmation and preserves later selection', async () => {
  const h = await loadFilesSurface(), remove = deferred();
  h.configure(c => c === 'delete_downloaded_files' ? remove.promise : c === 'list_downloads' ? [record('b')] : fallback(c));
  h.files.records([record('a'), record('b')]); h.files.select('a', true);
  await h.files.deleteSelected(); assert.equal(h.calls.length, 0);
  const work = h.files.deleteSelected(); h.files.select('b', true); remove.resolve({ deleted_count: 1, failed_count: 0 }); await work; await flush();
  assert.deepEqual(h.calls.find(c => c.command === 'delete_downloaded_files').args.paths, ['/fixture/a.txt']);
  assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['b']); assert.deepEqual(h.files.state.selectedIds, ['b']); h.files.dispose();
});

test('closed mutation/share/scan completion starts no reload and presents no late result', async () => {
  for (const action of ['deleteSelected', 'scan', 'share']) {
    const h = await loadFilesSurface(), pending = deferred();
    h.configure(c => ['delete_downloaded_files', 'scan_download_dir', 'share_downloaded_files_native'].includes(c) ? pending.promise : fallback(c));
    h.files.records([record('a')]); h.files.select('a', true);
    if (action === 'deleteSelected') await h.files.deleteSelected();
    const work = h.files[action](); h.files.dispose(); const calls = h.calls.length, message = h.files.state.statusMessage;
    pending.resolve(action === 'scan' ? [record('new')] : { deleted_count: 1 }); await work; await flush();
    assert.equal(h.calls.length, calls); assert.equal(h.files.state.statusMessage, message); assert.deepEqual(h.files.state.allRecords.map(r => r.id), ['a']);
  }
});

test('preview reads deduplicate paths, cap concurrency at four and mutate only their own map entry', async () => {
  const h = await loadFilesSurface(), reads = []; let active = 0, max = 0;
  h.configure(c => { if (c !== 'get_download_preview') return fallback(c); const d = deferred(); active++; max = Math.max(max, active); reads.push(d); return d.promise.finally(() => active--); });
  h.files.setViewMode('icons'); const map = h.files.state.previewMap;
  for (let i = 0; i < 10; i++) { h.files.preview('/fixture/' + i); h.files.preview('/fixture/' + i); }
  for (let batch = 0; batch < 3; batch++) { reads.slice(batch * 4, batch * 4 + 4).forEach(d => d.resolve({ kind: 'text', text: '授業 👩🏽‍💻' })); await flush(); }
  assert.equal(reads.length, 10); assert.equal(max, 4); assert.equal(h.files.state.previewMap, map); assert.equal(Object.keys(map).length, 10);
  h.files.preview('/fixture/1'); await flush(); assert.equal(reads.length, 10); h.files.dispose();
});

test('switching to list discards queued previews and a later icons view can request them again', async () => {
  const h = await loadFilesSurface(), reads = []; h.configure(c => { if (c !== 'get_download_preview') return fallback(c); const d = deferred(); reads.push(d); return d.promise; });
  h.files.setViewMode('icons'); for (let i = 0; i < 10; i++) h.files.preview('/fixture/' + i);
  h.files.setViewMode('list'); reads.forEach(d => d.resolve(null)); await flush(); assert.equal(reads.length, 4);
  h.files.setViewMode('icons'); h.files.preview('/fixture/9'); await flush(); assert.equal(reads.length, 5); reads[4].resolve(null); await flush(); h.files.dispose();
});

test('destroy cancels the hint timer and guards an already queued observer delivery', async () => {
  const h = await loadFilesSurface(); h.configure(fallback); h.files.setViewMode('icons');
  const node = { dataset: {}, isConnected: true }; h.files.lazyPreview(node, '/fixture/a.txt');
  h.files.hint('a'); const timer = h.clock.created[0]; h.files.dispose(); const calls = h.calls.length;
  assert.equal(h.clock.active.size, 0); assert.equal(h.observers[0].disconnected, true);
  timer.callback(); h.observers[0].receive([{ isIntersecting: true, target: node }]); await flush(); assert.equal(h.calls.length, calls);
});

test('closing and reopening duplicate scans rejects old rows and repeated scans have one follow-up', async () => {
  const h = await loadFilesSurface(), first = deferred(); let count = 0;
  h.configure(c => c === 'scan_duplicate_downloads' ? (++count === 1 ? first.promise : [{ content_hash: 'current', items: [], size_bytes: 0 }]) : fallback(c));
  const old = h.files.scanDuplicates(); await flush(); h.files.closeDuplicateModal();
  const requests = Array.from({ length: 100 }, () => h.files.scanDuplicates());
  first.resolve([{ content_hash: 'old', items: [], size_bytes: 0 }]); await Promise.all([old, ...requests]);
  assert.equal(count, 2); assert.equal(h.files.state.duplicateGroups[0].content_hash, 'current'); h.files.dispose();
});

test('a duplicate cleanup finishing after page disposal does not scan or reload files', async () => {
  const h = await loadFilesSurface(), cleanup = deferred();
  const groups = [{ content_hash: 'fixture', size_bytes: 10, items: [{ ...record('keep'), is_recommended: true }, { ...record('remove'), is_recommended: false }] }];
  h.configure(c => c === 'scan_duplicate_downloads' ? groups : c === 'cleanup_duplicate_downloads' ? cleanup.promise : fallback(c));
  await h.files.scanDuplicates(); await h.files.cleanupDuplicates(); const work = h.files.cleanupDuplicates();
  h.files.dispose(); const calls = h.calls.length; cleanup.resolve({ deleted_count: 1 }); await work; await flush();
  assert.equal(h.calls.length, calls); assert.equal(h.calls.filter(c => c.command === 'cleanup_duplicate_downloads').length, 1);
});

test('a failed registration disables its group and releases other late registrations', async () => {
  const h = await loadFilesSurface(), late = deferred(); let releaseLate;
  h.configure(fallback, (l, release) => {
    if (l.name === 'theme-changed') { releaseLate = release; return late.promise; }
    if (l.name === 'document-tab-control') return Promise.reject(new Error('subscription failed'));
    return release;
  });
  h.files.mount(); await flush(); assert.match(h.files.state.statusMessage, /subscription failed/);
  h.listeners.find(l => l.name === 'theme-changed').receive({ payload: 'dark' }); late.resolve(releaseLate); await flush();
  assert.equal(h.writes.filter(w => w.key === 'data-theme').length, 0);
  assert.deepEqual(h.listeners.map(l => l.releases), [1, 1, 1, 0]);
  assert.equal(h.calls.filter(c => c.command === 'list_downloads').length, 0); h.files.dispose();
});

test('successful deletion with a history error reports both outcomes for single, batch and duplicate actions', async () => {
  for (const action of ['single', 'batch', 'duplicates']) {
    const h = await loadFilesSurface();
    const result = { deleted_count: 1, failed_count: 0, errors: ['履歴の更新に失敗しました'] };
    const groups = [{ content_hash: 'fixture', size_bytes: 10, items: [{ ...record('keep'), is_recommended: true }, { ...record('remove'), is_recommended: false }] }];
    h.configure(c => ['delete_downloaded_files', 'cleanup_duplicate_downloads'].includes(c) ? result : c === 'scan_duplicate_downloads' ? groups : fallback(c));
    h.files.records([record('remove')]);
    if (action === 'single') await h.files.deleteEntry(record('remove'));
    else if (action === 'batch') {
      h.files.select('remove', true); await h.files.deleteSelected(); await h.files.deleteSelected();
    } else {
      await h.files.scanDuplicates(); await h.files.cleanupDuplicates(); await h.files.cleanupDuplicates();
      assert.match(h.files.state.dupFootOverride, /履歴の更新に失敗しました/);
    }
    await flush();
    assert.match(h.files.state.statusMessage, /削除しました/);
    assert.match(h.files.state.statusMessage, /履歴の更新に失敗しました/);
    assert.doesNotMatch(h.files.state.statusMessage, /1件失敗/);
    h.files.dispose();
  }
});


test('preview targets remain observed and offscreen images release their display references', async () => {
  const h = await loadFilesSurface();
  h.configure(c => c === 'get_download_preview' ? { kind: 'image', mime: 'image/png', data_url: 'data:image/png;base64,complete' } : fallback(c));
  h.files.setViewMode('icons');
  const node = { dataset: {}, isConnected: true };
  h.files.lazyPreview(node, '/fixture/image.png');
  const observer = h.observers[0];
  observer.receive([{ isIntersecting: true, target: node }]); await flush();
  assert.equal(Object.keys(h.files.state.previewMap).length, 1);
  assert.equal(observer.nodes.has(node), true);
  observer.receive([{ isIntersecting: false, target: node }]); await flush();
  assert.deepEqual(h.files.state.previewMap, {});
  observer.receive([{ isIntersecting: true, target: node }]); await flush();
  assert.equal(h.calls.filter(c => c.command === 'get_download_preview').length, 1);
  h.files.dispose();
});

test('large completed previews have a bounded reuse cache while visible previews keep their full content', async () => {
  const h = await loadFilesSurface();
  const complete = { kind: 'image', mime: 'image/png', data_url: 'data:image/png;base64,' + 'A'.repeat(4 * 1024 * 1024) };
  h.configure(c => c === 'get_download_preview' ? complete : fallback(c));
  h.files.setViewMode('icons');
  for (let i = 0; i < 20; i++) h.files.preview('/fixture/' + i + '.png');
  await flush();
  assert.equal(Object.keys(h.files.state.previewMap).length, 20);
  assert.equal(Object.values(h.files.state.previewMap).every(p => p.data_url === complete.data_url), true);
  assert.ok(h.files.state.previewStats.cacheBytes <= 32 * 1024 * 1024);
  h.files.setViewMode('list');
  assert.deepEqual(h.files.state.previewMap, {});
  h.files.dispose();
});

test('two observed nodes for one preview share IO and destroying one retains the other display', async () => {
  const h = await loadFilesSurface();
  h.configure(c => c === 'get_download_preview' ? { kind: 'text', mime: 'text/plain', text: 'full shared text' } : fallback(c));
  h.files.setViewMode('icons');
  const a = { dataset: {}, isConnected: true }, b = { dataset: {}, isConnected: true };
  const first = h.files.lazyPreview(a, '/fixture/shared.txt'), second = h.files.lazyPreview(b, '/fixture/shared.txt');
  h.observers[0].receive([{ isIntersecting: true, target: a }, { isIntersecting: true, target: b }]); await flush();
  first.destroy(); assert.equal(Object.keys(h.files.state.previewMap).length, 1);
  assert.equal(h.calls.filter(c => c.command === 'get_download_preview').length, 1);
  second.destroy(); assert.deepEqual(h.files.state.previewMap, {}); h.files.dispose();
});

test('leaving the viewport cancels queued previews and late active results are cached without reattaching images', async () => {
  const h = await loadFilesSurface(), pending = [];
  h.configure(c => { if (c !== 'get_download_preview') return fallback(c); const d = deferred(); pending.push(d); return d.promise; });
  h.files.setViewMode('icons');
  const nodes = Array.from({ length: 10 }, (_, i) => ({ dataset: {}, isConnected: true, path: '/fixture/' + i }));
  nodes.forEach(n => h.files.lazyPreview(n, n.path));
  h.observers[0].receive(nodes.map(target => ({ isIntersecting: true, target }))); assert.equal(pending.length, 4);
  h.observers[0].receive(nodes.map(target => ({ isIntersecting: false, target })));
  assert.equal(h.files.state.previewStats.queued, 0);
  pending.forEach(d => d.resolve({ kind: 'image', mime: 'image/png', data_url: 'data:image/png;base64,whole' })); await flush();
  assert.equal(pending.length, 4); assert.deepEqual(h.files.state.previewMap, {});
  h.observers[0].receive([{ isIntersecting: true, target: nodes[0] }]); await flush();
  assert.equal(pending.length, 4); assert.equal(Object.keys(h.files.state.previewMap).length, 1); h.files.dispose();
});

test('reusing a preview node drains old observations and publishes only its current file version', async () => {
  const h = await loadFilesSurface(), reads = [];
  h.configure(c => { if (c !== 'get_download_preview') return fallback(c); const d = deferred(); reads.push(d); return d.promise; });
  const first = record('image'), second = { ...first, size_bytes: 20, downloaded_at: 2 };
  h.files.records([first]); h.files.setViewMode('icons');
  const node = { dataset: {}, isConnected: true };
  const action = h.files.lazyPreview(node, filePreviewRequest(first)), observer = h.observers[0];
  observer.receive([{ isIntersecting: true, target: node }]);
  const stale = { isIntersecting: true, target: node, time: performance.now() };
  observer.pending.push(stale); h.files.records([second]); action.update(filePreviewRequest(second));
  assert.equal(observer.pending.length, 0);
  observer.receive([stale]); assert.equal(reads.length, 1);
  observer.receive([{ isIntersecting: true, target: node }]); assert.equal(reads.length, 2);
  reads[1].resolve({ kind: 'text', mime: 'text/plain', text: 'current complete text' }); await flush();
  reads[0].resolve({ kind: 'text', mime: 'text/plain', text: 'obsolete text' }); await flush();
  assert.equal(Object.values(h.files.state.previewMap)[0].text, 'current complete text');
  assert.equal(h.files.state.previewStats.cacheEntries, 1); action.destroy(); h.files.dispose();
});

test('record removal stops late preview caching and same-tick A to B to A can rearm the same node', async () => {
  const h = await loadFilesSurface(), reads = [];
  h.configure(c => { if (c !== 'get_download_preview') return fallback(c); const d = deferred(); reads.push(d); return d.promise; });
  const a = record('a'), b = record('b'); h.files.records([a]); h.files.setViewMode('icons');
  const node = { dataset: {}, isConnected: true }, action = h.files.lazyPreview(node, filePreviewRequest(a));
  h.observers[0].receive([{ isIntersecting: true, target: node }]);
  h.files.records([b]); h.files.records([a]); action.update(filePreviewRequest(a));
  h.observers[0].receive([{ isIntersecting: true, target: node }]); assert.equal(reads.length, 2);
  reads[1].resolve({ kind: 'text', mime: 'text/plain', text: 'fresh A' }); await flush();
  reads[0].resolve({ kind: 'text', mime: 'text/plain', text: 'old A' }); await flush();
  assert.equal(Object.values(h.files.state.previewMap)[0].text, 'fresh A');
  h.files.records([]); assert.deepEqual(h.files.state.previewMap, {}); assert.equal(h.files.state.previewStats.cacheEntries, 0);
  action.destroy(); h.files.dispose();
});

test('list to icons in one tick rearms existing nodes and unchanged snapshots reuse their cache', async () => {
  const h = await loadFilesSurface();
  h.configure(c => c === 'get_download_preview' ? { kind: 'text', mime: 'text/plain', text: 'complete' } : fallback(c));
  const r = record('a'); h.files.records([r]); h.files.setViewMode('icons');
  const node = { dataset: {}, isConnected: true }, action = h.files.lazyPreview(node, filePreviewRequest(r));
  h.observers[0].receive([{ isIntersecting: true, target: node }]); await flush();
  h.files.setViewMode('list'); assert.deepEqual(h.files.state.previewMap, {});
  h.files.setViewMode('icons'); h.files.records([{ ...r }]); action.update(filePreviewRequest(r));
  h.observers[0].receive([{ isIntersecting: true, target: node }]); await flush();
  assert.equal(Object.values(h.files.state.previewMap)[0].text, 'complete');
  assert.equal(h.calls.filter(c => c.command === 'get_download_preview').length, 1);
  action.destroy(); h.files.dispose();
});
