import test from 'node:test';
import assert from 'node:assert/strict';
import { loadTypeScript } from './load-typescript.mjs';
const { FilePreviewController, filePreviewRequest, previewBytes } = await loadTypeScript('src/lib/filePreviewController.ts');
const request = key => ({ path: '/fixture/' + key, key });
const preview = text => ({ kind: 'text', mime: 'text/plain', text });
function deferred() { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }

test('visible consumers share IO and cached complete data while released jobs stop publishing', async () => {
  const pending = deferred(), calls = [], published = [];
  const controller = new FilePreviewController(p => { calls.push(p); return pending.promise; }, (key, value) => published.push({ key, value }));
  const one = controller.retain(request('same')), two = controller.retain(request('same'));
  assert.equal(calls.length, 1);
  one(); one(); assert.equal(controller.stats.consumers, 1);
  const full = preview('完全な Unicode 👩🏽‍💻\n引用'); pending.resolve(full); await flush();
  assert.equal(published.at(-1).value, full);
  two(); assert.equal(published.at(-1).value, undefined); assert.equal(controller.stats.consumers, 0);
  const back = controller.retain(request('same')); assert.equal(calls.length, 1); assert.equal(published.at(-1).value, full);
  back(); controller.dispose(); assert.equal(controller.stats.retainedBytes, 0);
});

test('queue stays bounded to current consumers and no canceled waiting job starts', async () => {
  const loads = [], values = [];
  const controller = new FilePreviewController(path => { const d = deferred(); loads.push({ path, d }); return d.promise; }, (k, p) => values.push([k, p]), { maxConcurrent: 4 });
  const release = Array.from({ length: 100 }, (_, i) => controller.retain(request(String(i))));
  assert.equal(loads.length, 4); assert.equal(controller.stats.queued, 96);
  release.forEach(fn => fn()); assert.equal(controller.stats.queued, 0); assert.equal(controller.stats.consumers, 0);
  loads.forEach(({ d }) => d.resolve(preview('complete'))); await flush();
  assert.equal(loads.length, 4); assert.equal(values.filter(([, p]) => p !== undefined).length, 0);
  assert.equal(controller.stats.active, 0);
  const again = controller.retain(request('99')); assert.equal(loads.length, 5);
  loads[4].d.resolve(null); await flush(); again(); controller.dispose();
});

test('oversized preview displays fully and is shared until its last consumer leaves without entering the cache', async () => {
  const full = preview('全文 👩🏽‍💻'.repeat(1000)), published = []; let calls = 0;
  const controller = new FilePreviewController(async () => { calls++; return full; }, (_, p) => published.push(p), { maxBytes: 100 });
  const first = controller.retain(request('large')); await flush();
  assert.equal(published.at(-1), full); assert.equal(controller.stats.cacheEntries, 0);
  assert.equal(controller.stats.retainedBytes, previewBytes('large', full));
  const second = controller.retain(request('large')); assert.equal(calls, 1);
  first(); assert.equal(controller.stats.consumers, 1);
  second(); assert.equal(controller.stats.retainedBytes, 0);
  const back = controller.retain(request('large')); await flush(); assert.equal(calls, 2);
  assert.equal(published.at(-1), full); back(); controller.dispose();
});

test('LRU and negative cache stay within count and string budgets after a long directory traversal', async () => {
  const controller = new FilePreviewController(async path => path.endsWith('0') ? null : preview('全文'.repeat(40)), () => {}, { maxEntries: 8, maxBytes: 1500 });
  for (let i = 0; i < 500; i++) {
    const release = controller.retain(request(String(i))); await flush(); release();
    assert.ok(controller.stats.cacheEntries <= 8); assert.ok(controller.stats.cacheBytes <= 1500);
    assert.equal(controller.stats.consumers, 0); assert.equal(controller.stats.retainedBytes, controller.stats.cacheBytes);
  }
  controller.dispose(); assert.equal(controller.stats.cacheBytes, 0);
});

test('removed versions including A to B to A discard old completions and do not free the new job', async () => {
  const loads = [], values = [];
  const controller = new FilePreviewController(() => { const d = deferred(); loads.push(d); return d.promise; }, (key, p) => values.push([key, p]));
  controller.prune(['a']); const first = controller.retain(request('a'));
  controller.prune(['b']); const b = controller.retain(request('b'));
  controller.prune(['a']); const current = controller.retain(request('a'));
  assert.equal(loads.length, 3);
  loads[0].resolve(preview('old A')); loads[1].resolve(preview('old B')); await flush();
  assert.equal(values.filter(([, p]) => p !== undefined).length, 0);
  loads[2].resolve(preview('new A')); await flush();
  assert.equal(values.at(-1)[1].text, 'new A'); assert.equal(controller.stats.active, 0);
  first(); b(); assert.equal(controller.stats.consumers, 1);
  current(); controller.dispose();
});

test('list mode drops display consumers and waiting jobs, while same-version running IO is reused on return', async () => {
  const d = deferred(), values = []; let calls = 0;
  const controller = new FilePreviewController(() => { calls++; return d.promise; }, (_, p) => values.push(p));
  const oldRelease = controller.retain(request('file')); controller.clearConsumers();
  const newRelease = controller.retain(request('file')); oldRelease(); assert.equal(controller.stats.consumers, 1);
  d.resolve(preview('full')); await flush(); assert.equal(calls, 1); assert.equal(values.at(-1).text, 'full');
  newRelease(); controller.dispose();
});

test('disposal releases all strings and suppresses both late success and failure without starting queued IO', async () => {
  const loads = [], values = [];
  const controller = new FilePreviewController(() => { const d = deferred(); loads.push(d); return d.promise; }, (_, p) => values.push(p));
  const release = Array.from({ length: 20 }, (_, i) => controller.retain(request(String(i))));
  controller.dispose(); const count = values.length;
  loads.forEach((d, i) => i % 2 ? d.reject(Error('late')) : d.resolve(preview('late full contents'))); await flush();
  release.forEach(fn => fn()); controller.retain(request('new')); controller.prune(['new']);
  assert.equal(loads.length, 4); assert.equal(values.length, count); assert.equal(controller.stats.active, 0);
  assert.equal(controller.stats.retainedBytes, 0); assert.equal(controller.stats.consumers, 0);
});

test('preview version keys preserve path boundaries and detect overwrites while string weights count full Unicode', () => {
  const record = { id: 'id', path: '/授業/"a"\n.md', size_bytes: 10, downloaded_at: 100, file_exists: true };
  const first = filePreviewRequest(record);
  assert.equal(first.path, record.path);
  for (const change of [{ id: 'other' }, { size_bytes: 11 }, { downloaded_at: 101 }, { file_exists: false }, { path: '/other' }]) assert.notEqual(filePreviewRequest({ ...record, ...change }).key, first.key);
  assert.equal(filePreviewRequest({ ...record }).key, first.key);
  const full = { kind: 'image', mime: 'image/png', data_url: '全文 👩🏽‍💻', text: '引用\n' };
  assert.equal(previewBytes(first.key, full), 2 * (first.key.length + full.kind.length + full.mime.length + full.data_url.length + full.text.length));
});
