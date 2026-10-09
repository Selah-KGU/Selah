import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { loadLiveClearOwner } from './load-live-clear-owner.mjs';

const deferred = () => {let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const preview = name => ({active:false,session_id:null,course:{course_name:name},transcript_line_count:1,visible_lines:[{text:`${name} transcript`}],summaries:[]});
const active = id => ({...preview('recording'),active:true,session_id:id});
const count = (h,kind) => h.calls.filter(c=>c[0]===kind).length;

test('current clear releases busy and clears only its displayed course after the native acknowledgment', async () => {
  const h=await loadLiveClearOwner(), pending=deferred(), initial=preview('Course A');
  h.setSnapshot(initial);h.configure({clear:()=>pending.promise});
  const operation=h.executeClearCourseData();
  assert.equal(h.state.value.snapshot,initial);assert.equal(h.state.value.busy,true);
  assert.deepEqual(h.calls[0],['clear',{course_name:'Course A',course_code:undefined,teacher:undefined,
    day:4,period:1,room:'A101',time_label:'9:00-10:30',is_free_note:false}]);
  pending.resolve();await operation;
  assert.equal(h.state.value.busy,false);assert.equal(h.state.value.snapshot.transcript_line_count,0);
  assert.equal(h.state.value.overallSummary,'');
  assert.deepEqual(h.state.value.notice,{kind:'success',text:'Course A のキャッシュをクリアしました'});
});

test('a preview captured before clearing cannot restore the deleted course after acknowledgment', async () => {
  const h=await loadLiveClearOwner(), oldRead=deferred(), clear=deferred();
  h.configure({peek:()=>oldRead.promise,clear:()=>clear.promise});
  const reading=h.peek(), operation=h.executeClearCourseData();
  clear.resolve();await operation;oldRead.resolve(preview('removed Course A'));
  assert.equal(await reading,false);assert.equal(h.state.value.snapshot.transcript_line_count,0);
  assert.equal(count(h,'clear'),1);
});

test('course selection and free-note changes discard a retired clear success and failure', async () => {
  for (const selection of [1,null]) for (const failure of [false,true]) {
    const h=await loadLiveClearOwner(), pending=deferred(), initial=preview('Course A');
    h.setSnapshot(initial);h.configure({clear:()=>pending.promise});
    const operation=h.executeClearCourseData();h.select(selection);
    if(failure) pending.reject(new Error('old clear failure'));else pending.resolve();
    await operation;
    assert.equal(h.state.value.snapshot,initial);assert.equal(h.state.value.notice,null);
    assert.equal(h.state.value.overallSummary,'existing summary');assert.equal(h.state.value.busy,false);
  }
});

test('new active and completed recordings cannot be erased by an old clear response or error', async () => {
  for (const recording of [active('new'),{...active('completed'),active:false}]) for (const failure of [false,true]) {
    const h=await loadLiveClearOwner(), pending=deferred();
    h.configure({clear:()=>pending.promise});const operation=h.executeClearCourseData();
    h.setSnapshot(recording,true);
    if(failure) pending.reject(new Error('old error'));else pending.resolve();
    await operation;assert.equal(h.state.value.snapshot,recording);assert.equal(h.state.value.notice,null);
  }
});

test('passive replacement and pushed session epochs retire the displayed-state owner even while inactive', async () => {
  for (const change of ['replace','epoch','saved']) {
    const h=await loadLiveClearOwner(), pending=deferred(), initial=preview('Course A');
    h.setSnapshot(initial);h.configure({clear:()=>pending.promise});const operation=h.executeClearCourseData();
    if(change==='replace') h.setSnapshot(preview('fresh passive read'));
    if(change==='epoch') h.setSnapshot(initial,true);
    if(change==='saved') h.setSaved(true);
    const current=h.state.value.snapshot;pending.resolve();await operation;
    assert.equal(h.state.value.snapshot,current);assert.equal(h.state.value.notice,null);
  }
});

test('same course slot replacement keeps the owner while course renaming and midnight retire it', async () => {
  for (const change of ['same','rename','midnight']) {
    const h=await loadLiveClearOwner(), pending=deferred();
    h.configure({clear:()=>pending.promise});const operation=h.executeClearCourseData();
    if(change==='midnight') h.setTime(new Date(2026,9,9,0));
    else h.replaceCourses([{name:change==='rename'?'Renamed course':'Course A',day:4,period:1,room:'new room'}]);
    pending.resolve();await operation;
    assert.equal(h.state.value.notice?.kind,change==='same'?'success':undefined);
  }
});

test('destroyed pages neither issue new clear requests nor publish late clear results', async () => {
  for (const failure of [false,true]) {
    const h=await loadLiveClearOwner(), pending=deferred(), initial=preview('Course A');
    h.setSnapshot(initial);h.configure({clear:()=>pending.promise});const operation=h.executeClearCourseData();h.dispose();
    if(failure) pending.reject(new Error('late failure'));else pending.resolve();
    await operation;await h.executeClearCourseData();
    assert.equal(h.state.value.snapshot,initial);assert.equal(h.state.value.notice,null);assert.equal(count(h,'clear'),1);
  }
});

test('one hundred repeated clears and recording, finish, free-note gates issue at most one deletion', async () => {
  const h=await loadLiveClearOwner(), pending=deferred();h.configure({clear:()=>pending.promise});
  const operations=Array.from({length:100},()=>h.executeClearCourseData());
  assert.equal(count(h,'clear'),1);pending.resolve();await Promise.all(operations);
  for(const snapshot of [active('a'),{...active('a'),finish_phase:'saving_record'}]) {
    h.setSnapshot(snapshot);await h.executeClearCourseData();assert.equal(count(h,'clear'),1);
  }
  h.setSnapshot(preview('Course A'));h.select(null);await h.executeClearCourseData();assert.equal(count(h,'clear'),1);
});

test('a current native failure preserves content and can retry successfully', async () => {
  const h=await loadLiveClearOwner(), initial=preview('Course A');h.setSnapshot(initial);
  h.configure({clear:async()=>{throw new Error('cache is locked');}});await h.executeClearCourseData();
  assert.equal(h.state.value.snapshot,initial);assert.equal(h.state.value.busy,false);
  assert.deepEqual(h.state.value.notice,{kind:'error',text:'cache is locked'});
  h.configure({clear:async()=>{}});await h.executeClearCourseData();
  assert.equal(h.state.value.notice.kind,'success');assert.equal(h.state.value.snapshot.transcript_line_count,0);
});

test('the existing saved badge allows an owned clear while a newer save preview retires its result', async () => {
  for(const newer of [false,true]) {
    const h=await loadLiveClearOwner(), pending=deferred(), initial=preview('Course A');
    h.setSnapshot(initial);h.setSaved(true);h.configure({clear:()=>pending.promise});
    const operation=h.executeClearCourseData();if(newer) h.setSaved(true);
    pending.resolve();await operation;
    assert.equal(h.state.value.showSaveNotif,true);
    assert.equal(h.state.value.notice?.kind,newer?undefined:'success');
    assert.equal(h.state.value.snapshot===initial,newer);
  }
});

test('native clear-cache IPC strings become current errors and its null acknowledgment permits a successful retry', async () => {
  const wire=JSON.parse(await readFile('tests/fixtures/live-clear-cache-wire.json','utf8'));
  const h=await loadLiveClearOwner(), initial=preview('Course A');h.setSnapshot(initial);
  h.configure({clear:async()=>{throw wire.error;}});await h.executeClearCourseData();
  assert.equal(h.state.value.snapshot,initial);assert.equal(h.state.value.busy,false);
  assert.deepEqual(h.state.value.notice,{kind:'error',text:wire.error});
  h.configure({clear:async()=>wire.success});await h.executeClearCourseData();
  assert.equal(h.state.value.notice.kind,'success');assert.equal(h.state.value.snapshot.transcript_line_count,0);
});
