import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSessionRestore } from './load-session-restore.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const identity = { valid: true, username: 'full', display_name: '完全な名前', student_id: '123', faculty: '理工', department: '情報' };
const changed = h => h.calls.filter(([name]) => ['auth','expired','mail','recovered','reset','setAuthFromSession'].includes(name));

test('obsolete restoration performs no IO, including demo activation', async () => {
  for (const demo of [false, true]) {
    const h = await loadSessionRestore(); h.configure({ demo });
    assert.equal(await h.restoreAllSessions(() => false), null);
    assert.deepEqual(h.calls, []); assert.deepEqual(changed(h), []);
  }
});

test('canceling during initial disk reads prevents validation, headless sync and all store writes', async () => {
  const h = await loadSessionRestore(), snapshot = deferred(), stored = deferred(); let current = true;
  h.configure({ snapshot: () => snapshot.promise, stored: () => stored.promise });
  const run = h.restoreAllSessions(() => current); await flush(); current = false;
  snapshot.resolve(identity); stored.resolve({ kgc: true, luna: false, kwic: false });
  assert.equal(await run, null); assert.deepEqual(h.calls, [['snapshot'], ['stored']]);
  assert.deepEqual(changed(h), []); assert.equal(h.storage.size, 0);
});

test('canceling during secondary validation never starts the subsequent headless sync', async () => {
  const h = await loadSessionRestore(), validators = [deferred(), deferred()]; let current = true;
  h.configure({ validate: key => validators[key === 'luna' ? 0 : 1].promise });
  const run = h.restoreAllSessions(() => current); await flush(); current = false;
  validators.forEach(item => item.resolve(false)); assert.equal(await run, null);
  assert.deepEqual(h.calls, [['snapshot'], ['stored'], ['validate','luna'], ['validate','kwic']]);
  assert.deepEqual(changed(h), []);
});

test('canceling while an issued sync completes discards both recovered/reset notifications and identity writes', async () => {
  const h = await loadSessionRestore(), sync = [deferred(), deferred()]; let current = true;
  h.configure({ validate: () => false, sync: key => sync[key === 'luna' ? 0 : 1].promise });
  const run = h.restoreAllSessions(() => current); await flush();
  assert.equal(h.calls.filter(([name]) => name === 'sync').length, 2); current = false;
  sync[0].resolve(true); sync[1].resolve(false); assert.equal(await run, null);
  assert.deepEqual(changed(h), []); assert.equal(h.calls.some(([name]) => name === 'mailCheck'), false);
});

test('canceling during mail restoration discards late mail state without undoing earlier current results', async () => {
  for (const failure of [false, true]) {
    const h = await loadSessionRestore(), mail = deferred(); let current = true;
    h.configure({ mail: () => mail.promise }); const run = h.restoreAllSessions(() => current); await flush();
    assert.ok(h.state.auth); const before = structuredClone(changed(h)); current = false;
    if (failure) mail.reject(new Error('late mail'));
    else mail.resolve({ authenticated: true, email: 'stale@example.test', display_name: 'stale' });
    assert.equal(await run, null); assert.deepEqual(changed(h), before); assert.equal(h.state.mail, null);
  }
});

test('normal recovery retains complete identity and service semantics across stored-session combinations', async () => {
  for (const valid of [false, true]) for (const savedIdentity of [false, true]) for (const secondary of [false, true]) {
    const h = await loadSessionRestore();
    const status = savedIdentity ? { ...identity, valid } : { valid, username: '', display_name: '', student_id: '' };
    h.configure({ snapshot: () => status, stored: () => ({ kgc: savedIdentity, luna: secondary, kwic: secondary }) });
    const result = await h.restoreAllSessions();
    const ready = secondary || savedIdentity || valid;
    // An identity, validated secondary sessions, or existing KGC validity keeps
    // the original Dashboard behavior; no input fields are removed or capped.
    assert.deepEqual(result, ready ? status : null);
    if (valid || savedIdentity) assert.deepEqual(h.state.auth, status);
    if (!valid && !savedIdentity && secondary) assert.equal(h.state.auth.authenticated, true);
    assert.equal(h.state.mail?.email ?? null, valid ? 'full@example.test' : null);
  }
});

test('secondary validation errors preserve sessions and default callers need no lifetime argument', async () => {
  const h = await loadSessionRestore(); h.configure({ validate: () => Promise.reject(new Error('429')) });
  const restored = await h.restoreAllSessions(); assert.equal(restored.username, 'user');
  assert.deepEqual(h.calls.filter(([name]) => name === 'recovered'), [['recovered','luna'], ['recovered','kwic']]);
  assert.equal(h.calls.some(([name]) => name === 'sync'), false);
});
