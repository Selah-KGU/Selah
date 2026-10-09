import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSessionRestore, report, identity } from './load-session-restore.mjs';
function deferred() { let resolve,reject; const promise=new Promise((a,b)=>{resolve=a;reject=b;}); return {promise,resolve,reject}; }
async function flush() { for(let i=0;i<30;i++) await Promise.resolve(); }
const mutations = h => h.calls.filter(([name])=>['auth','expired','mail','luna','kwic'].includes(name));

test('obsolete restoration performs no IO, including demo activation', async()=>{
  for(const demo of [false,true]) { const h=await loadSessionRestore(); h.configure({demo}); assert.equal(await h.restoreAllSessions(()=>false),null); assert.deepEqual(h.calls,[]); }
});
test('startup submits one native recovery intent and projects full identity and proof', async()=>{
  const h=await loadSessionRestore(); h.configure({restore:()=>report()});
  assert.deepEqual(await h.restoreAllSessions(), {valid:true,...identity});
  assert.deepEqual(h.calls.filter(([name])=>name==='invoke'),[['invoke','restore_university_sessions']]);
  assert.deepEqual(h.state.luna,{authenticated:true}); assert.equal(h.state.expired,false);
  assert.equal(h.state.mail.email,'full@example.test');
});

test('persistence completion clears the warning without losing authenticated identity', async()=>{
  const h=await loadSessionRestore();
  let current=report(); current.snapshot.login_persistence_pending=true;
  h.configure({restore:()=>current});
  await h.restoreAllSessions();
  assert.equal(h.state.persistencePending,true);
  assert.equal(h.state.auth.username,identity.username);
  current=report(); current.snapshot.revision=1; current.snapshot.login_persistence_pending=false;
  await h.restoreAllSessions();
  assert.equal(h.state.persistencePending,false);
  assert.equal(h.state.auth.username,identity.username);
});
test('owner cancellation while native restoration completes discards every UI mutation', async()=>{
  const h=await loadSessionRestore(), native=deferred(); let current=true;
  h.configure({restore:()=>native.promise}); const pending=h.restoreAllSessions(()=>current); await flush(); current=false; native.resolve(report());
  assert.equal(await pending,null); assert.deepEqual(mutations(h),[]); assert.equal(h.calls.some(([name])=>name==='mailCheck'),false);
});
test('a native logout retires pending startup restoration without an owner callback', async()=>{
  const h=await loadSessionRestore(), native=deferred(); h.configure({restore:()=>native.promise});
  const pending=h.restoreAllSessions(); await flush(); h.universitySessionLifetime.accept(1); native.resolve(report());
  assert.equal(await pending,null); assert.deepEqual(mutations(h),[]);
});
test('mail completion and failures after cancellation cannot change the new owner', async()=>{
  for(const failed of [false,true]) { const h=await loadSessionRestore(), mail=deferred(); let current=true;
    h.configure({restore:()=>report(),mail:()=>mail.promise}); const pending=h.restoreAllSessions(()=>current); await flush();
    const before=structuredClone(mutations(h)); current=false;
    if(failed) mail.reject('offline'); else mail.resolve({authenticated:true,email:'stale',display_name:'stale'});
    assert.equal(await pending,null); assert.deepEqual(mutations(h),before);
  }
});
test('unverified disk credentials allow the offline shell without inventing login proof', async()=>{
  const h=await loadSessionRestore(); h.configure({restore:()=>report({health:'unverified',proof:false})});
  assert.deepEqual(await h.restoreAllSessions(),{valid:false,...identity});
  assert.deepEqual(h.state.luna,{authenticated:false}); assert.deepEqual(h.state.kwic,{authenticated:false}); assert.equal(h.state.expired,false);
});
test('unavailable preserves previous proof while confirmed expiry exposes reauthentication', async()=>{
  for(const health of ['unavailable','needs_login']) { const h=await loadSessionRestore();
    h.configure({restore:()=>report({health,present:health==='unavailable',proof:health==='unavailable'})}); await h.restoreAllSessions();
    assert.equal(h.state.luna.authenticated,health==='unavailable'); assert.equal(h.state.expired,health==='needs_login');
  }
});
test('core-only recovery works without KGC identity, but SSO evidence alone is not authentication', async()=>{
  for(const present of [false,true]) { const h=await loadSessionRestore(); h.configure({restore:()=>report({who:null,present,proof:present,health:present?'valid':'needs_login'})});
    assert.equal((await h.restoreAllSessions())!==null,present);
    if(present) assert.equal(h.state.auth.display_name,'ユーザー'); else assert.equal(h.state.auth,null);
  }
});
test('fresh WebView adopts the current native generation without canceling its own startup', async()=>{
  const h=await loadSessionRestore(); h.configure({restore:()=>report({generation:7})});
  assert.deepEqual(await h.restoreAllSessions(),{valid:true,...identity}); assert.equal(h.state.mail.email,'full@example.test');
});
test('signed-out snapshots never restore identity or service state', async()=>{
  const h=await loadSessionRestore(); h.configure({restore:()=>report({signedOut:true})});
  assert.equal(await h.restoreAllSessions(),null); assert.deepEqual(mutations(h),[]);
});
