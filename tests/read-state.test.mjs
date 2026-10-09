import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { loadReadState, tick, emptyIds } from './load-read-state.mjs';

test('one hundred pending read-state loads share one request and complete IDs', async () => {
  const h = await loadReadState();
  const loads = Array.from({ length: 100 }, () => h.loadReadIds());
  assert.ok(loads.every(p => p === loads[0]));
  await tick();
  assert.equal(h.requests.length, 1);
  assert.equal(h.requests[0].command, 'get_read_notifications');
  const data = { kgc: ['base'], luna: ['全文 日本語 🌕'], kwic: ['kwic'] };
  h.requests[0].resolve(data);
  await Promise.all(loads);
  assert.deepEqual(h.snapshot(), data);
});

test('a read, mark, and next read publish in order without losing old or new IDs', async () => {
  const h = await loadReadState();
  const initial = h.loadReadIds();
  const marked = h.markRead('luna', 'new');
  const final = h.loadReadIds();
  await tick();
  assert.equal(h.requests.length, 1);
  assert.equal(h.requests[0].command, 'get_read_notifications');
  h.requests[0].resolve({ ...emptyIds(), luna: ['old'] });
  await initial;
  await tick();
  assert.equal(h.requests.length, 2);
  assert.equal(h.requests[1].command, 'mark_notification_read');
  assert.deepEqual(h.snapshot().luna, ['old']);
  h.requests[1].resolve(null);
  await marked;
  await tick();
  assert.deepEqual(h.snapshot().luna, ['old', 'new']);
  assert.equal(h.requests[2].command, 'get_read_notifications');
  h.requests[2].resolve({ ...emptyIds(), luna: ['old', 'new'] });
  await final;
  assert.deepEqual(h.snapshot().luna, ['old', 'new']);
});

test('mixed notification marks run one at a time and update only after acknowledgment', async () => {
  const h = await loadReadState();
  const operations = [];
  for (let i = 0; i < 32; i++) {
    operations.push(i % 2 ? h.markBatchRead('kgc', [`id-${i}`, `id-${i}`]) : h.markRead('kgc', `id-${i}`));
  }
  for (let i = 0; i < 32; i++) {
    await tick();
    assert.equal(h.requests.length, i + 1);
    assert.equal(h.snapshot().kgc.length, i);
    const request = h.requests[i];
    assert.equal(request.command, i % 2 ? 'mark_batch_notification_read' : 'mark_notification_read');
    request.resolve(null);
    await operations[i];
  }
  assert.deepEqual(h.snapshot().kgc, Array.from({ length: 32 }, (_, i) => `id-${i}`));
});

test('failed initial read rejects its caller but does not strand queued marks', async () => {
  const h = await loadReadState();
  const initial = h.loadReadIds();
  const failed = assert.rejects(initial, /original read failure/);
  const marked = h.markRead('kwic', 'after failure');
  await tick();
  assert.equal(h.requests.length, 1);
  h.requests[0].reject(new Error('original read failure'));
  await failed;
  await tick();
  assert.equal(h.requests[1].command, 'mark_notification_read');
  h.requests[1].resolve(null);
  await marked;
  assert.deepEqual(h.snapshot().kwic, ['after failure']);
});

test('failed mark leaves the store unchanged and later writes still complete', async () => {
  const h = await loadReadState();
  const first = h.markRead('luna', 'failed');
  const failure = assert.rejects(first, /original write failure/);
  const next = h.markRead('luna', 'saved');
  await tick();
  assert.equal(h.requests.length, 1);
  assert.deepEqual(h.snapshot(), emptyIds());
  h.requests[0].reject(new Error('original write failure'));
  await failure;
  await tick();
  h.requests[1].resolve(null);
  await next;
  assert.deepEqual(h.snapshot().luna, ['saved']);
});

test('batch input is captured at admission and duplicate IDs are appended once', async () => {
  const h = await loadReadState();
  const blocking = h.loadReadIds();
  const ids = ['one', 'two', 'two', '日本語 🌕'];
  const marking = h.markBatchRead('luna', ids);
  ids.push('later mutation');
  await tick();
  assert.equal(h.requests.length, 1);
  h.requests[0].resolve({ ...emptyIds(), luna: ['one'] });
  await blocking;
  await tick();
  assert.deepEqual(h.requests[1].args, { source: 'luna', ids: ['one', 'two', 'two', '日本語 🌕'] });
  h.requests[1].resolve(null);
  await marking;
  assert.deepEqual(h.snapshot().luna, ['one', 'two', '日本語 🌕']);
});

test('old read completion does not clear the read queued after a mutation', async () => {
  const h = await loadReadState();
  const first = h.loadReadIds();
  const mark = h.markRead('luna', 'one');
  const second = h.loadReadIds();
  assert.notEqual(first, second);
  await tick();
  h.requests[0].resolve(emptyIds());
  await first;
  await tick();
  assert.equal(h.loadReadIds(), second);
  h.requests[1].resolve(null);
  await mark;
  await tick();
  assert.equal(h.requests.length, 3);
  assert.equal(h.loadReadIds(), second);
  h.requests[2].resolve({ ...emptyIds(), luna: ['one'] });
  await second;
});

test('demo reset retires old reads and unissued writes without delaying demo marks', async () => {
  const h = await loadReadState();
  const first = h.loadReadIds();
  await tick();
  assert.equal(h.requests.length, 1);
  const queued = h.markRead('luna', 'real queued');
  h.configure({ demo: true });
  await h.loadReadIds();
  await h.markBatchRead('kgc', ['demo', 'demo']);
  assert.deepEqual(h.snapshot(), { ...emptyIds(), kgc: ['demo'] });
  h.requests[0].resolve({ ...emptyIds(), luna: ['obsolete real IDs'] });
  await first;
  await queued;
  assert.equal(h.requests.length, 1);
  assert.deepEqual(h.snapshot(), { ...emptyIds(), kgc: ['demo'] });
  h.configure({ demo: false });
  const fresh = h.loadReadIds();
  await tick();
  assert.equal(h.requests.length, 2);
  h.requests[1].resolve({ ...emptyIds(), luna: ['current'] });
  await fresh;
  assert.deepEqual(h.snapshot(), { ...emptyIds(), luna: ['current'] });
});

test('an old read failure after demo reset cannot reject or overwrite the current mode', async () => {
  const h = await loadReadState();
  const old = h.loadReadIds();
  const settled = old.then(() => 'resolved', () => 'rejected');
  await tick();
  h.configure({ demo: true });
  await h.loadReadIds();
  await h.markRead('kwic', 'demo ID');
  h.requests[0].reject(new Error('obsolete old failure'));
  assert.equal(await settled, 'resolved');
  assert.deepEqual(h.snapshot().kwic, ['demo ID']);
});

for (const failed of [false, true]) {
  test(`an issued ${failed ? 'failed' : 'successful'} mark cannot change read IDs after demo reset`, async () => {
    const h = await loadReadState();
    const old = h.markBatchRead('luna', ['old real mark']);
    const settled = old.then(() => 'resolved', () => 'rejected');
    await tick();
    assert.equal(h.requests.length, 1);
    assert.equal(h.requests[0].command, 'mark_batch_notification_read');
    h.configure({ demo: true });
    await h.loadReadIds();
    await h.markRead('kwic', 'current demo');
    if (failed) h.requests[0].reject(new Error('obsolete write failure'));
    else h.requests[0].resolve(null);
    assert.equal(await settled, 'resolved');
    assert.deepEqual(h.snapshot(), { ...emptyIds(), kwic: ['current demo'] });
    assert.equal(h.requests.length, 1);
  });
}

test('marking without localStorage preserves the local-only behavior', async () => {
  const h = await loadReadState();
  h.configure({ storage: false });
  await h.markRead('luna', 'one');
  await h.markBatchRead('kwic', ['two', 'two']);
  assert.equal(h.requests.length, 0);
  assert.deepEqual(h.snapshot(), { kgc: [], luna: ['one'], kwic: ['two'] });
});

test('store callbacks can enqueue a mark and new read while the first read is publishing', async () => {
  const h = await loadReadState();
  let marking, next;
  let entered = false;
  const unsubscribe = h.readIdsStore.subscribe(data => {
    if (!entered && data.kgc.includes('base')) {
      entered = true;
      marking = h.markRead('kgc', 'from callback');
      next = h.loadReadIds();
    }
  });
  const initial = h.loadReadIds();
  await tick();
  h.requests[0].resolve({ ...emptyIds(), kgc: ['base'] });
  await initial;
  await tick();
  assert.equal(h.requests.length, 2);
  assert.equal(h.requests[1].command, 'mark_notification_read');
  h.requests[1].resolve(null);
  await marking;
  await tick();
  h.requests[2].resolve({ ...emptyIds(), kgc: ['base', 'from callback'] });
  await next;
  unsubscribe();
  assert.deepEqual(h.snapshot().kgc, ['base', 'from callback']);
});

test('native SQLite failure replies prevent false single and batch acknowledgments in the store', async () => {
  const wire = JSON.parse(await readFile('tests/fixtures/read-state-error-wire.json', 'utf8'));
  const h = await loadReadState();
  h.readIdsStore.set(wire.initial);
  for (const kind of ['single', 'batch']) {
    const mark = () => kind === 'single'
      ? h.markRead('luna', 'new single')
      : h.markBatchRead('kwic', ['new batch']);
    const before = structuredClone(h.snapshot());
    let offset = h.requests.length;
    const failed = mark().then(() => 'incorrect success', error => error);
    await tick();
    assert.equal(h.requests.length, offset + 1);
    assert.equal(h.requests[offset].command, kind === 'single' ? 'mark_notification_read' : 'mark_batch_notification_read');
    h.requests[offset].reject(wire[`${kind}_error`]);
    assert.equal(await failed, wire[`${kind}_error`]);
    assert.deepEqual(h.snapshot(), before);
    offset = h.requests.length;
    const retry = mark();
    await tick();
    assert.equal(h.requests.length, offset + 1);
    h.requests[offset].resolve(wire[`retry_${kind}`]);
    await retry;
  }
  assert.deepEqual(h.snapshot(), wire.retry_read);
});

test('native malformed-cache error replies retain the current store and permit a fresh load', async () => {
  const wire = JSON.parse(await readFile('tests/fixtures/read-state-error-wire.json', 'utf8'));
  const h = await loadReadState();
  h.readIdsStore.set(wire.retry_read);
  const failed = h.loadReadIds().then(() => 'incorrect success', error => error);
  await tick();
  assert.equal(h.requests.length, 1);
  h.requests[0].reject(wire.read_error);
  assert.equal(await failed, wire.read_error);
  assert.deepEqual(h.snapshot(), wire.retry_read);
  const retry = h.loadReadIds();
  await tick();
  assert.equal(h.requests.length, 2);
  h.requests[1].resolve(wire.retry_read);
  await retry;
  assert.deepEqual(h.snapshot(), wire.retry_read);
});
