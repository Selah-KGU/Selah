import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
const { TrayStatusRefresh, TrayStatusWriter, ResourceScope } = await loadTypeScript("tests/fixtures/tray-status-refresh.ts");
const flush = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function harness(t) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const scope = new ResourceScope(), reads = [], writes = [], activity = [], errors = [];
  const writer = new TrayStatusWriter(async items => { writes.push(items); });
  const refresh = new TrayStatusRefresh(scope, {
    read: () => { const read = deferred(); reads.push(read); return read.promise; },
    build: status => [status], writer,
    activity: patch => activity.push(patch), failed: error => errors.push(error.message),
  });
  t.after(() => scope.dispose());
  return { scope, refresh, reads, writes, activity, errors,
    async tick() { t.mock.timers.tick(300); await flush(); },
  };
}

test("200 synchronous status events share one metadata read and one publication", async t => {
  const h = harness(t);
  for (let count = 0; count < 200; count++) h.refresh.request();
  await h.tick(); assert.equal(h.reads.length, 1);
  h.reads[0].resolve("Live記録中 10分"); await flush();
  assert.deepEqual(h.writes, [["Live記録中 10分"]]);
  assert.equal(h.activity.find(patch => patch.lastOk !== undefined).lastOk, true);
});

test("events during a delayed read discard its obsolete result and share one following read", async t => {
  const h = harness(t);
  h.refresh.request(); await h.tick();
  for (let count = 0; count < 200; count++) h.refresh.request();
  await h.tick(); assert.equal(h.reads.length, 1, "reads overlapped");
  h.reads[0].resolve("old live owner"); await flush();
  assert.deepEqual(h.writes, []);
  assert.equal(h.reads.length, 2);
  h.reads[1].resolve("new live owner"); await flush();
  assert.deepEqual(h.writes, [["new live owner"]]);
});

test("an event invalidates a read immediately, before its debounce timer fires", async t => {
  const h = harness(t);
  h.refresh.request(); await h.tick();
  h.refresh.request();
  h.reads[0].resolve("obsolete before debounce"); await flush();
  assert.deepEqual(h.writes, []);
  await h.tick(); h.reads[1].resolve("current"); await flush();
  assert.deepEqual(h.writes, [["current"]]);
});

test("disposal removes pending timers and prevents a late read from writing or updating tasks", async t => {
  const h = harness(t);
  h.refresh.request(); await h.tick();
  h.scope.dispose(); const before = [...h.activity];
  h.reads[0].resolve("late response"); await flush();
  h.refresh.request(); await h.tick();
  assert.equal(h.reads.length, 1);
  assert.deepEqual(h.writes, []);
  assert.deepEqual(h.activity, before);
});

test("a failed current read leaves the previous native status intact and can retry", async t => {
  const h = harness(t);
  h.refresh.request(); await h.tick(); h.reads[0].resolve("recording"); await flush();
  h.refresh.request(); await h.tick(); h.reads[1].reject(new Error("IPC unavailable")); await flush();
  assert.deepEqual(h.writes, [["recording"]]);
  assert.deepEqual(h.errors, ["IPC unavailable"]);
  assert.equal(h.activity.filter(patch => patch.lastOk !== undefined).at(-1).lastOk, false);
  h.refresh.request(); await h.tick(); h.reads[2].resolve("paused"); await flush();
  assert.deepEqual(h.writes, [["recording"], ["paused"]]);
});

test("obsolete read failures are ignored while queued updates continue", async t => {
  const h = harness(t);
  h.refresh.request(); await h.tick(); h.refresh.request(); await h.tick();
  h.reads[0].reject(new Error("old IPC failure")); await flush();
  assert.deepEqual(h.errors, []);
  h.reads[1].resolve("current"); await flush();
  assert.deepEqual(h.writes, [["current"]]);
});

test("equal successful items skip native writes and preserve cycle order", async () => {
  const writes = [], writer = new TrayStatusWriter(async items => { writes.push(items); });
  const items = ["live", "class"];
  assert.equal(await writer.write(items, () => true), true);
  items[0] = "mutated caller array";
  assert.equal(await writer.write(["live", "class"], () => true), true);
  assert.deepEqual(writes, [["live", "class"]]);
  await writer.write(["class", "live"], () => true);
  assert.deepEqual(writes[1], ["class", "live"]);
});

test("an in-flight old write finishes before the new run and an obsolete queued clear is skipped", async () => {
  const writes = [], release = deferred();
  let restarted = false;
  const writer = new TrayStatusWriter(async items => {
    writes.push(items);
    if (writes.length === 1) await release.promise;
  });
  const old = writer.write(["old run"], () => true); await flush();
  const clear = writer.write([], () => !restarted);
  restarted = true;
  const latest = writer.write(["new run"], () => restarted);
  await flush(); assert.deepEqual(writes, [["old run"]]);
  release.resolve();
  assert.deepEqual(await Promise.all([old, clear, latest]), [true, false, true]);
  assert.deepEqual(writes, [["old run"], ["new run"]]);
});

test("obsolete queued writes never publish and stop clears after issued writes", async () => {
  const writes = [], release = deferred(); let current = true;
  const writer = new TrayStatusWriter(async items => {
    writes.push(items); if (writes.length === 1) await release.promise;
  });
  const initial = writer.write(["initial"], () => true); await flush();
  const obsolete = writer.write(["obsolete"], () => current);
  current = false;
  const clear = writer.write([], () => true);
  release.resolve();
  assert.deepEqual(await Promise.all([initial, obsolete, clear]), [true, false, true]);
  assert.deepEqual(writes, [["initial"], []]);
});

test("publication failure does not cache its items or poison later writes", async () => {
  const writes = []; let fail = true;
  const writer = new TrayStatusWriter(async items => {
    writes.push(items); if (fail) { fail = false; throw new Error("native tray write failed"); }
  });
  await assert.rejects(writer.write(["same"], () => true), /native tray write failed/);
  assert.equal(await writer.write(["same"], () => true), true);
  await writer.write([], () => true);
  assert.deepEqual(writes, [["same"], ["same"], []]);
});
