import test from 'node:test';
import assert from 'node:assert/strict';
import { loadDocumentTabs } from './load-document-tabs.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const tab = id => ({ id, target: id, title: id, type: 'reader', active: true, controls: [] });
const fallback = command => command === 'get_app_theme' ? 'light' : command === 'document_tabs_list' ? [tab('initial')] : false;
const emit = (h, name, payload) => h.listeners.filter(l => l.name === name).forEach(l => l.receive({ payload }));

// These scenarios failed against the original component: late registrations
// survived disposal, and an in-flight poll overwrote a newer pushed tab list.
test('closing during listener registration releases late subscriptions and starts no timers or reads', async () => {
  const h = await loadDocumentTabs();
  const pending = [];
  h.configure(fallback, (listener, release) => {
    const request = deferred(); pending.push({ ...request, release }); return request.promise;
  });
  void h.panel.mount(); await flush();
  assert.ok(pending.length > 0);
  h.panel.dispose();
  for (let i = 0; i < pending.length; i++) { pending[i].resolve(pending[i].release); await flush(); }
  assert.ok(h.listeners.every(l => l.releases === 1));
  assert.equal(h.clock.active.size, 0);
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_list').length, 0);
});

test('a pushed tab list and title hint survive an older poll response', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback);
  void h.panel.mount(); await flush();
  const response = deferred(); h.configure(command => command === 'document_tabs_list' ? response.promise : fallback(command));
  const read = h.panel.refresh(); await flush();
  emit(h, 'document-tabs-changed', { owner: 'document-tabs', tabs: [tab('new')] });
  emit(h, 'document-tab-title-hint', { owner: 'document-tabs', target: 'new', title: 'new title' });
  response.resolve([tab('old')]); await read;
  assert.deepEqual(h.panel.state.tabs.map(t => t.id), ['new']);
  assert.deepEqual(h.panel.state.titleHints, { new: 'new title' });
  h.panel.dispose();
});

test('a registration failure releases the whole group, including pending registrations', async () => {
  const h = await loadDocumentTabs(); const pending = [];
  h.configure(fallback, (listener, release) => {
    const request = deferred(); pending.push({ ...request, release }); return request.promise;
  });
  void h.panel.mount(); await flush();
  assert.equal(pending.length, 6);
  pending[0].resolve(pending[0].release); await flush();
  pending[1].reject(new Error('listen failed')); await flush();
  assert.match(h.panel.state.error, /listen failed/);
  for (let i = 2; i < pending.length; i++) pending[i].resolve(pending[i].release);
  await flush();
  assert.equal(h.listeners[0].releases, 1);
  assert.ok(h.listeners.slice(2).every(l => l.releases === 1));
  const state = h.panel.state, writes = h.themeWrites.length;
  emit(h, 'document-tabs-changed', { owner: 'document-tabs', tabs: [tab('late')] });
  emit(h, 'theme-changed', 'dark'); emit(h, 'app-theme-changed', null);
  assert.deepEqual(h.panel.state, state);
  assert.equal(h.themeWrites.length, writes);
  assert.equal(h.calls.length, 0);
  assert.equal(h.clock.active.size, 0);
  h.panel.dispose(); assert.equal(h.listeners[0].releases, 1);
});

test('closing during native reads blocks late state, theme writes, events and follow-up IPC', async () => {
  const h = await loadDocumentTabs(); const pending = [];
  h.configure(() => { const request = deferred(); pending.push(request); return request.promise; });
  void h.panel.mount(); await flush();
  assert.equal(h.calls.length, 3);
  h.panel.select({ ...tab('files'), type: 'files' });
  const state = h.panel.state, writes = h.themeWrites.length;
  h.panel.search(); const search = h.clock.created.find(t => t.kind === 'timeout');
  h.panel.dispose();
  pending.forEach((p, i) => p.resolve(h.calls[i].command === 'document_tabs_list' ? [tab('late')] : h.calls[i].command === 'get_app_theme' ? 'dark' : true));
  await flush();
  emit(h, 'theme-changed', 'dark'); emit(h, 'app-theme-changed', null);
  emit(h, 'document-tabs-changed', { owner: 'document-tabs', tabs: [tab('event after close')] });
  emit(h, 'document-tab-title-hint', { owner: 'document-tabs', target: 'late', title: 'late' });
  emit(h, 'document-tabs-agent-visibility', { owner: 'document-tabs', open: true });
  search.callback(); h.panel.hide(false);
  await h.panel.refresh(); await h.panel.theme(); await h.panel.run(() => h.configure(() => {}));
  assert.deepEqual(h.panel.state, state);
  assert.equal(h.themeWrites.length, writes);
  assert.equal(h.calls.length, 3);
  assert.equal(h.clock.active.size, 0);
  assert.ok(h.listeners.every(l => l.releases === 1));
});

test('100 concurrent recovery requests serialize into one follow-up read', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); void h.panel.mount(); await flush();
  const requests = []; let running = 0, maximum = 0;
  h.configure(command => {
    if (command !== 'document_tabs_list') return fallback(command);
    const p = deferred(); requests.push(p); maximum = Math.max(maximum, ++running);
    return p.promise.finally(() => running--);
  });
  const first = h.panel.refresh(); await flush();
  const burst = Array.from({ length: 100 }, () => h.panel.refresh()); await flush();
  assert.equal(requests.length, 1);
  requests[0].resolve([tab('first')]); await first; await flush();
  assert.equal(requests.length, 2);
  requests[1].resolve([tab('latest')]); await Promise.all(burst);
  assert.equal(maximum, 1);
  assert.deepEqual(h.panel.state.tabs.map(t => t.id), ['latest']);
  h.panel.dispose();
});

test('hidden polling releases its timer; resuming catches up once and stale ticks are inert', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); void h.panel.mount(); await flush();
  const old = h.clock.created.find(t => t.kind === 'interval');
  const before = h.calls.length;
  h.panel.hide(true); assert.equal(h.clock.active.size, 0);
  old.callback(); await flush(); assert.equal(h.calls.length, before);
  emit(h, 'document-tabs-changed', { owner: 'document-tabs', tabs: [tab('hidden update')] });
  assert.equal(h.panel.state.tabs[0].id, 'hidden update');
  h.panel.hide(false); h.panel.hide(false); await flush();
  assert.equal(h.clock.active.size, 1);
  assert.equal(h.calls.length, before + 1);
  old.callback(); await flush(); assert.equal(h.calls.length, before + 1);
  h.clock.created.at(-1).callback(); await flush(); assert.equal(h.calls.length, before + 2);
  h.panel.dispose(); assert.equal(h.clock.active.size, 0);
});

test('a component mounted hidden creates no recovery timer or tab read until visible', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); h.panel.hide(true);
  void h.panel.mount(); await flush();
  assert.equal(h.clock.active.size, 0);
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_list').length, 0);
  h.panel.hide(false); await flush();
  assert.equal(h.clock.active.size, 1);
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_list').length, 1);
  h.panel.dispose();
});

test('theme pushes supersede older success and fallback; app notifications coalesce reads', async () => {
  const h = await loadDocumentTabs(); const pending = [];
  h.configure(command => {
    if (command !== 'get_app_theme') return fallback(command);
    const request = deferred(); pending.push(request); return request.promise;
  });
  void h.panel.mount(); await flush();
  assert.equal(pending.length, 1);
  emit(h, 'theme-changed', 'dark'); const writes = h.themeWrites.length;
  pending[0].resolve('light'); await flush(); assert.equal(h.themeWrites.length, writes);
  const next = h.panel.theme(); await flush();
  emit(h, 'theme-changed', 'dark'); const afterPush = h.themeWrites.length;
  pending[1].reject(new Error('obsolete theme failure')); await next;
  assert.equal(h.themeWrites.length, afterPush);
  for (let i = 0; i < 100; i++) emit(h, 'app-theme-changed', null);
  await flush(); assert.equal(pending.length, 3);
  pending[2].resolve('dark'); await flush();
  assert.equal(h.themeWrites.at(-1).value, 'dark');
  const failing = h.panel.theme(); await flush();
  pending[3].reject(new Error('current theme failure')); await failing;
  assert.equal(h.themeWrites.at(-1).value, 'light'); // Stored fallback still works.
  h.panel.dispose();
});

test('agent visibility events supersede startup and toggle responses', async () => {
  const h = await loadDocumentTabs(); const pending = [];
  h.configure(command => {
    if (command !== 'document_tabs_agent_is_open') return fallback(command);
    const request = deferred(); pending.push(request); return request.promise;
  });
  void h.panel.mount(); await flush();
  emit(h, 'document-tabs-agent-visibility', { owner: 'document-tabs', open: true });
  pending[0].resolve(false); await flush(); assert.equal(h.panel.state.agentOpen, true);
  const toggle = h.panel.toggleAgent(); await flush();
  emit(h, 'document-tabs-agent-visibility', { owner: 'document-tabs', open: false });
  pending[1].resolve(true); await toggle;
  assert.equal(h.panel.state.agentOpen, false);
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_open_agent').length, 1);
  h.panel.dispose();
});

test('dragging and optimistic reorder reject pre-drag polls and preserve the native order', async () => {
  const h = await loadDocumentTabs(); h.configure(command => command === 'document_tabs_list' ? [tab('a'), tab('b')] : fallback(command));
  void h.panel.mount(); await flush();
  const old = deferred(); h.configure(command => command === 'document_tabs_list' ? old.promise : fallback(command));
  const read = h.panel.refresh(); await flush();
  h.panel.pointerDown({ button: 0, pointerId: 1, clientX: 0 }, 'a', 0);
  emit(h, 'document-tabs-changed', { owner: 'document-tabs', tabs: [tab('unexpected')] });
  const during = h.calls.length; await h.panel.refresh(); assert.equal(h.calls.length, during);
  h.panel.pointerMove({ pointerId: 1, clientX: 100, currentTarget: { offsetWidth: 98, setPointerCapture() {} } });
  h.panel.pointerUp({ pointerId: 1, currentTarget: { releasePointerCapture() {} } });
  old.resolve([tab('a'), tab('b')]); await read;
  assert.deepEqual(h.panel.state.tabs.map(t => t.id), ['b', 'a']);
  assert.deepEqual(h.calls.find(c => c.command === 'document_tabs_reorder').args.ids, ['b', 'a']);
  h.panel.dispose(); assert.equal(h.clock.active.size, 0);
});

test('mutations remain serialized and wait for a read issued after their completion', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); void h.panel.mount(); await flush();
  const reads = [], action = deferred(); let actions = 0;
  h.configure(command => { if (command !== 'document_tabs_list') return fallback(command); const request = deferred(); reads.push(request); return request.promise; });
  const oldRead = h.panel.refresh(); await flush();
  const operation = h.panel.run(() => { actions++; return action.promise; });
  await h.panel.run(() => { actions++; return Promise.resolve(); }); assert.equal(actions, 1);
  action.resolve(); await flush(); assert.equal(h.panel.state.busy, true);
  reads[0].resolve([tab('before mutation')]); await oldRead; await flush();
  assert.equal(reads.length, 2);
  assert.equal(h.panel.state.tabs[0].id, 'initial');
  reads[1].resolve([tab('after mutation')]); await operation;
  assert.equal(h.panel.state.busy, false); assert.equal(h.panel.state.tabs[0].id, 'after mutation');
  h.panel.dispose(); await h.panel.run(() => { actions++; return Promise.resolve(); }); assert.equal(actions, 1);
});

test('current read failures can retry; obsolete errors cannot replace a pushed success', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); void h.panel.mount(); await flush();
  const bad = deferred(); h.configure(command => command === 'document_tabs_list' ? bad.promise : fallback(command));
  const read = h.panel.refresh(); await flush();
  emit(h, 'document-tabs-changed', { owner: 'document-tabs', tabs: [tab('new')] });
  bad.reject(new Error('obsolete read failure')); await read; assert.equal(h.panel.state.error, '');
  h.configure(() => Promise.reject(new Error('current read failure'))); await h.panel.refresh();
  assert.match(h.panel.state.error, /current read failure/);
  h.configure(fallback); await h.panel.refresh(); assert.equal(h.panel.state.error, '');
  const pending = deferred(); h.configure(command => command === 'document_tabs_list' ? pending.promise : fallback(command));
  const finalRead = h.panel.refresh(); await flush();
  emit(h, 'document-tabs-changed', { owner: 'another-owner', tabs: [tab('foreign')] });
  pending.resolve([tab('own read')]); await finalRead; assert.equal(h.panel.state.tabs[0].id, 'own read');
  h.panel.dispose();
});

test('a queued recovery read is skipped when hidden before the current read completes', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); void h.panel.mount(); await flush();
  const pending = deferred(); h.configure(command => command === 'document_tabs_list' ? pending.promise : fallback(command));
  const tick = h.clock.created.find(t => t.kind === 'interval').callback;
  const baseline = h.calls.length;
  tick(); await flush(); tick(); await flush(); h.panel.hide(true);
  pending.resolve([tab('in flight')]); await flush();
  assert.equal(h.calls.length, baseline + 1);
  // Explicit mutation completion still requests its state, even if the window
  // was hidden during the action. Only recovery work is suspended.
  const operation = h.panel.run(() => Promise.resolve()); await flush();
  assert.equal(h.calls.length, baseline + 2); await operation;
  h.panel.dispose();
});
