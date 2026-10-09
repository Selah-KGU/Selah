import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { ResourceScope, ResourceSlot, acquireResourceGroup } = await loadTypeScript("src/lib/resourceScope.ts");
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
};

test("owned intervals stop queued ticks after release or disposal and cannot revive with a new timer", () => {
  const set = globalThis.setInterval, clear = globalThis.clearInterval;
  const queued = [], live = new Set(), cleared = [];
  globalThis.setInterval = (callback, delay) => {
    const timer = { callback, delay };
    queued.push(timer); live.add(timer); return timer;
  };
  globalThis.clearInterval = timer => { cleared.push(timer); live.delete(timer); };
  try {
    const scope = new ResourceScope();
    let ticks = 0;
    const release = scope.interval(() => ticks++, 4500);
    assert.equal(queued[0].delay, 4500);
    queued[0].callback();
    assert.equal(ticks, 1);
    release(); release();
    queued[0].callback();
    assert.equal(ticks, 1);
    assert.equal(cleared.length, 1);
    scope.interval(() => ticks++, 30_000);
    queued[0].callback(); queued[1].callback();
    assert.equal(ticks, 2);
    assert.equal(live.size, 1);
    scope.dispose(); scope.dispose();
    queued[0].callback(); queued[1].callback();
    assert.equal(ticks, 2);
    assert.equal(live.size, 0);
    assert.equal(cleared.length, 2);
    scope.interval(() => ticks++, 1)();
    assert.equal(queued.length, 2);
  } finally { globalThis.setInterval = set; globalThis.clearInterval = clear; }
});

test("closing during native registration disables queued callbacks and releases the late listener once", async () => {
  const scope = new ResourceScope();
  const registered = deferred();
  let callbacks = 0, releases = 0;
  const callback = scope.guard(() => callbacks++);
  const pending = scope.acquire(() => registered.promise);
  callback();
  assert.equal(callbacks, 1);
  scope.dispose();
  callback();
  registered.resolve(() => releases++);
  const release = await pending;
  release();
  scope.dispose();
  assert.equal(callbacks, 1);
  assert.equal(releases, 1);
  let factories = 0;
  await scope.acquire(async () => { factories++; return () => {}; });
  assert.equal(factories, 0);
});

test("cleanup failures do not keep remaining subscriptions alive", async () => {
  const errors = [], released = [];
  const scope = new ResourceScope(error => errors.push(error.message));
  scope.own(() => released.push("first"));
  scope.own(() => { throw new Error("sync release"); });
  scope.own(() => Promise.reject(new Error("async release")));
  scope.own(() => released.push("last"));
  scope.dispose();
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(released, ["last", "first"]);
  assert.deepEqual(errors.sort(), ["async release", "sync release"]);
});

test("late conversation registrations cannot replace the current stream or publish stale tokens", async () => {
  const scope = new ResourceScope();
  const slot = new ResourceSlot(scope);
  const firstReady = deferred(), secondReady = deferred();
  const listening = new Set(), tokens = [], released = [];
  let firstEvent, secondEvent;
  const first = slot.replace(current => {
    listening.add("first");
    firstEvent = () => { if (current()) tokens.push("old"); };
    return firstReady.promise;
  });
  const second = slot.replace(current => {
    listening.add("second");
    secondEvent = () => { if (current()) tokens.push("new"); };
    return secondReady.promise;
  });
  firstEvent();
  secondEvent();
  secondReady.resolve(() => { listening.delete("second"); released.push("second"); });
  assert.equal(await second, true);
  firstReady.resolve(() => { listening.delete("first"); released.push("first"); });
  assert.equal(await first, false);
  assert.deepEqual([...listening], ["second"]);
  assert.deepEqual(tokens, ["new"]);
  scope.dispose();
  secondEvent();
  assert.deepEqual(tokens, ["new"]);
  assert.equal(listening.size, 0);
  assert.deepEqual(released, ["first", "second"]);
});

test("switching away from an already bound stream removes it before starting the next registration", async () => {
  const scope = new ResourceScope();
  const slot = new ResourceSlot(scope);
  const order = [];
  await slot.replace(async () => { order.push("first registered"); return () => order.push("first released"); });
  await slot.replace(async () => { order.push("second registered"); return () => order.push("second released"); });
  scope.dispose();
  assert.deepEqual(order, ["first registered", "first released", "second registered", "second released"]);
});

test("obsolete registration failures are discarded while current failures remain visible", async () => {
  const scope = new ResourceScope();
  const slot = new ResourceSlot(scope);
  const ready = deferred();
  const obsolete = slot.replace(() => ready.promise);
  await slot.replace(async () => () => {});
  ready.reject(new Error("obsolete failure"));
  assert.equal(await obsolete, false);
  await assert.rejects(slot.replace(async () => { throw new Error("current failure"); }), /current failure/);
  scope.dispose();
});

test("replacement timers do not clear a newer message and closing cancels all pending work", t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const scope = new ResourceScope();
  let status = "first";
  const clearOld = scope.schedule(() => { status = ""; }, 4000);
  t.mock.timers.tick(1000);
  clearOld();
  status = "loading";
  t.mock.timers.tick(4000);
  assert.equal(status, "loading");
  scope.schedule(() => { status = "unexpected"; }, 4000);
  scope.dispose();
  scope.schedule(() => { status = "registered after close"; }, 1);
  t.mock.timers.tick(5000);
  assert.equal(status, "loading");
});

test("a timed cleanup which fires normally remains idempotent when the page closes", t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const scope = new ResourceScope();
  let calls = 0;
  const release = scope.schedule(() => calls++, 20);
  t.mock.timers.tick(20);
  release();
  scope.dispose();
  t.mock.timers.tick(50);
  assert.equal(calls, 1);
});

test("child scopes close independently, inherit cleanup error handling, and close with their parent", () => {
  const errors = [], calls = [];
  const parent = new ResourceScope(error => errors.push(error.message));
  const first = parent.fork(), second = parent.fork();
  first.own(() => calls.push("first"));
  first.own(() => { throw new Error("child cleanup"); });
  second.own(() => calls.push("second"));
  first.dispose();
  assert.equal(parent.active, true);
  assert.equal(second.active, true);
  assert.deepEqual(calls, ["first"]);
  parent.dispose();
  first.dispose();
  assert.deepEqual(calls, ["first", "second"]);
  assert.deepEqual(errors, ["child cleanup"]);
  assert.equal(parent.fork().active, false);
});

test("one failed group registration disables established and pending callbacks and releases late listeners", async () => {
  const parent = new ResourceScope();
  const failing = deferred(), delayed = deferred();
  const events = [], releases = [];
  let establishedEvent, delayedEvent;
  const pending = acquireResourceGroup(parent, [
    async group => {
      establishedEvent = group.guard(() => events.push("established"));
      return () => releases.push("established");
    },
    () => failing.promise,
    group => {
      delayedEvent = group.guard(() => events.push("delayed"));
      return delayed.promise;
    },
  ]);
  await Promise.resolve();
  establishedEvent();
  failing.reject(new Error("registration failed"));
  await assert.rejects(pending, /registration failed/);
  establishedEvent();
  delayedEvent();
  assert.deepEqual(events, ["established"]);
  assert.deepEqual(releases, ["established"]);
  delayed.resolve(() => releases.push("delayed"));
  await new Promise(resolve => setImmediate(resolve));
  parent.dispose();
  assert.deepEqual(releases, ["established", "delayed"]);
});

test("unbinding a pending STT group immediately releases listeners already registered and silences queued events", async () => {
  const parent = new ResourceScope();
  const slot = new ResourceSlot(parent);
  const delayed = deferred();
  const events = [], releases = [];
  let firstEvent, delayedEvent;
  const pending = slot.replace((current, registrationScope) => acquireResourceGroup(registrationScope, [
    async group => {
      firstEvent = group.guard(() => { if (current()) events.push("first"); });
      return () => releases.push("first");
    },
    group => {
      delayedEvent = group.guard(() => { if (current()) events.push("delayed"); });
      return delayed.promise;
    },
  ]));
  await Promise.resolve();
  firstEvent();
  slot.clear();
  assert.deepEqual(releases, ["first"]);
  firstEvent();
  delayedEvent();
  const replacement = slot.replace(async current => {
    assert.equal(current(), true);
    return () => releases.push("replacement");
  });
  assert.equal(await replacement, true);
  delayed.resolve(() => releases.push("delayed"));
  assert.equal(await pending, false);
  assert.deepEqual(events, ["first"]);
  parent.dispose();
  assert.deepEqual(releases, ["first", "delayed", "replacement"]);
});

test("closing during group registration skips future factories and releases each late handle once", async () => {
  const parent = new ResourceScope();
  const first = deferred(), second = deferred();
  let calls = 0;
  const pending = acquireResourceGroup(parent, [() => first.promise, () => second.promise]);
  parent.dispose();
  first.resolve(() => calls++);
  second.resolve(() => calls++);
  const release = await pending;
  release();
  parent.dispose();
  assert.equal(calls, 2);
  await acquireResourceGroup(parent, [async () => { assert.fail("factory after close"); }]);
});

test('released one-shot timers ignore queued callbacks and cannot consume replacement work', () => {
  const set = globalThis.setTimeout, clear = globalThis.clearTimeout;
  const queued = [], live = new Set(), cleared = [];
  globalThis.setTimeout = callback => { const timer = { callback }; queued.push(timer); live.add(timer); return timer; };
  globalThis.clearTimeout = timer => { cleared.push(timer); live.delete(timer); };
  try {
    const scope = new ResourceScope(); const calls = [];
    const old = scope.schedule(() => calls.push('obsolete'), 140);
    old(); old();
    const latest = scope.schedule(() => calls.push('current'), 140);
    queued[0].callback(); assert.deepEqual(calls, []); assert.equal(live.size, 1);
    queued[1].callback(); queued[1].callback(); latest();
    assert.deepEqual(calls, ['current']); assert.equal(live.size, 0); assert.equal(cleared.length, 2);
    const child = scope.fork(); child.schedule(() => calls.push('closed child'), 140); child.dispose(); queued[2].callback();
    scope.schedule(() => calls.push('closed parent'), 140); scope.dispose(); queued[3].callback();
    scope.schedule(() => calls.push('after close'), 140);
    assert.deepEqual(calls, ['current']); assert.equal(queued.length, 4); assert.equal(live.size, 0);
  } finally { globalThis.setTimeout = set; globalThis.clearTimeout = clear; }
});
