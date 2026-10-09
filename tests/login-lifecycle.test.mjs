import test from 'node:test';
import assert from 'node:assert/strict';
import { loadLogin } from './load-login.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const names = ['login-success','login-error','login-cancelled'];
const calls = (h, name) => h.calls.filter(call => call.name === name);
function emit(h, name, payload, old = false) {
  for (const listener of h.listeners.filter(listener => listener.name === name && (old || !listener.releases))) listener.receive({ payload });
}

test('all login subscriptions register in parallel and opening waits for every handle without duplicate windows', async () => {
  const h = await loadLogin(), pending = names.map(() => deferred());
  h.configure({ listen: (listener, release) => pending[names.indexOf(listener.name)].promise.then(() => release) });
  h.login.mount(); const open = h.login.handleLogin();
  await Promise.all(Array.from({ length: 100 }, () => h.login.handleLogin())); await flush();
  assert.deepEqual(h.listeners.map(listener => listener.name), names); assert.equal(calls(h, 'openLogin').length, 0);
  pending[0].resolve(); pending[1].resolve(); await flush(); assert.equal(calls(h, 'openLogin').length, 0);
  pending[2].resolve(); await open; assert.equal(calls(h, 'openLogin').length, 1);
  assert.equal(h.state.auth.loading, true); h.login.dispose();
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1]);
});

test('closing during registration disables every queued callback and releases late handles once', async () => {
  const h = await loadLogin(), pending = deferred();
  h.configure({ listen: (listener, release) => pending.promise.then(() => release) });
  h.login.mount(); const open = h.login.handleLogin(); await flush(); h.login.dispose();
  const before = structuredClone(h.state), count = h.calls.length;
  emit(h, 'login-success', { username: 'stale' }, true); emit(h, 'login-error', 'stale', true); emit(h, 'login-cancelled', null, true);
  pending.resolve(); await open; await flush(); h.login.dispose();
  assert.deepEqual(h.state, before); assert.equal(h.calls.length, count);
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1]);
});

test('a failed registration closes the entire group, silences old callbacks and retries on the next login click', async () => {
  const h = await loadLogin(), delayed = deferred();
  h.configure({ listen: (listener, release) => listener.name === 'login-error' ? Promise.reject(new Error('registration failed'))
    : listener.name === 'login-cancelled' ? delayed.promise.then(() => release) : release });
  h.login.mount(); await h.login.handleLogin(); await flush();
  assert.equal(calls(h, 'openLogin').length, 0);
  assert.equal(h.state.auth.error, 'registration failed');
  assert.equal(h.listeners[0].releases, 1);
  emit(h, 'login-success', { username: 'old' }, true); assert.equal(h.state.auth.authenticated, false);
  const old = h.listeners.slice(); h.configure({}); await h.login.handleLogin();
  assert.equal(h.listeners.length, 6); assert.equal(calls(h, 'openLogin').length, 1);
  delayed.resolve(); await flush();
  old.forEach(listener => listener.receive({ payload: 'old' }));
  assert.equal(h.state.auth.error, ''); assert.equal(h.state.auth.authenticated, false);
  h.login.dispose(); assert.deepEqual(h.listeners.map(listener => listener.releases), [1,0,1,1,1,1]);
});

test('active success, error and cancellation retain all identity fields and existing loading behavior', async () => {
  const h = await loadLogin(); h.login.mount(); await flush();
  const identity = { username: 'full', display_name: '完全な名前', student_id: '123', faculty: '理工', department: '情報', extra: 'preserved' };
  emit(h, 'login-success', identity); assert.deepEqual(calls(h, 'session')[0].value, identity);
  assert.equal(calls(h, 'startPolling').length, 1); assert.equal(h.state.auth.authenticated, true);
  emit(h, 'login-error', 'SSO failure'); assert.equal(h.state.auth.error, 'SSO failure'); assert.equal(h.state.auth.loading, false);
  emit(h, 'login-error', ''); assert.equal(h.state.auth.error, 'ログインに失敗しました');
  await h.login.handleLogin(); emit(h, 'login-cancelled'); assert.equal(h.state.auth.loading, false);
  assert.equal(h.state.auth.retained, 'unchanged'); h.login.dispose();
});

test('late window failures after page disposal do not change the successor auth state or start new actions', async () => {
  const h = await loadLogin(), window = deferred(); h.configure({ open: () => window.promise });
  h.login.mount(); await flush(); const open = h.login.handleLogin(); await flush();
  h.login.dispose(); h.state.auth = { authenticated: true, username: 'next', loading: false, error: '' };
  const before = structuredClone(h.state.auth), count = h.calls.length;
  window.reject(new Error('late failure')); await open;
  await h.login.handleLogin(); await h.login.startDemoMode(); h.login.handleLogoClick();
  h.login.minimizeWindow(); h.login.toggleMaximize(); h.login.closeWindow();
  emit(h, 'login-error', 'stale', true); assert.deepEqual(h.state.auth, before); assert.equal(h.calls.length, count);
});

test('failed window opening preserves native error text and permits another attempt', async () => {
  for (const error of [new Error('native failure'), { message: 'native failure' }, 'native failure']) {
    const h = await loadLogin(); h.configure({ open: () => Promise.reject(error) });
    h.login.mount(); await flush(); await h.login.handleLogin();
    assert.equal(h.state.auth.error, 'native failure'); assert.equal(h.state.auth.loading, false);
    h.configure({}); await h.login.handleLogin(); assert.equal(calls(h, 'openLogin').length, 2);
    assert.equal(h.state.auth.error, ''); h.login.dispose();
  }
});

test('logo timer replacement ignores queued old timeouts, preserves seven-tap confirmation and cleans up on closing', async () => {
  const h = await loadLogin(); h.login.handleLogoClick(); const old = h.timers[0];
  h.login.handleLogoClick(); old.callback(); assert.equal(h.login.state.logoTapCount, 2);
  assert.equal(h.liveTimers.size, 1); assert.equal(old.delay, 3000);
  for (let i = 0; i < 5; i++) h.login.handleLogoClick();
  assert.equal(h.login.state.showDemoConfirm, true); assert.equal(h.liveTimers.size, 0);
  h.login.cancelDemoMode(); assert.equal(h.login.state.showDemoConfirm, false);
  h.login.handleLogoClick(); h.login.dispose(); const before = h.login.state;
  h.timers.forEach(timer => timer.callback()); assert.deepEqual(h.login.state, before);
  assert.equal(h.liveTimers.size, 0); assert.ok(h.timers.every(timer => timer.clears === 1));
});

test('current logo timeout resets the sequence and current window controls retain their actions', async () => {
  const h = await loadLogin(); for (let i = 0; i < 6; i++) h.login.handleLogoClick();
  h.timers.at(-1).callback(); assert.equal(h.login.state.logoTapCount, 0);
  h.login.handleLogoClick(); assert.equal(h.login.state.showDemoConfirm, false);
  h.login.minimizeWindow(); h.login.toggleMaximize(); h.login.closeWindow();
  assert.deepEqual(h.calls.map(call => call.name), ['minimize','toggleMaximize','close']); h.login.dispose();
});

test('demo entry shares a pending attempt, gets a lifetime guard and discards failures after closing', async () => {
  const h = await loadLogin(), demo = deferred(); h.configure({ demo: () => demo.promise });
  const start = h.login.startDemoMode(); const duplicate = h.login.startDemoMode(); await flush();
  assert.equal(calls(h, 'enterDemo').length, 1); const current = calls(h, 'enterDemo')[0].current;
  assert.equal(current(), true); h.login.dispose(); assert.equal(current(), false);
  const before = structuredClone(h.state); demo.reject(new Error('late demo')); await Promise.all([start, duplicate]);
  assert.deepEqual(h.state, before);
});

test('one throwing unsubscribe cannot keep other login subscriptions or the logo timer alive', async () => {
  const h = await loadLogin(); h.configure({ release: listener => { if (listener.name === 'login-error') throw new Error('release failure'); } });
  h.login.mount(); await flush(); h.login.handleLogoClick(); h.login.dispose(); h.login.dispose();
  assert.deepEqual(h.listeners.map(listener => listener.releases), [1,1,1]); assert.equal(h.liveTimers.size, 0);
  assert.ok(h.warnings.some(message => message.includes('release failure')));
});
