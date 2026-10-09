import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { LiveTranscriptFollow } = await loadTypeScript("src/lib/views/live/liveTranscriptFollow.ts");
const { ResourceScope } = await loadTypeScript("src/lib/resourceScope.ts");

function setup() {
  const scope = new ResourceScope();
  const frames = [], live = new Set();
  const schedule = callback => {
    const frame = { callback, cancels: 0 };
    frames.push(frame); live.add(frame);
    return () => { frame.cancels++; live.delete(frame); };
  };
  const target = { reads: 0, writes: [], height: 100,
    get scrollHeight() { this.reads++; return this.height; },
    set scrollTop(value) { this.writes.push(value); },
  };
  const state = { recordingId: "recording-a", lineCount: 0, target, visible: true, listening: true, autoFollow: true };
  const follow = new LiveTranscriptFollow(scope, schedule);
  const run = frame => { live.delete(frame); frame.callback(); };
  return { scope, frames, live, target, state, follow, run };
}

test("10000 line updates share one frame and one layout read using the latest height", () => {
  const s = setup();
  for (let i = 1; i <= 10_000; i++) s.follow.update({ ...s.state, lineCount: i });
  assert.equal(s.frames.length, 1);
  assert.equal(s.target.reads, 0);
  s.target.height = 12345;
  s.run(s.frames[0]);
  assert.equal(s.target.reads, 1);
  assert.deepEqual(s.target.writes, [12345]);
  for (let i = 0; i < 1000; i++) s.follow.update({ ...s.state, lineCount: 10_000 });
  assert.equal(s.frames.length, 1);
  s.follow.update({ ...s.state, lineCount: 10_001 });
  assert.equal(s.frames.length, 2);
  s.scope.dispose();
});

test("hidden, covered, paused and manually scrolled panes cancel pending work and resume at the same line count", () => {
  for (const field of ["visible", "listening", "autoFollow"]) {
    const s = setup();
    s.follow.update(s.state);
    const old = s.frames[0];
    s.follow.update({ ...s.state, [field]: false });
    assert.equal(old.cancels, 1);
    old.callback();
    assert.equal(s.target.reads, 0);
    assert.deepEqual(s.target.writes, []);
    for (let i = 0; i < 1000; i++) s.follow.update({ ...s.state, lineCount: i, [field]: false });
    assert.equal(s.frames.length, 1);
    s.follow.update(s.state);
    assert.equal(s.frames.length, 2);
    old.callback();
    s.run(s.frames[1]);
    assert.deepEqual(s.target.writes, [100]);
    s.scope.dispose();
  }
});

test("canceling a completed follow while hidden still schedules one catch-up frame on return", () => {
  const s = setup();
  s.follow.update(s.state); s.run(s.frames[0]);
  s.follow.update({ ...s.state, visible: false });
  s.follow.update(s.state);
  assert.equal(s.frames.length, 2);
  s.target.height = 900; s.run(s.frames[1]);
  assert.deepEqual(s.target.writes, [100, 900]);
  s.scope.dispose();
});

test("old recording frames cannot read or consume a replacement recording's pending frame", () => {
  const s = setup();
  s.follow.update({ ...s.state, lineCount: 25 });
  const old = s.frames[0];
  s.follow.update({ ...s.state, recordingId: "recording-b", lineCount: 25 });
  assert.equal(old.cancels, 1);
  old.callback();
  assert.equal(s.target.reads, 0);
  s.follow.update({ ...s.state, recordingId: "recording-b", lineCount: 26 });
  assert.equal(s.frames.length, 2);
  s.run(s.frames[1]);
  assert.deepEqual(s.target.writes, [100]);
  s.scope.dispose();
});

test("replacing or removing the DOM target invalidates old frames without reading it", () => {
  const s = setup();
  const next = { scrollHeight: 300, scrollTop: 0 };
  s.follow.update(s.state);
  s.follow.update({ ...s.state, target: next });
  s.frames[0].callback();
  assert.equal(s.target.reads, 0);
  s.run(s.frames[1]);
  assert.equal(next.scrollTop, 300);
  s.follow.update({ ...s.state, target: null });
  s.follow.update({ ...s.state, target: next });
  assert.equal(s.frames.length, 3);
  s.scope.dispose();
});

test("scope disposal invalidates queued frames and prevents future scheduling", () => {
  const s = setup();
  s.follow.update(s.state);
  s.scope.dispose(); s.scope.dispose();
  s.frames[0].callback();
  s.follow.update({ ...s.state, lineCount: 1 });
  assert.equal(s.target.reads, 0);
  assert.equal(s.frames.length, 1);
  assert.equal(s.frames[0].cancels, 1);
  const dead = new LiveTranscriptFollow(s.scope, () => { throw new Error("must not schedule"); });
  dead.update(s.state);
});

test("inactive records cannot follow an archived or preview transcript", () => {
  const s = setup();
  s.follow.update({ ...s.state, recordingId: null });
  assert.equal(s.frames.length, 0);
  s.scope.dispose();
});

test("a failed frame submission or layout read does not mark a line count as followed", () => {
  const s = setup();
  let attempts = 0, readAttempts = 0;
  const target = {
    get scrollHeight() { if (!readAttempts++) throw new Error("layout failure"); return 456; },
    scrollTop: 0,
  };
  const follow = new LiveTranscriptFollow(s.scope, callback => {
    if (!attempts++) throw new Error("submission failure");
    s.frames.push({ callback }); return () => {};
  });
  const state = { ...s.state, target };
  assert.throws(() => follow.update(state), /submission failure/);
  follow.update(state);
  assert.throws(() => s.frames[0].callback(), /layout failure/);
  follow.update(state);
  s.frames[1].callback();
  assert.equal(target.scrollTop, 456);
  s.scope.dispose();
});
