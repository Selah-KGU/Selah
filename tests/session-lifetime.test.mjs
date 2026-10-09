import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

let instance = 0;
async function load() {
  const source = await readFile('src/lib/api.ts', 'utf8');
  const guard = source.match(/async function withSessionGuard[^]*?\n}\n/)?.[0];
  const sync = source.match(/export async function syncSession[^]*?\n}\n/)?.[0];
  assert.ok(guard && sync);
  const result = await build({stdin: {resolveDir: process.cwd(), loader: 'ts', contents: `
    import { SessionLifetime, universitySessionLifetime } from ${JSON.stringify(resolve('src/lib/sessionLifetime.ts'))};
    export { SessionLifetime, universitySessionLifetime };
    export const events = [];
    let mailRequestRevision = 0;
    export const retireMail = () => { mailRequestRevision++; };
    const applyMailStatus = status => events.push(["mail", status]);
    const result = (service, recovered = true) => ({results:[{service,outcome:recovered?'verified':'unavailable',recovered}],snapshot:{generation:0,revision:1}});
    let run = async service => result(service);
    export function configure(next) { run = next; }
    function invoke(name, args) { events.push(['invoke', args.service]); return run(args.service); }
    function applyRecoveryReport(report) { events.push(['projection',report]); }
    const mailCheckSession = async () => ({authenticated:false,email:'',display_name:''});
    const mailAuthState = {set:value=>events.push(['mail',value])};
    ${sync}
    ${guard}
    export { withSessionGuard };
  `}, bundle:true, write:false, platform:'node', format:'esm', define:{'console.warn':'ignore'}, banner:{js:'function ignore() {}'}});
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#lifetime-${++instance}`);
}
function deferred() { let resolve, reject; const promise = new Promise((a,b)=>{resolve=a;reject=b;}); return {promise,resolve,reject}; }
async function flush() { for(let i=0;i<30;i++) await Promise.resolve(); }

test('overlapping callers share a service and distinct service flows serialize', async () => {
  const h = await load(), first = deferred();
  h.configure(s => s === 'luna' ? first.promise : outcome('kwic'));
  const requests = Array.from({length: 100}, () => h.syncSession('luna'));
  const other = h.syncSession('kwic'); await flush();
  assert.deepEqual(h.events, [['invoke','luna']]);
  first.resolve(outcome());
  assert.ok((await Promise.all(requests)).every(value => value.results[0].recovered)); assert.equal((await other).results[0].service, 'kwic');
  assert.deepEqual(h.events.filter(([name])=>name==='invoke'), [['invoke','luna'],['invoke','kwic']]);
});

test('new native generation cancels old work and queues; stale cleanup cannot evict the new owner', async () => {
  const h = await load(), old = deferred(), fresh = deferred();
  h.configure(() => old.promise);
  const first = h.syncSession('luna').catch(e => e.message);
  const queued = h.syncSession('kwic').catch(e => e.message); await flush();
  h.universitySessionLifetime.accept(1); h.configure(() => fresh.promise);
  const next = h.syncSession('luna'); await flush();
  old.resolve(outcome()); assert.match(await first, /cancelled/); assert.match(await queued, /cancelled/);
  const joined = h.syncSession('luna'); await flush();
  assert.deepEqual(h.events, [['invoke','luna'],['invoke','luna']]);
  fresh.resolve(outcome('luna',true,1)); assert.equal((await next).results[0].recovered, true); assert.equal((await joined).results[0].recovered,true);
  assert.equal(h.universitySessionLifetime.accept(0), false);
});

test('late API success or expiry after logout never reaches callers or launches recovery', async () => {
  for (const failure of [false,true]) {
    const h = await load(), response = deferred();
    const pending = h.withSessionGuard("luna", () => response.promise).catch(e => e.message);
    h.universitySessionLifetime.retire();
    if(failure) response.reject('luna-expired'); else response.resolve('old-account-data');
    assert.match(await pending, /cancelled/); assert.deepEqual(h.events, []);
  }
});

test('late recovery success and failure cannot recover/reset a newer account or retry the old API', async () => {
  for (const result of [false,true]) {
    const h = await load(), recovery = deferred(); let requests = 0;
    h.configure(() => recovery.promise);
    const pending = h.withSessionGuard("luna", async () => { requests++; throw 'luna-expired'; }).catch(e => e.message);
    await flush(); h.universitySessionLifetime.accept(2); recovery.resolve(outcome('luna',result));
    assert.match(await pending, /cancelled/); assert.equal(requests, 1);
    assert.deepEqual(h.events, [['invoke','luna']]);
  }
});

test('same-generation confirmed expiry still recovers and retries once', async () => {
  const h = await load(); let requests = 0;
  const value = await h.withSessionGuard("luna", async () => { if(++requests === 1) throw 'luna-expired'; return 'fresh-data'; });
  assert.equal(value,'fresh-data'); assert.equal(requests,2);
  assert.equal(h.events[0][0],'invoke'); assert.equal(h.events[0][1],'luna'); assert.equal(h.events[1][0],'projection');
});

function outcome(service='luna',recovered=true,generation=0) { return {results:[{service,outcome:recovered?'verified':'unavailable',recovered}],snapshot:{generation,revision:1}}; }

test('unavailable or deferred native results never retry or infer expiry from display text', async()=>{
  for(const outcomeName of ['unavailable','deferred']) {
    const h=await load(); let calls=0;
    h.configure(()=>({results:[{service:'luna',outcome:outcomeName,recovered:false}]}));
    await assert.rejects(h.withSessionGuard('luna',async()=>{calls++;throw new Error('Lunaセッションが期限切れです');}), /Luna/);
    assert.equal(calls,1); assert.equal(h.events.filter(([name])=>name==='projection').length,1);
  }
});
test('mutations are never replayed after a successful native recovery', async()=>{
  const h=await load(); let calls=0;
  await assert.rejects(h.withSessionGuard('luna',async()=>{calls++;throw new Error('response lost');},false),/response lost/);
  assert.equal(calls,1);
});
test('older revisions in the same account cannot overwrite newer status', async()=>{
  const {SessionLifetime}=await load(), owner=new SessionLifetime();
  assert.equal(owner.acceptSnapshot(2,10),true);
  assert.equal(owner.acceptSnapshot(2,9),false);
  assert.equal(owner.acceptSnapshot(1,100),false);
  assert.equal(owner.acceptSnapshot(3,1),true);
});

 test('mail responses from a disconnected connection never reach callers or refresh a newer one', async () => {
  for (const failure of [false, true]) {
    const h = await load(), response = deferred();
    const pending = h.withSessionGuard('mail', () => response.promise).catch(e => e.message);
    h.retireMail();
    if (failure) response.reject('old error'); else response.resolve('old mailbox');
    assert.match(await pending, /Mail connection changed/);
    assert.deepEqual(h.events, []);
  }
});
