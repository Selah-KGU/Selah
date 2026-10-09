import test from 'node:test';
import assert from 'node:assert/strict';
import {loadLiveStartOwner} from './load-live-start-owner.mjs';

const tick = () => new Promise(resolve => setImmediate(resolve));
const course = {course_name:'講義',day:1,period:2};
const session = (id='A',revision=1,extra={}) => ({active:true,session_id:id,update_revision:revision,course,
  started_at:'2026-10-08 10:00:00',summaries:[],visible_lines:[],transcript_line_count:0,pending_from_line:0,
  finish_phase:null,finish_revision:0,summarizing:false,next_summary_at_ms:null,...extra});
function deferred() {let resolve,reject;const promise = new Promise((done,fail) => {resolve=done;reject=fail;});return {promise,resolve,reject};}
const commands = h => h.calls.filter(([name]) => name === 'invoke');
const cancels = h => h.calls.filter(([name]) => name === 'cancel');
const messages = h => h.calls.filter(([name]) => name === 'message');
function emit(h,name,payload) {for (const listener of h.listeners.filter(listener => listener.name === name)) listener.receive({payload});}

test('100 start clicks share one synchronous busy gate, creation and owner-scoped stream request',async () => {
  const h = await loadLiveStartOwner(), create = deferred();
  h.configure({create:() => create.promise});
  const runs = Array.from({length:100},() => h.startSession(course)); await tick();
  assert.equal(h.calls.filter(([name]) => name === 'create').length,1);
  create.resolve(session()); await Promise.all(runs);
  assert.deepEqual(commands(h),[['invoke','stt_start_stream',{caller:'live',liveSessionId:'A'}]]);
  assert.equal(h.state.value.busy,false); assert.equal(h.state.value.autoFollow,true);
  assert.equal(h.state.value.overallSummary,''); assert.equal(h.state.value.partialText,''); assert.equal(h.state.value.lastSaved,null);
  assert.deepEqual(cancels(h),[]); h.dispose();
});

test('closing while readiness is pending prevents native creation and audio',async () => {
  const h = await loadLiveStartOwner(), ready = deferred();
  h.configure({ready:() => ready.promise,create:() => session()});
  const run = h.startSession(course); await tick();h.dispose();ready.resolve({ready:true});await run;
  assert.equal(h.calls.filter(([name]) => name === 'create').length,0);
  assert.deepEqual(commands(h),[]); assert.deepEqual(cancels(h),[]);
});

test('late creation after view disposal starts no audio, publishes no snapshot and keeps backend data',async () => {
  const h = await loadLiveStartOwner(), create = deferred();
  h.configure({create:() => create.promise});const run = h.startSession(course);await tick();h.dispose();
  const previous = h.state.value; create.resolve(session());await run;
  assert.strictEqual(h.state.value.snapshot,previous.snapshot);
  assert.equal(h.state.value.overallSummary,previous.overallSummary);
  assert.equal(h.state.value.lastSaved,previous.lastSaved);
  assert.deepEqual(commands(h),[]);assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[]);
});

test('new recording event supersedes an old creation without starting audio or cancelling either recording',async () => {
  const h = await loadLiveStartOwner(), create = deferred();
  h.configure({create:() => create.promise});const run = h.startSession(course);await tick();
  h.pushSession(session('B',10));const previous=h.state.value.snapshot;
  create.resolve(session('A',1));await run;
  assert.strictEqual(h.state.value.snapshot,previous);
  assert.deepEqual(commands(h),[]);assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[]);
  assert.equal(h.state.value.lastSaved.summary_markdown,'saved preview');h.dispose();
});

test('a newer finish reservation prevents delayed creation from starting or cancelling audio',async () => {
  const h = await loadLiveStartOwner(), create = deferred();
  h.configure({create:() => create.promise});const run = h.startSession(course);await tick();
  h.pushSession(session('A',10,{finish_phase:'saving_record',finish_revision:2}));
  create.resolve(session('A',1));await run;
  assert.equal(h.state.value.snapshot.finish_phase,'saving_record');
  assert.deepEqual(commands(h),[]);assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[]);h.dispose();
});

test('already-issued audio can finish after view disposal without late cleanup or notices',async () => {
  for (const fail of [false,true]) {
    const h = await loadLiveStartOwner(), stream=deferred();
    h.configure({create:() => session(),invoke:() => stream.promise});
    const run=h.startSession(course);await tick();assert.equal(commands(h).length,1);h.dispose();
    if(fail) stream.reject(new Error('old audio command failed'));else stream.resolve();await run;
    assert.equal(h.state.value.autoFollow,false);assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[]);
  }
});

test('a replacement recording keeps its state when the previous audio start fails',async () => {
  const h=await loadLiveStartOwner(),stream=deferred();
  h.configure({create:() => session(),invoke:() => stream.promise});const run=h.startSession(course);await tick();
  h.pushSession(session('B',10));stream.reject(new Error('A failed'));await run;
  assert.equal(h.state.value.snapshot.session_id,'B');
  assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[]);assert.equal(h.calls.filter(([name]) => name === 'resync').length,0);h.dispose();
});

test('current stream failure preserves the error and cancels and resyncs its own recording once',async () => {
  const h=await loadLiveStartOwner();
  h.configure({create:() => session(),invoke:() => Promise.reject(new Error('device failure'))});
  await h.startSession(course);
  assert.deepEqual(cancels(h),[['cancel','A']]);assert.deepEqual(messages(h),[['message','error','device failure']]);
  assert.equal(h.calls.filter(([name]) => name === 'resync').length,1);
  assert.equal(h.state.value.sttPhase,'idle');assert.equal(h.state.value.busy,false);h.dispose();
});

test('pushed listening state supersedes a delayed command failure without discarding the recording',async () => {
  const h=await loadLiveStartOwner(),stream=deferred();await h.bindLiveSttListeners();
  h.configure({create:() => session(),invoke:() => stream.promise});const run=h.startSession(course);await tick();
  emit(h,'stt-state',{caller:'live',live_session_id:'A',state:'listening'});
  stream.reject(new Error('late transport error'));await run;
  assert.equal(h.state.value.sttListening,true);assert.equal(h.state.value.sttPhase,'listening');
  assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[]);h.dispose();
});

test('STT error for a replacement recording cannot use the older pending-start cleanup marker',async () => {
  const h=await loadLiveStartOwner(),stream=deferred();await h.bindLiveSttListeners();
  h.configure({create:() => session(),invoke:() => stream.promise});const run=h.startSession(course);await tick();
  h.pushSession(session('B',10));emit(h,'stt-error',{caller:'live',live_session_id:'B',message:'B decoder error'});await tick();
  assert.deepEqual(cancels(h),[]);assert.deepEqual(messages(h),[['message','error','B decoder error']]);
  stream.resolve();await run;h.dispose();
});

test('current STT error owns one cancellation even when the command later rejects; disposed listeners stay inert',async () => {
  const h=await loadLiveStartOwner(),stream=deferred();await h.bindLiveSttListeners();
  h.configure({create:() => session(),invoke:() => stream.promise});const run=h.startSession(course);await tick();
  emit(h,'stt-error',{caller:'live',live_session_id:'A',message:'decoder error'});await tick();
  assert.deepEqual(cancels(h),[['cancel','A']]);stream.reject(new Error('command failed too'));await run;
  assert.deepEqual(cancels(h),[['cancel','A']]);assert.deepEqual(messages(h),[['message','error','decoder error']]);
  h.dispose();const count=h.calls.length;emit(h,'stt-error',{caller:'live',live_session_id:'A',message:'after disposal'});
  assert.equal(h.calls.length,count);assert.ok(h.listeners.every(listener => listener.releases === 1));
});

test('disabled readiness and demo retain existing error and listening behavior',async () => {
  const denied=await loadLiveStartOwner();
  denied.configure({ready:() => ({ready:false,message:'AI disabled'}),create:() => session()});await denied.startSession(course);
  assert.deepEqual(messages(denied),[['message','error','AI disabled']]);assert.equal(commands(denied).length,0);denied.dispose();
  const demo=await loadLiveStartOwner();demo.configure({demo:true,create:() => session()});await demo.startSession(course);
  assert.equal(demo.state.value.sttListening,true);assert.equal(demo.state.value.sttPhase,'listening');
  assert.ok(demo.state.value.lastEffectiveSpeechAtMs > 0);assert.equal(commands(demo).length,0);demo.dispose();
});

test('late cancellation completion cannot resync or clear the replacement recording silence deadline',async () => {
  const h=await loadLiveStartOwner(),cancel=deferred();await h.bindLiveSttListeners();
  h.configure({create:() => session(),invoke:() => Promise.reject(new Error('A failed')),cancel:() => cancel.promise});
  const run=h.startSession(course);await tick();assert.deepEqual(cancels(h),[['cancel','A']]);
  h.pushSession(session('B',10));emit(h,'stt-state',{caller:'live',live_session_id:'B',state:'listening'});
  const speechAt = h.state.value.lastEffectiveSpeechAtMs;assert.ok(speechAt > 0);
  cancel.resolve();await run;
  assert.equal(h.state.value.lastEffectiveSpeechAtMs,speechAt);assert.equal(h.state.value.sttListening,true);
  assert.equal(h.calls.filter(([name]) => name === 'resync').length,0);h.dispose();
});

test('a delayed STT-event cancellation also skips recovery after a replacement recording takes over',async () => {
  const h=await loadLiveStartOwner(),cancel=deferred(),stream=deferred();await h.bindLiveSttListeners();
  h.configure({create:() => session(),invoke:() => stream.promise,cancel:() => cancel.promise});
  const run=h.startSession(course);await tick();
  emit(h,'stt-error',{caller:'live',live_session_id:'A',message:'A decoder failed'});await tick();
  assert.deepEqual(cancels(h),[['cancel','A']]);
  h.pushSession(session('B',10));emit(h,'stt-state',{caller:'live',live_session_id:'B',state:'listening'});
  emit(h,'stt-partial',{caller:'live',live_session_id:'B',text:'B partial',seq:10});
  const speechAt=h.state.value.lastEffectiveSpeechAtMs;
  cancel.resolve();stream.reject(new Error('old command failed'));await run;await tick();
  assert.equal(h.calls.filter(([name]) => name === 'resync').length,0);
  assert.equal(h.state.value.partialText,'B partial');assert.equal(h.state.value.lastEffectiveSpeechAtMs,speechAt);h.dispose();
});
