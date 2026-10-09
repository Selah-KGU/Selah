import test from 'node:test';
import assert from 'node:assert/strict';
import { loadBackgroundRefresh } from './load-background-refresh.mjs';
import { loadVisibleLogin } from './load-visible-login.mjs';
import { loadLogin } from './load-login.mjs';

const tick = () => new Promise(resolve => setImmediate(resolve));
const aiStatus = (running = false, last_run = 42) => ({ running, last_run, last_ok: true, interval_minutes: 5 });
const sessionStatus = (name = 'current') => ({ generation: 0, signed_out: false, university: { generation: 0, revision: 1, signed_out: false, services: [
  { service: 'luna', state: 'valid', credentials_present: true, last_verified_at: 1 },
  { service: 'kwic', state: 'needs_login', credentials_present: false, last_verified_at: null },
]}, luna_authenticated: true, kwic_authenticated: false, session_expired: true,
  mail_authenticated: true, mail_email: 'example@test.invalid', mail_display_name: name, kgc_session_present: true,
  username: name, display_name: name, student_id: '1', faculty: 'F', department: 'D' });
function complete(request, ts = 42) {
  if (request.name === 'get_schedule_snapshot') request.resolve({ snapshot_updated_at: ts });
  else if (request.name === 'get_data_cache_updated_at') request.resolve(ts);
  else if (request.name === 'get_backend_task_timestamps') request.resolve({
    rows: request.args.keys.map(key => ({key,updated_at:ts})), schedule_updated_at:ts,
  });
  else if (request.name === 'get_backend_ai_refresh_status') request.resolve(aiStatus(false, ts));
  else if (request.name === 'backend_sync_session_status_now') request.resolve(sessionStatus());
  else throw new Error(`Unexpected command ${request.name}`);
}
const requestCount = (h, name) => h.requests.filter(request => request.name === name).length;

test('100 ordinary starts register once and hydrate 14 task stamps, AI and caches once', async () => {
  const h = await loadBackgroundRefresh();
  for (let i = 0; i < 100; i++) h.startBackgroundPolling();
  await tick();
  assert.equal(h.document.listeners.size, 1);
  assert.equal(h.calls.filter(call => call[0] === 'add').length, 1);
  assert.equal(h.calls.filter(call => call[0] === 'register').length, 16);
  assert.equal(h.calls.filter(call => call[0] === 'cache').length, 1);
  assert.equal(h.requests.length, 2);
  assert.equal(requestCount(h, 'get_data_cache_updated_at'), 0);
  assert.equal(requestCount(h, 'get_schedule_snapshot'), 0);
  assert.equal(requestCount(h, 'get_backend_task_timestamps'), 1);
  h.requests.forEach(request => complete(request));
  await tick();
  assert.equal(h.state.updates.length, 15);
  assert.equal(h.state.tasks.get('exams').lastRunTs, 42000);
  assert.equal(h.state.tasks.get('ai_scheduler').intervalMs, 300000);
  assert.equal(h.state.tasks.get('notifications').intervalMs, 43200000);
  h.stopBackgroundPolling();
});

test('public concurrent status reads coalesce and settled reads can retry', async () => {
  const h = await loadBackgroundRefresh();
  const tasks = Array.from({length:100}, () => h.refreshBackendTaskStatuses());
  const ai = Array.from({length:100}, () => h.refreshBackendAiTaskStatus());
  assert.ok(ai.every(request => request === ai[0]));
  await tick();
  assert.equal(h.requests.length, 2);
  h.requests.forEach(request => complete(request));
  await Promise.all([...tasks,...ai]);
  assert.equal(h.state.updates.length, 15);
  const retry = h.refreshBackendAiTaskStatus();
  await tick();
  h.requests.at(-1).reject(new Error('current read failed'));
  await assert.rejects(retry, /current read failed/);
  const next = h.refreshBackendAiTaskStatus();
  await tick();
  h.requests.at(-1).resolve(aiStatus(true));
  await next;
  assert.deepEqual(h.state.ai,{notif:true,todo:true});
});

test('stop before deferred reads start prevents native IO; queued old visibility is inert', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling();
  const queued = h.document.registrations[0];
  h.stopBackgroundPolling();
  queued();
  await tick();
  assert.equal(h.requests.length, 0);
  assert.equal(h.document.listeners.size, 0);
  assert.equal(h.calls.filter(call => call[0] === 'cache').length, 1);
});

test('stop retires late task, AI and session replies and obsolete failures', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling();
  h.document.emit();
  await tick();
  assert.ok(h.requests.some(request => request.name === 'backend_sync_session_status_now'));
  h.stopBackgroundPolling();
  h.requests.forEach(request => request.name === 'get_backend_ai_refresh_status'
    ? request.reject(new Error('obsolete AI failure')) : complete(request));
  await tick();
  assert.deepEqual(h.state.updates,[]);
  assert.deepEqual(h.state.session,{});
  assert.equal(h.state.identity,null);
  assert.equal(h.state.ai,null);
  assert.deepEqual(h.warnings,[]);
});

test('stop/start gives a new owner independent of unresolved old IO and callbacks', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); h.document.emit(); await tick();
  const old = [...h.requests], queued = h.document.registrations[0];
  h.stopBackgroundPolling();
  h.startBackgroundPolling(); h.document.emit(); await tick();
  const current = h.requests.slice(old.length);
  assert.equal(current.length,3);
  queued();
  assert.equal(h.calls.filter(call => call[0] === 'cache').length,4);
  old.forEach(request => request.reject(new Error('old owner failed')));
  await tick();
  assert.deepEqual(h.warnings,[]);
  assert.deepEqual(h.state.updates,[]);
  // Finishing old requests cannot release the pending slot of the new owner.
  h.refreshBackendAiTaskStatus(); await tick();
  assert.equal(h.requests.length,6);
  current.forEach(request => complete(request,100)); await tick();
  assert.equal(h.state.updates.length,15);
  assert.equal(h.state.tasks.get('schedule_data').lastRunTs,100000);
  assert.equal(h.state.identity.username,'current');
  h.stopBackgroundPolling();
});

test('pushed cache update retires only that task read and preserves its newer timestamp', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  h.callbacks.get('backend-cache-updated')({payload:{keys:['luna_todo','luna_todo','']}});
  const pushed = h.state.updates.find(([key]) => key === 'luna_todo')[1];
  h.requests.forEach(request => complete(request)); await tick();
  assert.equal(h.state.updates.filter(([key]) => key === 'luna_todo').length,1);
  assert.equal(h.state.tasks.get('luna_todo').lastRunTs,pushed.lastRunTs);
  assert.equal(h.state.tasks.get('exams').lastRunTs,42000);
  const next = h.refreshBackendTaskStatuses(); await tick();
  const reads = h.requests.slice(2);
  assert.equal(reads.length,1);
  reads.forEach(request => complete(request,101)); await next;
  assert.equal(h.state.tasks.get('luna_todo').lastRunTs,101000);
  h.stopBackgroundPolling();
});

test('AI push supersedes an older read and next read starts without waiting for it', async () => {
  const h = await loadBackgroundRefresh();
  const older = h.refreshBackendAiTaskStatus(); await tick();
  h.callbacks.get('backend-ai-refresh-status')({payload:aiStatus(true,100)});
  const newer = h.refreshBackendAiTaskStatus(); await tick();
  assert.equal(h.requests.length,2);
  h.requests[0].resolve(aiStatus(false,1)); await older;
  assert.deepEqual(h.state.ai,{notif:true,todo:true});
  h.requests[1].resolve(aiStatus(false,101)); await newer;
  assert.deepEqual(h.state.ai,{notif:false,todo:false});
});

test('session push wins over a pending foreground snapshot, including full identity', async () => {
  const h = await loadBackgroundRefresh();
  const old = h.syncBackendSessionStatusNow(); await tick();
  h.callbacks.get('backend-session-status')({payload:sessionStatus('pushed')});
  h.requests[0].resolve(sessionStatus('obsolete')); await old;
  assert.deepEqual(h.state.identity,{username:'pushed',display_name:'pushed',student_id:'1',faculty:'F',department:'D'});
  assert.deepEqual(h.state.session.mail,{authenticated:true,email:'example@test.invalid',displayName:'pushed',connectionId:null});
  assert.deepEqual(h.state.session.kwic,{authenticated:false});
  assert.equal(h.state.session.expired,true);
});

test('visibility bursts share AI, respect session cooldown and ignore hidden windows', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  h.document.visibilityState = 'hidden'; h.document.emit();
  assert.equal(h.calls.filter(call => call[0] === 'cache').length,1);
  h.document.visibilityState = 'visible';
  for(let i=0;i<100;i++) h.document.emit();
  await tick();
  assert.equal(requestCount(h,'get_backend_ai_refresh_status'),1);
  assert.equal(requestCount(h,'backend_sync_session_status_now'),1);
  h.requests.forEach(request => complete(request)); await tick();
  h.document.emit(); await tick();
  assert.equal(requestCount(h,'get_backend_ai_refresh_status'),2);
  assert.equal(requestCount(h,'backend_sync_session_status_now'),1);
  h.requests.at(-1).resolve(aiStatus()); await tick();
  h.stopBackgroundPolling();
});

test('explicit successful login refreshes an active owner while ordinary starts stay idempotent', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); h.document.emit(); await tick();
  const old = [...h.requests];
  h.startBackgroundPolling(true); h.document.emit(); await tick();
  assert.equal(h.document.registrations.length,1);
  assert.equal(h.requests.length,6);
  old.forEach(request => complete(request,1)); await tick();
  assert.deepEqual(h.state.updates,[]);
  h.requests.slice(3).forEach(request => complete(request,200)); await tick();
  assert.equal(h.state.updates.length,15);
  assert.equal(h.state.tasks.get('schedule_data').lastRunTs,200000);
  h.startBackgroundPolling(); await tick();
  assert.equal(h.requests.length,6);
  h.stopBackgroundPolling();
});

test('demo entry detaches a real owner and performs no native status reads', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  h.configure({demo:true}); h.startBackgroundPolling();
  assert.equal(h.document.listeners.size,0);
  h.requests.forEach(request => complete(request)); await tick();
  assert.deepEqual(h.state.updates,[]);
  const previous = h.requests.length;
  await h.refreshBackendAiTaskStatus();
  await h.syncBackendSessionStatusNow();
  h.document.registrations[0]();
  assert.equal(h.requests.length,previous);
  assert.deepEqual(h.state.ai,{notif:false,todo:false});
});

test('missing and failed task stamps keep null/error fallback and exams DB alias', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  assert.ok(h.requests.some(request => request.args?.keys?.includes('exam_timetable')));
  h.requests.forEach(request => {
    if (request.name === 'get_backend_task_timestamps') request.resolve({
      rows:request.args.keys.map(key => ({key,updated_at:key === 'grades' ? null : key === 'weather' ? 0 : 42})),
      schedule_updated_at:0,
    });
    else complete(request);
  }); await tick();
  for (const key of ['grades','weather','schedule_data']) {
    assert.equal(h.state.tasks.get(key).lastRunTs,null);
    assert.equal(h.state.tasks.get(key).lastOk,null);
  }
  assert.deepEqual(h.warnings,[]);
  h.stopBackgroundPolling();
});

test('session cooldown includes its exact 60-second boundary and resets with a new lifetime', async () => {
  const h = await loadBackgroundRefresh();
  h.configure({now:0}); h.startBackgroundPolling(); h.document.emit(); await tick();
  assert.equal(requestCount(h,'backend_sync_session_status_now'),1);
  h.requests.forEach(request => complete(request)); await tick();
  h.configure({now:59999}); h.document.emit(); await tick();
  assert.equal(requestCount(h,'backend_sync_session_status_now'),1);
  h.requests.at(-1).resolve(aiStatus()); await tick();
  h.configure({now:60000}); h.document.emit(); await tick();
  assert.equal(requestCount(h,'backend_sync_session_status_now'),2);
  h.requests.slice(3).forEach(request => complete(request)); await tick();
  h.stopBackgroundPolling();
  h.configure({now:60001}); h.startBackgroundPolling(); h.document.emit(); await tick();
  assert.equal(requestCount(h,'backend_sync_session_status_now'),3);
  h.stopBackgroundPolling(); h.requests.forEach(request => complete(request)); await tick();
});

test('manual full AI refresh keeps result semantics and supersedes old status hydration', async () => {
  const h = await loadBackgroundRefresh();
  const old = h.refreshBackendAiTaskStatus(); await tick();
  const manual = h.backendAiRefreshNow(false); await tick();
  assert.deepEqual(h.requests[1].args,{force:false,keys:null});
  const fresh = aiStatus(true,500);
  h.requests[1].resolve(fresh);
  assert.strictEqual(await manual,fresh);
  h.requests[0].resolve(aiStatus(false,1)); await old;
  assert.deepEqual(h.state.ai,{notif:true,todo:true});
});

test('keyed manual AI refresh returns its result without modifying global scheduler state', async () => {
  const h = await loadBackgroundRefresh();
  const pending = h.refreshBackendAiTaskStatus(); await tick();
  const manual = h.backendAiRefreshNow(true,['ai_todo_analysis']); await tick();
  assert.deepEqual(h.requests[1].args,{force:true,keys:['ai_todo_analysis']});
  const result = aiStatus(true,500);
  h.requests[1].resolve(result);
  assert.strictEqual(await manual,result);
  assert.equal(h.state.ai,null);
  h.requests[0].resolve(aiStatus(false)); await pending;
  assert.deepEqual(h.state.ai,{notif:false,todo:false});
});

test('synchronous store re-entry joins the established AI read instead of issuing another', async () => {
  const h = await loadBackgroundRefresh();
  let reentered;
  h.configure({ai:() => { reentered = h.refreshBackendAiTaskStatus(); }});
  const pending = h.refreshBackendAiTaskStatus(); await tick();
  h.requests[0].resolve(aiStatus(true)); await pending;
  assert.strictEqual(reentered,pending);
  assert.equal(h.requests.length,1);
});

test('production re-login completion catches up an already active background owner once', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  h.requests.forEach(request => complete(request)); await tick();
  const login = await loadVisibleLogin();
  login.configure({poll:refresh => h.startBackgroundPolling(refresh)});
  const runs = Array.from({length:100}, () => login.initiateRelogin()); await tick();
  login.listeners.find(listener => listener.name === 'university-login-complete').receive({payload:{luna_authenticated:true,kwic_authenticated:true}});
  await Promise.all(runs); await tick();
  assert.equal(h.requests.length,4);
  assert.equal(h.document.registrations.length,1);
  h.requests.slice(2).forEach(request => complete(request,500)); await tick();
  assert.equal(h.state.tasks.get('schedule_data').lastRunTs,500000);
  h.stopBackgroundPolling();
});

test('production login identity catches up an already active background owner', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  h.requests.forEach(request => complete(request)); await tick();
  const login = await loadLogin();
  login.configure({poll:refresh => h.startBackgroundPolling(refresh)});
  login.login.mount(); await tick();
  login.listeners.find(listener => listener.name === 'login-success').receive({payload:{username:'new-user'}});
  await tick();
  assert.equal(h.requests.length,4);
  assert.equal(h.document.registrations.length,1);
  h.requests.slice(2).forEach(request => complete(request,501)); await tick();
  assert.equal(h.state.tasks.get('schedule_data').lastRunTs,501000);
  login.login.dispose(); h.stopBackgroundPolling();
});

test('one metadata request contains exactly the task DB keys and separate schedule flag', async () => {
  const h = await loadBackgroundRefresh();
  const pending = h.refreshBackendTaskStatuses(); await tick();
  assert.equal(h.requests.length,1);
  assert.equal(h.requests[0].name,'get_backend_task_timestamps');
  assert.deepEqual(h.requests[0].args, {
    keys:['notifications','luna_todo','luna_updates','mail_inbox','cancellations','makeup','rooms','weather',
      'student_profile','grades','exam_timetable','registration','kwic_home'], includeSchedule:true,
  });
  complete(h.requests[0]); await pending;
  assert.equal(h.state.updates.length,14);
});

test('failed metadata batch preserves null fallback and a later request retries once', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  const batch = h.requests.find(request => request.name === 'get_backend_task_timestamps');
  batch.reject(new Error('metadata unavailable'));
  h.requests.find(request => request.name === 'get_backend_ai_refresh_status').resolve(aiStatus()); await tick();
  for (const [key, task] of h.state.tasks) {
    if (key === 'ai_scheduler' || key === 'preemptive_renewal') continue;
    assert.equal(task.lastRunTs,null); assert.equal(task.lastOk,null);
  }
  assert.deepEqual(h.warnings,[]);
  const retry = h.refreshBackendTaskStatuses(); await tick();
  assert.equal(requestCount(h,'get_backend_task_timestamps'),2);
  complete(h.requests.at(-1),123); await retry;
  assert.equal(h.state.tasks.get('exams').lastRunTs,123000);
  h.stopBackgroundPolling();
});

test('a pushed key allows a new batch before the old batch settles without disturbing other indicators', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await tick();
  const old = h.requests.find(request => request.name === 'get_backend_task_timestamps');
  h.callbacks.get('backend-cache-updated')({payload:{keys:['luna_todo']}});
  const next = h.refreshBackendTaskStatuses(); await tick();
  const fresh = h.requests.at(-1);
  assert.equal(requestCount(h,'get_backend_task_timestamps'),2);
  complete(fresh,500); await tick();
  assert.equal(h.state.tasks.get('luna_todo').lastRunTs,500000);
  complete(old,42); await next;
  h.requests.find(request => request.name === 'get_backend_ai_refresh_status').resolve(aiStatus()); await tick();
  assert.equal(h.state.tasks.get('luna_todo').lastRunTs,500000);
  assert.equal(h.state.tasks.get('exams').lastRunTs,42000);
  assert.equal(h.state.updates.filter(([key]) => key === 'luna_todo').length,2);
  h.stopBackgroundPolling();
});

test('pushed key between indicator and batch microtasks leaves unrelated reads usable', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling();
  await Promise.resolve();
  h.callbacks.get('backend-cache-updated')({payload:{keys:['luna_todo']}});
  await tick();
  assert.equal(requestCount(h,'get_backend_task_timestamps'),1);
  h.requests.forEach(request => complete(request,42)); await tick();
  assert.equal(h.state.tasks.get('luna_todo').lastRunTs,100000);
  assert.equal(h.state.tasks.get('exams').lastRunTs,42000);
  assert.equal(h.state.updates.length,15);
  h.stopBackgroundPolling();
});

test('unrelated cache events do not abandon a shared task metadata read', async () => {
  const h = await loadBackgroundRefresh();
  const first = h.refreshBackendTaskStatuses(); await tick();
  h.callbacks.get('backend-cache-updated')({payload:{keys:['detail_generated_todo','live_generated_todo']}});
  const joined = h.refreshBackendTaskStatuses(); await tick();
  assert.equal(h.requests.length,1);
  complete(h.requests[0]); await Promise.all([first,joined]);
  assert.equal(h.state.updates.filter(([key]) => key === 'schedule_data').length,1);
});

test('stop between indicator and batch microtasks cancels the unissued metadata request', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling(); await Promise.resolve(); h.stopBackgroundPolling(); await tick();
  assert.equal(requestCount(h,'get_backend_task_timestamps'),0);
  h.requests.forEach(request => complete(request)); await tick();
  assert.deepEqual(h.state.updates,[]);
});

test('missing rows, negative stamps and a colliding raw schedule key keep independent metadata semantics', async () => {
  const h = await loadBackgroundRefresh();
  const pending = h.refreshBackendTaskStatuses(); await tick();
  assert.equal(h.requests.length,1);
  assert.equal(h.requests[0].name,'get_backend_task_timestamps');
  h.requests[0].resolve({rows:[{key:'exam_timetable',updated_at:99},{key:'weather',updated_at:-1},
    {key:'schedule_data',updated_at:999},{key:'unrelated',updated_at:77}],schedule_updated_at:0});
  await pending;
  const updates = new Map(h.state.updates);
  assert.equal(updates.size,14);
  assert.equal(updates.get('exams').lastRunTs,99000);
  for (const key of ['schedule_data','weather','grades']) assert.equal(updates.get(key).lastRunTs,null);
  assert.equal(updates.has('unrelated'),false);
});

test('demo task status reads remain local and preserve the synthetic schedule timestamp', async () => {
  const h = await loadBackgroundRefresh();
  h.configure({demo:true}); await h.refreshBackendTaskStatuses();
  assert.equal(h.requests.length,0);
  assert.equal(h.state.updates.length,14);
  assert.ok(h.state.updates.every(([key,value]) => key === 'schedule_data'
    ? value.lastRunTs === 100000000 && value.lastOk === true : value.lastRunTs === null && value.lastOk === null));
});


test('old native generation cannot overwrite pushed session status', async () => {
  const h = await loadBackgroundRefresh();
  h.universitySessionLifetime.accept(2);
  h.callbacks.get('backend-session-status')({payload:{generation:2,luna_authenticated:true,kwic_authenticated:true,session_expired:false}});
  const before = structuredClone(h.state.session);
  const updates = h.state.updates.length;
  h.callbacks.get('backend-session-status')({payload:{generation:1,luna_authenticated:false,kwic_authenticated:false,session_expired:true}});
  assert.deepEqual(h.state.session, before);
  assert.equal(h.state.updates.length, updates);
});

test('connectivity and focus catch-up share the session cooldown and release listeners on stop', async () => {
  const h = await loadBackgroundRefresh();
  h.startBackgroundPolling();
  h.window.emit('online'); h.window.emit('focus');
  await tick();
  assert.equal(h.requests.filter(r => r.name === 'backend_sync_session_status_now').length, 1);
  h.stopBackgroundPolling();
  assert.equal(h.window.listeners.size, 0);
});
