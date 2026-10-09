import test from 'node:test';
import assert from 'node:assert/strict';
import { loadVisibleLogin } from './load-visible-login.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const names = ['login-success','university-login-complete','login-error','login-cancelled'];
const complete = { luna_authenticated: true, kwic_authenticated: true };
const calls = (h, name) => h.calls.filter(([event]) => event === name);
function emit(h, name, payload, old = false) {
  for (const listener of h.listeners.filter(listener => listener.name === name && (old || !listener.releases))) listener.receive({ payload });
}

test('100 re-login requests share a promise, four subscriptions, one window and one refresh after completion', async () => {
  const h = await loadVisibleLogin(); const runs = Array.from({ length: 100 }, () => h.initiateRelogin());
  assert.ok(runs.every(run => run === runs[0])); await flush();
  assert.deepEqual(h.listeners.map(listener => listener.name), names); assert.equal(calls(h, 'open').length, 1);
  assert.equal(h.state.running, true); emit(h, 'university-login-complete', complete);
  assert.deepEqual(await Promise.all(runs), Array(100).fill(complete));
  assert.deepEqual(calls(h, 'poll'), [['poll',false]]);
  assert.deepEqual(calls(h, 'progress'), [['progress',true],['progress',false]]);
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1,1]);
});

test('all four subscriptions register together and the window waits for every handle', async () => {
  const h = await loadVisibleLogin(), pending = names.map(() => deferred());
  h.configure({ listen: (listener, release) => pending[names.indexOf(listener.name)].promise.then(() => release) });
  const run = h.initiateRelogin(); await flush(); assert.deepEqual(h.listeners.map(listener => listener.name), names);
  pending[0].resolve(); pending[1].resolve(); pending[2].resolve(); await flush();
  assert.equal(calls(h, 'open').length, 0); pending[3].resolve(); await flush(); assert.equal(calls(h, 'open').length, 1);
  emit(h, 'university-login-complete', complete); assert.deepEqual(await run, complete);
});

test('terminal events during registration settle once, release late handles and never open a window afterward', async () => {
  for (const event of ['university-login-complete','login-error','login-cancelled']) {
    const h = await loadVisibleLogin(), pending = names.map(() => deferred());
    h.configure({ listen: (listener, release) => pending[names.indexOf(listener.name)].promise.then(() => release) });
    const run = h.initiateRelogin(); await flush();
    assert.deepEqual(h.listeners.map(listener => listener.name), names);
    emit(h, event, complete); assert.deepEqual(await run, event === 'university-login-complete' ? complete : null);
    const before = structuredClone(h.state), count = h.calls.length;
    emit(h, 'login-success', { username: 'stale' }, true); emit(h, 'login-error', 'stale', true);
    emit(h, 'university-login-complete', { luna_authenticated: false, kwic_authenticated: false }, true);
    for (const index of [3,0,2,1]) { pending[index].resolve(); await flush(); }
    assert.deepEqual(h.state, before); assert.equal(h.calls.length, count);
    assert.equal(calls(h, 'open').length, 0); assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1,1]);
    assert.equal(h.warnings.length, event === 'login-error' ? 1 : 0);
  }
});

test('one registration failure releases the group, preserves the failure and allows a fresh request', async () => {
  const h = await loadVisibleLogin(), late = deferred();
  h.configure({ listen: (listener, release) => listener.name === 'login-error' ? Promise.reject(new Error('subscribe failure'))
    : listener.name === 'login-cancelled' ? late.promise.then(() => release) : release });
  assert.equal(await h.initiateRelogin(), null); assert.equal(h.state.running, false);
  assert.equal(calls(h, 'open').length, 0); assert.equal(calls(h, 'poll').length, 0);
  assert.ok(h.warnings[0].includes('subscribe failure'));
  const old = h.listeners.slice(); h.configure({}); const next = h.initiateRelogin(); await flush();
  late.resolve(); await flush(); old.forEach(listener => listener.receive({ payload: { username: 'stale' } }));
  assert.equal(h.state.identity, null); assert.equal(h.state.running, true);
  emit(h, 'university-login-complete', complete); assert.deepEqual(await next, complete);
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,0,1,1,1,1,1]);
});

test('identity success keeps every field but waits for core completion before resolving or refreshing', async () => {
  const h = await loadVisibleLogin(); let settled = false;
  const run = h.initiateRelogin().then(value => { settled = true; return value; }); await flush();
  const identity = { username: 'full', display_name: '完全な名前', student_id: '123', faculty: '理工', department: '情報', extra: { complete: true } };
  emit(h, 'login-success', identity); await flush(); assert.equal(h.state.identity, identity);
  assert.equal(settled, false); assert.equal(calls(h, 'poll').length, 0); assert.equal(h.state.running, true);
  const partial = { luna_authenticated: true, kwic_authenticated: false };
  emit(h, 'university-login-complete', partial); assert.deepEqual(await run, partial); assert.equal(calls(h, 'poll').length, 1);
});

test('cancel and error preserve private rejection messages and public null results without refreshing', async () => {
  for (const [name, message] of [['login-cancelled','__login_cancelled__'],['login-error','再ログインに失敗しました']]) {
    const h = await loadVisibleLogin(); const run = h.openVisibleLogin();
    const rejected = assert.rejects(run, error => error.message === message); await flush(); emit(h, name, 'native detail'); await rejected;
    assert.equal(h.state.running, false); assert.equal(calls(h, 'poll').length, 0);
    assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1,1]);
  }
});

test('a late open rejection from a completed attempt cannot clear a new attempt or its progress', async () => {
  const h = await loadVisibleLogin(), oldOpen = deferred(); h.configure({ open: () => oldOpen.promise });
  const first = h.initiateRelogin(); await flush(); const old = h.listeners.slice();
  emit(h, 'university-login-complete', complete); await first;
  h.configure({}); const next = h.initiateRelogin(); await flush(); const before = h.calls.length;
  oldOpen.reject(new Error('late window failure')); old.forEach(listener => listener.receive({ payload: complete })); await flush();
  assert.equal(h.state.running, true); assert.equal(h.calls.length, before); assert.deepEqual(h.warnings, []);
  emit(h, 'login-cancelled'); assert.equal(await next, null); assert.equal(h.state.running, false);
});

test('a failed open releases all subscriptions and retains the native error for the next retry', async () => {
  const h = await loadVisibleLogin(); h.configure({ open: () => Promise.reject(new Error('window unavailable')) });
  assert.equal(await h.initiateRelogin(), null); assert.equal(h.state.running, false);
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1,1]); assert.ok(h.warnings[0].includes('window unavailable'));
  h.configure({}); const next = h.initiateRelogin(); await flush(); emit(h, 'university-login-complete', complete);
  assert.deepEqual(await next, complete); assert.equal(calls(h, 'open').length, 2);
});

test('cleanup exceptions still release every subscription, settle completion and clear progress', async () => {
  const h = await loadVisibleLogin(); h.configure({ release: listener => { if (listener.name === 'login-error') throw new Error('cleanup failure'); } });
  const run = h.initiateRelogin(); await flush(); emit(h, 'university-login-complete', complete);
  assert.deepEqual(await run, complete); assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1,1]);
  assert.equal(h.state.running, false); assert.ok(h.warnings.some(message => message.includes('cleanup failure')));
});

test('store reentrancy joins the current request before registrations or a second window can start', async () => {
  const h = await loadVisibleLogin(); let joined;
  h.configure({ progress: running => { if (running) joined = h.initiateRelogin(); } });
  const first = h.initiateRelogin(); await flush(); assert.equal(joined, first); assert.equal(h.listeners.length, 4);
  emit(h, 'university-login-complete', complete); assert.deepEqual(await joined, complete);
  assert.equal(calls(h, 'poll').length, 1);
});

test('demo re-login retains its result and expiry reset without native subscriptions or window requests', async () => {
  const h = await loadVisibleLogin(); h.configure({ demo: true });
  assert.deepEqual(await h.initiateRelogin(), complete); assert.equal(h.state.expired, false);
  assert.equal(h.listeners.length, 0); assert.deepEqual(h.calls, [['expired',false]]);
});


test('older native login notifications cannot overwrite a new identity or complete its attempt', async () => {
  const h = await loadVisibleLogin();
  const run = h.initiateRelogin(); await flush();
  emit(h, 'login-success', { username: 'new', generation: 2 });
  emit(h, 'login-success', { username: 'old', generation: 1 });
  emit(h, 'university-login-complete', { ...complete, generation: 1 });
  await flush();
  assert.equal(h.state.identity.username, 'new');
  assert.equal(h.state.running, true);
  emit(h, 'university-login-complete', { ...complete, generation: 2 });
  assert.equal((await run).generation, 2);
});
