import test from 'node:test';
import assert from 'node:assert/strict';
import { loadAppStartup } from './load-app-startup.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const calls = (h, name) => h.calls.filter(call => call.name === name);
const logout = h => h.listeners.find(listener => listener.name === 'logout').receive({ payload: null });

test('root subscribes before activity reads and reopens active LIVE while university recovery is pending', async () => {
  const h = await loadAppStartup(), pending = deferred();
  h.configure({ live: () => true, restore: () => pending.promise }); h.app.mount(); await flush();
  assert.equal(h.listeners.length, 1);
  assert.equal(calls(h, 'live_has_active_session').length, 1);
  assert.equal(calls(h, 'live_get_session').length, 0);
  assert.equal(h.state.activeTab, 'live'); assert.equal(h.app.state.everLoggedIn, true);
  assert.equal(h.app.state.restoring, true);
  pending.resolve({ valid: true }); await flush();
  assert.equal(h.app.state.restoring, false); assert.equal(calls(h, 'startPolling').length, 1);
  assert.equal(calls(h, 'startTray').length, 1); assert.equal(calls(h, 'checkUpdate').length, 1);
  h.app.dispose();
});

test('destroying during logout registration releases the late handle and prevents startup reads', async () => {
  const h = await loadAppStartup(), ready = deferred(); let cleanup;
  h.configure({ listen: (listener, release) => { cleanup = release; return ready.promise; } });
  h.app.mount(); await flush(); h.app.dispose();
  const before = h.calls.length; logout(h); ready.resolve(cleanup); await flush();
  assert.equal(h.listeners[0].releases, 1); assert.equal(h.calls.length, before);
  assert.equal(calls(h, 'restoreAllSessions').length, 0); assert.equal(calls(h, 'live_has_active_session').length, 0);
});

test('logout invalidates pending activity and university restoration before late results can restart work', async () => {
  const h = await loadAppStartup({ 'selah-ever-auth': '1', 'selah-ever-auth-source': 'real' });
  const live = deferred(), restore = deferred();
  h.configure({ live: () => live.promise, restore: () => restore.promise }); h.app.mount(); await flush();
  const current = calls(h, 'restoreAllSessions')[0].current;
  assert.equal(current(), true); logout(h); await flush(); assert.equal(current(), false);
  assert.deepEqual(h.app.state, { demoBootFlag: false, everLoggedIn: false, restoring: false });
  live.resolve(true); restore.resolve({ valid: true }); await flush();
  assert.equal(h.state.activeTab, 'home'); assert.equal(calls(h, 'startPolling').length, 0);
  assert.equal(calls(h, 'startTray').length, 0); assert.equal(calls(h, 'checkUpdate').length, 0);
  assert.equal(h.storage.has('selah-ever-auth'), false); assert.equal(h.storage.has('selah-ever-auth-source'), false);
  assert.equal(calls(h, 'invalidateCache').length, 1); h.app.dispose();
});

test('destroying during reads discards late failures and never starts tray, polling or update checks', async () => {
  const h = await loadAppStartup({ 'selah-ever-auth': '1' }), live = deferred(), restore = deferred();
  h.configure({ live: () => live.promise, restore: () => restore.promise }); h.app.mount(); await flush();
  h.app.dispose(); live.reject(new Error('late LIVE')); restore.reject(new Error('late restore')); await flush();
  assert.equal(calls(h, 'startPolling').length, 0); assert.equal(calls(h, 'startTray').length, 0);
  assert.equal(calls(h, 'checkUpdate').length, 0); assert.deepEqual(h.warnings, []);
  assert.equal(h.state.sessionExpired, false); assert.equal(h.listeners[0].releases, 1);
});

test('demo boot skips university restore and retains tray and update check behavior', async () => {
  const h = await loadAppStartup({ 'selah-demo-mode': '1' }); h.configure({ demo: true });
  h.app.mount(); await flush(); assert.equal(h.app.state.demoBootFlag, true);
  assert.equal(h.app.state.restoring, false); assert.equal(calls(h, 'restoreAllSessions').length, 0);
  assert.equal(calls(h, 'startPolling').length, 0); assert.equal(calls(h, 'startTray').length, 1);
  assert.equal(calls(h, 'checkUpdate').length, 1); logout(h); await flush();
  assert.equal(h.state.demoMode, false); assert.equal(h.app.state.demoBootFlag, false); h.app.dispose();
});

test('cached returning users retain the expired-session badge when restoration returns null or fails', async () => {
  for (const failure of [false, true]) {
    const h = await loadAppStartup({ 'selah-ever-auth': '1', 'selah-ever-auth-source': 'real' });
    h.configure({ restore: () => failure ? Promise.reject(new Error('offline')) : null }); h.app.mount(); await flush();
    assert.equal(h.state.sessionExpired, true); assert.equal(calls(h, 'startPolling').length, 1);
    assert.equal(h.app.state.restoring, false); assert.equal(calls(h, 'checkUpdate').length, 1); h.app.dispose();
  }
});

test('logout before a late registration completes prevents demo activation and real restoration', async () => {
  const h = await loadAppStartup({ 'selah-demo-mode': '1' }), ready = deferred(); let cleanup;
  h.configure({ demo: true, listen: (listener, release) => { cleanup = release; return ready.promise; } });
  h.app.mount(); await flush(); logout(h); ready.resolve(cleanup); await flush();
  assert.equal(calls(h, 'restoreDemo').length, 0); assert.equal(calls(h, 'restoreAllSessions').length, 0);
  assert.equal(calls(h, 'startTray').length, 0); h.app.dispose();
});
