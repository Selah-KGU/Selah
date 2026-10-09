import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { resolve } from 'node:path';
let instance = 0;
async function fixture() {
  const source = await readFile('src/lib/api.ts', 'utf8');
  const fn = source.match(/export function setAuthFromSession\([^]*?\n}\n/)[0];
  const output = await build({stdin: {contents: `
    import { universitySessionLifetime } from ${JSON.stringify(resolve('src/lib/sessionLifetime.ts'))};
    export { universitySessionLifetime };
    export const values = new Map(), changes = [];
    const localStorage = { getItem: key => values.get(key) ?? null, setItem: (key,value) => values.set(key,value) };
    const invalidateCache = () => changes.push('cache');
    const aiNotifStore = { set: value => changes.push(['notifications',value]) };
    const aiTodoStore = { set: value => changes.push(['todos',value]) };
    const universityLoginPersistencePending = { set: value => changes.push(['pending',value]) };
    const authState = { set: value => changes.push(['auth',value.username]) };
    const EVER_AUTH_KEY = 'ever', EVER_AUTH_SOURCE_KEY = 'source';
    ${fn}
  `,loader:'ts',resolveDir:process.cwd()},bundle:true,write:false,platform:'node',format:'esm'});
  return import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString('base64')}#owner-${++instance}`);
}
test('unowned and different-account caches are cleared before publishing identity', async () => {
  const h = await fixture();
  h.setAuthFromSession({username:'alice',generation:1});
  assert.deepEqual(h.changes, ['cache',['notifications',null],['todos',null],['auth','alice']]);
  h.changes.length=0;
  h.setAuthFromSession({username:'alice',generation:1});
  assert.deepEqual(h.changes,[['auth','alice']]);
  h.changes.length=0;
  h.setAuthFromSession({username:'bob',generation:2});
  assert.deepEqual(h.changes,['cache',['notifications',null],['todos',null],['auth','bob']]);
  assert.equal(h.values.get('selah-cache-owner'),'bob');
});

test('a verified login remains usable while persistence is pending, and stale login events cannot change it', async () => {
  const h = await fixture();
  h.setAuthFromSession({username:'alice',generation:2,persistence_pending:true});
  assert.ok(h.changes.some(change => Array.isArray(change) && change[0]==='auth' && change[1]==='alice'));
  assert.deepEqual(h.changes.filter(change => Array.isArray(change) && change[0]==='pending'), [['pending',true]]);
  h.changes.length=0;
  h.setAuthFromSession({username:'old',generation:1,persistence_pending:false});
  assert.deepEqual(h.changes,[]);
});
test('a retired login cannot change the cache owner or clear current data', async () => {
  const h = await fixture();
  h.setAuthFromSession({username:'bob',generation:2});
  h.changes.length=0;
  h.setAuthFromSession({username:'alice',generation:1});
  assert.deepEqual(h.changes,[]);
  assert.equal(h.values.get('selah-cache-owner'),'bob');
});
