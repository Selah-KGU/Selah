import test from 'node:test';
import assert from 'node:assert/strict';
import { loadLiveTimers } from './load-live-timers.mjs';

const active = id => ({ active: true, session_id: id, finish_phase: null });
const saved = (id = 'a', text = 'saved', revision = 1) => ({ saved: true, snapshot: { session_id: id, update_revision: revision }, summary_markdown: text });
const timer = (h, kind, delay) => h.liveTimers().find(t => t.kind === kind && t.delay === delay);
const settle = () => new Promise(resolve => setImmediate(resolve));

test('one hundred saved notices own one timeout and obsolete callbacks cannot hide the latest save', async () => {
  const h = await loadLiveTimers();
  for (let i = 0; i < 100; i++) h.rememberSaved(saved('a', `saved ${i}`, i));
  assert.equal(h.liveTimers().length, 1);
  const current = timer(h, 'timeout', 6000);
  for (const old of h.timers.filter(t => t !== current)) h.fire(old.id);
  assert.equal(h.state.value.showSaveNotif, true);
  assert.equal(h.state.value.saveOwner, true);
  assert.deepEqual(h.state.value.lastSaved, { summary_markdown: 'saved 99' });
  h.fire(current.id);
  assert.equal(h.state.value.showSaveNotif, false);
  assert.equal(h.state.value.saveOwner, false);
  assert.equal(h.liveTimers().length, 0);
});

test('equal-text replacement notices reject old callbacks and retain the new cleanup owner', async () => {
  const h = await loadLiveTimers();
  h.setMessage('success', 'same text');
  const first = timer(h, 'timeout', 4000);
  h.setMessage('success', 'same text');
  const second = timer(h, 'timeout', 4000);
  h.fire(first.id);
  assert.equal(h.state.value.notice?.text, 'same text');
  assert.equal(h.state.value.noticeOwner, true);
  h.clearNotice();
  assert.equal(second.active, false);
  assert.equal(h.liveTimers().length, 0);
});

test('persistent readiness and error notices replace timeout owners without being erased', async () => {
  const h = await loadLiveTimers();
  h.setMessage('success', 'transient');
  const old = timer(h, 'timeout', 4000);
  h.setReadinessNotice('settings required');
  h.fire(old.id);
  assert.deepEqual(h.state.value.notice, { kind: 'warning', text: 'settings required', source: 'readiness', action: 'open-ai-settings' });
  h.clearReadinessNotice();
  assert.equal(h.state.value.notice, null);
  h.setMessage('error', 'real error');
  h.setReadinessNotice('must not replace this error');
  assert.equal(h.state.value.notice.text, 'real error');
  h.setNotice('warning', 'persistent', { autoClearMs: -1 });
  assert.equal(h.liveTimers().length, 0);
});

test('destroyed LIVE pages reject queued save and notice callbacks and release once', async () => {
  const h = await loadLiveTimers();
  h.rememberSaved(saved());
  h.setMessage('success', 'current notice');
  const queued = [...h.liveTimers()];
  h.destroy();
  h.destroy();
  const state = h.state.value;
  for (const old of queued) h.fire(old.id);
  assert.deepEqual(h.state.value, state);
  assert.equal(h.liveTimers().length, 0);
  assert.ok(queued.every(t => t.clears === 1));
  h.rememberSaved(saved('a', 'late saved response'));
  h.setMessage('success', 'late notice');
  assert.deepEqual(h.state.value, state);
});

test('hidden LIVE invalidates queued course-clock callbacks while visible restarts exactly one', async () => {
  const h = await loadLiveTimers();
  h.setState({ schedule: { courses: [] } });
  for (let i = 0; i < 100; i++) h.applyLiveSurfacePolicy(true, false, false);
  const old = timer(h, 'interval', 60000);
  assert.equal(h.liveTimers().length, 1);
  h.applyLiveSurfacePolicy(false, true, false);
  const before = h.state.value.now;
  const calls = h.calls.filter(c => c[0] === 'schedule').length;
  h.setTime(500000);
  h.fire(old.id);
  assert.equal(h.state.value.now, before);
  assert.equal(h.calls.filter(c => c[0] === 'schedule').length, calls);
  h.applyLiveSurfacePolicy(true, false, false);
  const current = timer(h, 'interval', 60000);
  assert.notEqual(current.id, old.id);
  h.setTime(600000);
  h.fire(old.id);
  assert.equal(h.state.value.now, 500000);
  h.fire(current.id);
  assert.equal(h.state.value.now, 600000);
});

test('recording auto checks continue while hidden and active snapshot replacement preserves one timer', async () => {
  const h = await loadLiveTimers();
  h.setState({ snapshot: active('a'), listening: true });
  h.markLiveListeningStarted();
  for (let i = 0; i < 100; i++) h.setState({ snapshot: { ...active('a'), transcript_line_count: i } });
  h.applyLiveSurfacePolicy(false, true, true);
  assert.equal(h.liveTimers().length, 1);
  const current = timer(h, 'interval', 60000);
  h.setTime(700000);
  h.fire(current.id);
  await settle();
  assert.deepEqual(h.calls.filter(c => c[0] === 'pause'), [['pause', true]]);
  assert.equal(current.active, true);
});

test('released auto-check callbacks cannot consume a replacement recording silence clock', async () => {
  const h = await loadLiveTimers();
  h.setState({ snapshot: active('a'), listening: true });
  const old = timer(h, 'interval', 60000);
  h.setState({ snapshot: { active: false } });
  h.setTime(200000);
  h.setState({ snapshot: active('b'), listening: true });
  h.markLiveListeningStarted();
  const current = timer(h, 'interval', 60000);
  h.setTime(800000);
  h.fire(old.id);
  await settle();
  assert.equal(h.calls.filter(c => c[0] === 'pause').length, 0);
  assert.equal(h.state.value.lastEffectiveSpeechAtMs, 200000);
  h.fire(current.id);
  await settle();
  assert.deepEqual(h.calls.filter(c => c[0] === 'pause'), [['pause', true]]);
});

test('busy auto checks do not overlap and paused recordings retain the twenty-minute finish rule', async () => {
  const h = await loadLiveTimers();
  let finish;
  h.configure({ stop: () => new Promise(resolve => { finish = resolve; }) });
  h.setState({ snapshot: active('a'), listening: false });
  h.markLivePaused();
  const current = timer(h, 'interval', 60000);
  h.setTime(1300000);
  for (let i = 0; i < 100; i++) h.fire(current.id);
  assert.deepEqual(h.calls.filter(c => c[0] === 'stop'), [['stop', true]]);
  assert.equal(h.state.value.autoLifecycleBusy, true);
  finish();
  await settle();
  assert.equal(h.state.value.autoLifecycleBusy, false);
  h.destroy();
  const state = h.state.value;
  h.fire(current.id);
  await h.checkLiveAutoLifecycle();
  assert.deepEqual(h.state.value, state);
  assert.equal(h.calls.filter(c => c[0] === 'stop').length, 1);
});

test('saving another active recording is ignored and unsaved replies do not create a badge timeout', async () => {
  const h = await loadLiveTimers();
  h.setState({ snapshot: active('b') });
  h.rememberSaved(saved('a', 'old result'));
  assert.equal(h.state.value.lastSaved, null);
  assert.equal(h.liveTimers().filter(t => t.kind === 'timeout').length, 0);
  h.rememberSaved({ ...saved('b', 'not saved'), saved: false });
  assert.equal(h.state.value.showSaveNotif, false);
  assert.equal(h.liveTimers().filter(t => t.kind === 'timeout').length, 0);
});
