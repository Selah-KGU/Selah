import test from 'node:test';
import assert from 'node:assert/strict';
import { loadDashboard } from './load-dashboard.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const names = ['tray-open-tab', 'open-agent-tab', 'ai-config-changed', 'live-todo-suggestions'];
function emit(h, name, payload) {
  for (const listener of h.listeners.filter(listener => listener.name === name)) listener.receive({ payload });
}

test('all four independent subscriptions start before a slow registration completes', async () => {
  const h = await loadDashboard(), pending = deferred();
  h.configure({ listen: (listener, release) => pending.promise.then(() => release) });
  h.dashboard.mount(); await flush();
  assert.deepEqual(h.listeners.map(listener => listener.name), names);
  assert.equal(h.calls.filter(call => call.key === 'ai-readiness').length, 1);
  h.dashboard.dispose(); pending.resolve(); await flush();
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1, 1, 1, 1]);
});

test('closing during registration silences all late events and releases every late handle once', async () => {
  const h = await loadDashboard(), pending = names.map(() => deferred());
  h.configure({ listen: (listener, release) => pending[names.indexOf(listener.name)].promise.then(() => release) });
  h.dashboard.mount(); await flush(); h.dashboard.dispose();
  const before = structuredClone(h.state), calls = h.calls.length;
  for (const [name, payload] of [['tray-open-tab', 'live'], ['open-agent-tab'], ['ai-config-changed'],
    ['live-todo-suggestions', { source_path: 'new.md', suggestions: [{ title: 'late' }] }]]) emit(h, name, payload);
  for (const index of [3, 1, 0, 2]) { pending[index].resolve(); await flush(); }
  h.dashboard.dispose();
  assert.deepEqual(h.state, before); assert.equal(h.calls.length, calls);
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1, 1, 1, 1]);
  assert.deepEqual(h.caches.map(cache => cache.releases), [1, 1, 1, 1]);
});

test('active callbacks retain tray navigation, AI refresh and complete LIVE TODO payloads', async () => {
  const h = await loadDashboard(); h.dashboard.mount(); await flush();
  emit(h, 'tray-open-tab', 'live'); assert.equal(h.state.activeTab, 'live');
  emit(h, 'tray-open-tab', ''); assert.equal(h.state.activeTab, 'live');
  emit(h, 'open-agent-tab'); assert.equal(h.state.activeTab, 'agent');
  emit(h, 'ai-config-changed'); assert.equal(h.calls.filter(call => call.key === 'ai-readiness').length, 2);
  const suggestions = [{ title: '全文 👩🏽‍💻', course: '授業', due: '2026-10-08', extra: { complete: true } }];
  emit(h, 'live-todo-suggestions', { suggestions, source_path: '/tmp/全文.md' });
  assert.deepEqual(h.state.liveTodoDrafts, { suggestions, sourcePath: '/tmp/全文.md' });
  assert.equal(h.state.liveTodoPending, false);
  emit(h, 'live-todo-suggestions', { suggestions: [] }); assert.equal(h.state.liveTodoDrafts, null);
  h.dashboard.dispose();
});

test('late callbacks from an old dashboard cannot change a newly mounted dashboard state', async () => {
  const old = await loadDashboard(), pending = deferred();
  old.configure({ listen: (listener, release) => pending.promise.then(() => release) });
  old.dashboard.mount(); await flush(); old.dashboard.dispose();
  const next = await loadDashboard(); next.dashboard.mount(); await flush();
  // Production stores are shared across instances. Connect the isolated store
  // boundary to the successor so a stale setter would affect that successor.
  Object.defineProperty(old.state, 'activeTab', { get: () => next.state.activeTab, set: value => { next.state.activeTab = value; } });
  pending.resolve(); await flush();
  emit(old, 'tray-open-tab', 'live'); emit(old, 'open-agent-tab');
  assert.equal(next.state.activeTab, 'home');
  emit(next, 'tray-open-tab', 'todo'); assert.equal(next.state.activeTab, 'todo');
  next.dashboard.dispose();
  assert.deepEqual(old.listeners.map(listener => listener.releases), [1, 1, 1, 1]);
  assert.deepEqual(next.listeners.map(listener => listener.releases), [1, 1, 1, 1]);
});

test('late onboarding and read-ID completion cannot reopen UI or recalculate badges after closing', async () => {
  const h = await loadDashboard(), onboarding = deferred();
  assert.equal(h.readIdsCompletion.length, 1);
  h.configure({ onboarding: () => onboarding.promise,
    cache: { notifications: { entries: [{ id: 'unread' }] } } });
  h.dashboard.mount(); await flush(); h.dashboard.dispose();
  const before = structuredClone(h.state), calls = h.calls.length;
  onboarding.resolve(true); h.readIdsCompletion[0](); await flush();
  for (const cache of h.caches) cache.receive([{ isRead: false }]);
  assert.deepEqual(h.state, before); assert.equal(h.calls.length, calls);
});

test('late successful and failed page imports are discarded; mounted imports still work and deduplicate', async () => {
  for (const fails of [false, true]) {
    const h = await loadDashboard(), pending = deferred(); let reads = 0;
    h.dashboard.loader('mail', () => { reads++; return pending.promise; });
    const importing = h.dashboard.ensureViewLoaded('mail');
    await h.dashboard.ensureViewLoaded('mail'); assert.equal(reads, 1);
    h.dashboard.dispose();
    if (fails) pending.reject(new Error('late page')); else pending.resolve({ default: { page: 'late' } });
    await importing; assert.deepEqual(h.dashboard.views, { lazyViews: {}, lazyErrors: {} });
    await h.dashboard.ensureViewLoaded('mail'); assert.equal(reads, 1);
  }
  const h = await loadDashboard(); const component = { page: 'loaded' };
  h.dashboard.loader('mail', async () => ({ default: component })); await h.dashboard.ensureViewLoaded('mail');
  assert.equal(h.dashboard.views.lazyViews.mail, component); h.dashboard.dispose();
});

test('a failed subscription leaves other independent events working and does not leak handles', async () => {
  const h = await loadDashboard(), pending = deferred();
  h.configure({ listen: (listener, release) => listener.name === 'open-agent-tab'
    ? Promise.reject(new Error('agent subscription failure'))
    : listener.name === 'live-todo-suggestions' ? pending.promise.then(() => release) : release });
  h.dashboard.mount(); await flush();
  assert.deepEqual(h.listeners.map(listener => listener.name), names);
  emit(h, 'tray-open-tab', 'live'); assert.equal(h.state.activeTab, 'live');
  h.dashboard.dispose(); pending.resolve(); await flush();
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1, 0, 1, 1]);
});

test('one throwing native cleanup does not keep other native or cache subscriptions alive', async () => {
  const h = await loadDashboard();
  h.configure({ release: listener => { if (listener.name === 'ai-config-changed') throw new Error('cleanup failure'); } });
  h.dashboard.mount(); await flush(); h.dashboard.dispose(); h.dashboard.dispose();
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1, 1, 1, 1]);
  assert.deepEqual(h.caches.map(cache => cache.releases), [1, 1, 1, 1]);
  assert.ok(h.warnings.some(message => message.includes('cleanup failure')));
});

test('active cache and read-ID callbacks retain exact notification and mail badge rules', async () => {
  const h = await loadDashboard();
  h.configure({ cache: {
    notifications: { entries: [{ id: 'kgc-read' }, { id: 'kgc-unread' }, { title: 'fallback', date: '10/08' }] },
    luna_updates: [{ url: 'luna-read' }, { idnumber: 'luna-unread' }],
    kwic_home: { sections: [{ title: 'メインリンク', items: [{ id: 'skip' }] },
      { title: '注目コンテンツ', items: [{ id: 'skip' }] },
      { title: 'news', items: [{ id: 'kwic-read' }, { id: 'kwic-unread' }] }] },
  } });
  h.state.readIdsStore = { kgc: ['kgc-read', 'fallback10/08'], luna: ['luna-read'], kwic: ['kwic-read'] };
  h.readIdsCompletion[0](); await flush(); assert.equal(h.state.unreadNotifCount, 3);
  h.caches.find(cache => cache.name === 'mail_inbox').receive([{ isRead: true }, { isRead: false }, { isRead: false }]);
  assert.equal(h.state.unreadMailCount, 2);
  h.state.readIdsStore.kgc.push('kgc-unread');
  h.caches.find(cache => cache.name === 'notifications').receive({}); assert.equal(h.state.unreadNotifCount, 2);
  h.dashboard.dispose();
});
