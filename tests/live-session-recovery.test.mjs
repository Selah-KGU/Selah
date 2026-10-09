import test from 'node:test';
import assert from 'node:assert/strict';
import {loadLiveSessionRecovery} from './load-live-session-recovery.mjs';
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((done, failed) => {resolve = done; reject = failed;});
  return {promise,resolve,reject};
};
const recording = (id, revision, count = 1) => ({active:true,session_id:id,update_revision:revision,
  course:null,started_at:'10:00',transcript_line_count:count,
  visible_lines:Array.from({length:count}, (_,i) => ({at:'10:00',text:`complete line ${i} 🌙`})),
  pending_from_line:0,summaries:[],finish_revision:0,finish_phase:null});

test('ten thousand recovery notifications share each read and keep a separate following read', async () => {
  const live = await loadLiveSessionRecovery();
  const gates = [deferred(),deferred()], started = [deferred(),deferred()];
  let reads = 0;
  live.configure(() => {const i = reads++; started[i].resolve(); return gates[i].promise;});
  const first = Array.from({length:10000}, () => live.resyncSession());
  await started[0].promise;
  const next = Array.from({length:10000}, () => live.resyncSession());
  gates[0].resolve(recording('input',1));
  await started[1].promise;
  await first[0];
  assert.equal(live.state().transcript_line_count,1);
  let nextDone = false;
  next[0].then(() => {nextDone = true;});
  await Promise.resolve();
  assert.equal(nextDone,false);
  gates[1].resolve(recording('input',2,2));
  await Promise.all([...first,...next]);
  assert.equal(reads,2);
  assert.equal(live.state().transcript_line_count,2);
  assert.equal(new Set(first).size,1);
  assert.equal(new Set(next).size,1);
  assert.notEqual(first[0],next[0]);
});

test('a failed read logs once and still runs the queued batch, with fulfilled public completions', async () => {
  const live = await loadLiveSessionRecovery(), gate = deferred(), started = deferred();
  const failure = new Error('IPC unavailable');
  let reads = 0;
  live.configure(() => {if (++reads === 1) {started.resolve(); return gate.promise;} return recording('input',2,2);});
  const first = [live.resyncSession(),live.resyncSession()];
  await started.promise;
  const next = [live.resyncSession(),live.resyncSession()];
  gate.reject(failure);
  const results = await Promise.allSettled([...first,...next]);
  assert.ok(results.every(result => result.status === 'fulfilled'));
  assert.equal(reads,2);
  assert.equal(live.warnings.length,1);
  assert.equal(live.warnings[0][1],failure);
  assert.equal(live.state().session_id,'input');
  assert.equal(live.state().transcript_line_count,2);
});

test('a pushed replacement stays current when the old read returns, then the next read fills its gap', async () => {
  const live = await loadLiveSessionRecovery(), gate = deferred(), started = deferred();
  let reads = 0;
  live.configure(() => {if (++reads === 1) {started.resolve(); return gate.promise;} return recording('new',4,3);});
  live.push(recording('old',1));
  const first = live.resyncSession();
  await started.promise;
  live.push(recording('new',3,2));
  const next = live.resyncSession();
  gate.resolve(recording('old',2,20));
  await first;
  assert.equal(live.state().session_id,'new');
  await next;
  assert.equal(reads,2);
  assert.equal(live.state().update_revision,4);
  assert.equal(live.state().transcript_line_count,3);
});

test('disposal ignores the late read and skips pending recovery IO and warnings', async () => {
  for (const fail of [false,true]) {
    const live = await loadLiveSessionRecovery(), gate = deferred(), started = deferred();
    let reads = 0;
    live.configure(() => {reads++; started.resolve(); return gate.promise;});
    live.push(recording('current',1));
    const before = live.state(), first = live.resyncSession();
    await started.promise;
    const next = live.resyncSession();
    live.dispose();
    if (fail) gate.reject('late failure'); else gate.resolve(recording('old',2,20));
    await Promise.all([first,next,live.resyncSession()]);
    assert.equal(reads,1);
    assert.equal(live.state(),before);
    assert.equal(live.warnings.length,0);
  }
});

test('an explicit recovery after failure reads again instead of retaining the failed completion', async () => {
  const live = await loadLiveSessionRecovery();
  let reads = 0;
  live.configure(() => {if (++reads === 1) throw 'sync native bridge failure'; return recording('recovered',3);});
  const failed = live.resyncSession();
  await failed;
  const retry = live.resyncSession();
  await retry;
  assert.notEqual(failed,retry);
  assert.equal(reads,2);
  assert.equal(live.state().session_id,'recovered');
  assert.equal(live.warnings.length,1);
});
