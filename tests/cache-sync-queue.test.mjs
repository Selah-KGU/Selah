import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { createCacheSyncQueue } = await loadTypeScript(process.env.SELAH_CACHE_SYNC_QUEUE_SOURCE || "src/lib/cacheSyncQueue.ts");
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};

test("a burst of 200 updates becomes one batch and preserves forced refresh", async () => {
  const calls = [];
  const sync = createCacheSyncQueue(async (keys, stale) => calls.push({ keys, stale }));
  await Promise.all(Array.from({ length: 200 }, (_, i) => sync([i % 2 ? "todo" : "schedule"], i !== 99)));
  assert.deepEqual(calls, [{ keys: ["schedule", "todo"], stale: false }]);
});

test("an event during a read triggers another read with no overlapping applies", async () => {
  const firstStarted = deferred();
  const releaseFirst = deferred();
  const calls = [];
  let active = 0;
  const sync = createCacheSyncQueue(async keys => {
    assert.equal(++active, 1);
    calls.push(keys);
    if (calls.length === 1) { firstStarted.resolve(); await releaseFirst.promise; }
    active--;
  });
  const first = sync(["todo"]);
  await firstStarted.promise;
  const second = sync(["todo"]);
  const third = sync(["schedule"]);
  assert.equal(calls.length, 1);
  releaseFirst.resolve();
  await Promise.all([first, second, third]);
  assert.deepEqual(calls, [["todo"], ["todo", "schedule"]]);
});

test("a failed batch rejects its callers and the next request still runs", async () => {
  let calls = 0;
  const failure = new Error("IPC disconnected");
  const sync = createCacheSyncQueue(async () => { if (++calls === 1) throw failure; });
  const results = await Promise.allSettled([sync(["todo"]), sync(["schedule"])]);
  assert.ok(results.every(result => result.status === "rejected" && result.reason === failure));
  await sync(["todo"]);
  assert.equal(calls, 2);
});

test("an update queued during a failing read is still applied", async () => {
  const started = deferred();
  const release = deferred();
  let calls = 0;
  const sync = createCacheSyncQueue(async () => {
    if (++calls === 1) { started.resolve(); await release.promise; throw new Error("read failed"); }
  });
  const firstResult = sync(["todo"]).then(() => "resolved", () => "rejected");
  await started.promise;
  const second = sync(["todo"]);
  release.resolve();
  await second;
  assert.equal(await firstResult, "rejected");
  assert.equal(calls, 2);
});


test("ten thousand pending requests share one completion per batch, including forced refresh", async () => {
  const gates = [deferred(), deferred()];
  const started = [deferred(), deferred()];
  const calls = [];
  const sync = createCacheSyncQueue(async (keys, stale) => {
    const index = calls.length;
    calls.push({keys, stale});
    started[index].resolve();
    await gates[index].promise;
  });
  const first = Array.from({length:10000}, (_, i) => sync(["live_session", i % 2 ? "schedule" : "todo"], i !== 9999));
  await started[0].promise;
  const next = Array.from({length:10000}, (_, i) => sync([i % 2 ? "live_session" : "theme"], i !== 0));
  gates[0].resolve();
  await started[1].promise;
  let nextDone = false;
  next[0].then(() => {nextDone = true;});
  await first[0];
  await Promise.resolve();
  assert.equal(nextDone, false);
  // Release before identity assertions so the frozen predecessor also exits.
  gates[1].resolve();
  await Promise.all([...first, ...next]);
  assert.deepEqual(calls, [
    {keys:["live_session", "todo", "schedule"], stale:false},
    {keys:["theme", "live_session"], stale:false},
  ]);
  assert.equal(new Set(first).size, 1);
  assert.equal(new Set(next).size, 1);
  assert.notEqual(first[0], next[0]);
});

test("empty requests resolve immediately without joining or forcing a queued batch", async () => {
  const gate = deferred();
  const calls = [];
  const sync = createCacheSyncQueue(async (keys, stale) => { calls.push({keys, stale}); await gate.promise; });
  const queued = sync(["todo"], true);
  let done = false;
  queued.then(() => {done = true;});
  const empty = sync(["", ""], false);
  await empty;
  assert.equal(done, false);
  assert.notEqual(empty, queued);
  gate.resolve();
  await queued;
  assert.deepEqual(calls, [{keys:["todo"], stale:true}]);
});

test("synchronous reentrant apply requests belong to the next batch with their own failure", async () => {
  const failure = new Error("second batch failed");
  const calls = [];
  let nextResult;
  let second;
  let third;
  const sync = createCacheSyncQueue(async (keys, stale) => {
    calls.push({keys, stale});
    if (calls.length === 1) {
      second = sync(["schedule"], true);
      third = sync(["todo"], false);
      nextResult = Promise.allSettled([second, third]);
    } else if (calls.length === 2) throw failure;
  });
  const first = sync(["live_session"], true);
  await first;
  const result = await nextResult;
  assert.ok(result.every(value => value.status === "rejected" && value.reason === failure));
  await sync(["after failure"], true);
  assert.deepEqual(calls, [
    {keys:["live_session"], stale:true}, {keys:["schedule", "todo"], stale:false},
    {keys:["after failure"], stale:true},
  ]);
  assert.notEqual(first, second);
  assert.equal(second, third);
});

test("completion callbacks can enqueue another read without losing its keys or joining an old completion", async () => {
  const calls = [];
  const sync = createCacheSyncQueue(async keys => { calls.push(keys); });
  const first = sync(["first"]);
  let next;
  await first.then(() => { next = sync(["second"]); });
  await next;
  assert.notEqual(first, next);
  assert.deepEqual(calls, [["first"], ["second"]]);
});

test("primitive synchronous failure rejects only its batch and preserves the later forced request", async () => {
  let calls = 0;
  const sync = createCacheSyncQueue(() => {
    if (++calls === 1) throw "registry unavailable";
    return Promise.resolve();
  });
  const first = sync(["todo"]);
  const firstResult = first.catch(value => value);
  assert.equal(await firstResult, "registry unavailable");
  await sync(["todo"], false);
  assert.equal(calls, 2);
});
