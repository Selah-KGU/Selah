import test from 'node:test';
import assert from 'node:assert/strict';
import { loadCopilotDock } from './load-copilot-dock.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const tab = id => ({ id, title: id, url: '', type: 'reader', active: true });
const emit = (h, payload) => h.listeners.forEach(l => l.receive({ payload }));

// Both fail with the previous script: its initial read precedes registration,
// and a late refresh response overwrites newly received tabs.
test('dock subscribes before initial reading and pushed tabs supersede the pending snapshot', async () => {
  const h = await loadCopilotDock(); const pending = deferred(); h.configure(() => pending.promise);
  void h.dock.mount(); await flush();
  assert.deepEqual(h.steps, ['document-tabs-changed', 'document_tabs_list']);
  emit(h, { owner: 'document-tabs', tabs: [tab('new')] });
  pending.resolve([tab('old')]); await flush();
  assert.deepEqual(h.dock.state.tabs.map(t => t.id), ['new']); h.dock.dispose();
});

test('dock rejects old refresh results after an event while retaining foreign-owner filtering', async () => {
  const h = await loadCopilotDock(); h.configure(() => [tab('initial')]); void h.dock.mount(); await flush();
  const pending = deferred(); h.configure(() => pending.promise);
  const read = h.dock.refresh(); await flush();
  emit(h, { owner: 'document-tabs', tabs: [tab('new')] }); pending.resolve([tab('old')]); await read;
  assert.deepEqual(h.dock.state.tabs.map(t => t.id), ['new']);
  h.dock.dispose();
});

test('closing before listener registration completes silences callbacks and releases the late handle once', async () => {
  const h = await loadCopilotDock(); const pending = deferred(); let release;
  h.configure(() => [tab('late')], (listener, cleanup) => { release = cleanup; return pending.promise; });
  void h.dock.mount(); await flush(); assert.equal(h.listeners.length, 1);
  h.dock.dispose(); const before = h.dock.state;
  emit(h, { owner: 'document-tabs', tabs: [tab('closed')] });
  pending.resolve(release); await flush();
  assert.equal(h.listeners[0].releases, 1); assert.equal(h.calls.length, 0);
  assert.deepEqual(h.dock.state, before);
  h.dock.hide(true); h.dock.hide(false); await h.dock.refresh();
  h.dock.dispose(); assert.equal(h.listeners[0].releases, 1); assert.equal(h.calls.length, 0);
});

test('closing during a read prevents late state changes and future native actions', async () => {
  const h = await loadCopilotDock(); const pending = deferred(); h.configure(() => pending.promise);
  void h.dock.mount(); await flush(); const before = h.dock.state;
  h.dock.dispose(); pending.resolve([tab('late')]); await flush();
  emit(h, { owner: 'document-tabs', tabs: [tab('after close')] });
  h.dock.newTab(); h.dock.revealTab('closed'); h.dock.closeTab('closed'); await h.dock.refresh();
  assert.deepEqual(h.dock.state, before); assert.equal(h.calls.length, 1);
});

test('visibility catches up once on returning; animation pause and hidden events remain live', async () => {
  const h = await loadCopilotDock(); h.configure(() => [tab('initial')]); void h.dock.mount(); await flush();
  h.dock.hide(true); assert.equal(h.dock.state.docHidden, true);
  emit(h, { owner: 'document-tabs', tabs: [tab('hidden update')] });
  assert.equal(h.dock.state.tabs[0].id, 'hidden update');
  assert.equal(h.calls.length, 1);
  h.dock.hide(false); h.dock.hide(false); await flush();
  assert.equal(h.calls.length, 2); assert.equal(h.dock.state.docHidden, false);
  h.dock.dispose();
});

test('visibility changes during registration do not open a gap before event subscription', async () => {
  const h = await loadCopilotDock(); const pending = deferred(); let release;
  h.configure(() => [tab('ready')], (listener, cleanup) => { release = cleanup; return pending.promise; });
  void h.dock.mount(); await flush();
  for (let i = 0; i < 100; i++) { h.dock.hide(true); h.dock.hide(false); }
  assert.equal(h.listeners.length, 1); assert.equal(h.calls.length, 0);
  pending.resolve(release); await flush();
  assert.equal(h.calls.length, 1); assert.equal(h.dock.state.tabs[0].id, 'ready');
  h.dock.dispose();
});

test('bursts of recovery requests have one in-flight read and one merged follow-up', async () => {
  const h = await loadCopilotDock(); h.configure(() => [tab('initial')]); void h.dock.mount(); await flush();
  const requests = []; let active = 0, maximum = 0;
  h.configure(() => { const p = deferred(); requests.push(p); maximum = Math.max(maximum, ++active); return p.promise.finally(() => active--); });
  const first = h.dock.refresh(); await flush();
  const burst = Array.from({ length: 100 }, () => h.dock.refresh()); await flush();
  assert.equal(requests.length, 1);
  requests[0].resolve([tab('first')]); await first; await flush();
  assert.equal(requests.length, 2);
  requests[1].resolve([tab('latest')]); await Promise.all(burst);
  assert.equal(maximum, 1); assert.equal(h.dock.state.tabs[0].id, 'latest'); h.dock.dispose();
});

test('foreign-owner events do not invalidate the current read, and empty own state clears closed tabs', async () => {
  const h = await loadCopilotDock(); const pending = deferred(); h.configure(() => pending.promise);
  void h.dock.mount(); await flush();
  emit(h, { owner: 'another-window', tabs: [tab('foreign')] });
  pending.resolve([tab('mine')]); await flush(); assert.equal(h.dock.state.tabs[0].id, 'mine');
  emit(h, { owner: 'document-tabs', tabs: [] }); assert.deepEqual(h.dock.state.tabs, []);
  h.dock.dispose();
});

test('read failures preserve pushed state and a later recovery retries', async () => {
  const h = await loadCopilotDock(); h.configure(() => [tab('initial')]); void h.dock.mount(); await flush();
  const pending = deferred(); h.configure(() => pending.promise); const read = h.dock.refresh(); await flush();
  emit(h, { owner: 'document-tabs', tabs: [tab('new')] }); pending.reject(new Error('old failure')); await read;
  assert.equal(h.dock.state.tabs[0].id, 'new');
  h.configure(() => Promise.reject(new Error('current failure'))); await h.dock.refresh();
  assert.equal(h.dock.state.tabs[0].id, 'new');
  h.configure(() => [tab('recovered')]); h.dock.hide(true); h.dock.hide(false); await flush();
  assert.equal(h.dock.state.tabs[0].id, 'recovered'); h.dock.dispose();
});

test('dock commands remain serialized, preserve close focus=false, and invalidate pre-action reads', async () => {
  const h = await loadCopilotDock(); h.configure(() => [tab('initial')]); void h.dock.mount(); await flush();
  const read = deferred(), close = deferred();
  h.configure(command => command === 'document_tabs_list' ? read.promise : close.promise);
  const refreshing = h.dock.refresh(); await flush();
  h.dock.closeTab('initial'); h.dock.newTab(); h.dock.revealTab('initial');
  assert.equal(h.calls.filter(c => c.command !== 'document_tabs_list').length, 1);
  assert.deepEqual(h.calls.at(-1), { command: 'document_tabs_close', args: { owner: 'document-tabs', id: 'initial', focus: false } });
  emit(h, { owner: 'document-tabs', tabs: [] }); read.resolve([tab('initial')]); await refreshing;
  assert.deepEqual(h.dock.state.tabs, []); assert.equal(h.dock.state.busy, true);
  close.resolve(); await flush(); assert.equal(h.dock.state.busy, false); h.dock.dispose();
});

test('a failed subscription is retried when visible again and its obsolete callback stays disabled', async () => {
  const h = await loadCopilotDock();
  h.configure(() => [tab('recovered')], () => Promise.reject(new Error('registration failed')));
  const warn = console.warn, warnings = [];
  console.warn = (...args) => warnings.push(args);
  try {
    void h.dock.mount(); await flush(); assert.equal(h.calls.length, 0);
    assert.equal(warnings.length, 1);
    const obsolete = h.listeners[0];
    h.configure(() => [tab('recovered')]); h.dock.hide(true); h.dock.hide(false); await flush();
    assert.equal(h.listeners.length, 2); assert.equal(h.calls.length, 1);
    assert.equal(h.dock.state.tabs[0].id, 'recovered');
    obsolete.receive({ payload: { owner: 'document-tabs', tabs: [tab('obsolete')] } });
    assert.equal(h.dock.state.tabs[0].id, 'recovered');
    h.dock.dispose(); assert.equal(h.listeners[1].releases, 1);
  } finally { console.warn = warn; h.dock.dispose(); }
});
