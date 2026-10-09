import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
const { AgentSpeechInput, ResourceScope } = await loadTypeScript("tests/fixtures/agent-speech-input.ts");
function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const flush = () => new Promise(resolve => setImmediate(resolve));
const live = { caller: "live", live_session_id: "recording-a", input_session_id: null };
const listening = owner => ({ phase: "listening", session_id: 20, owner });
function harness(prefix = "input") {
  const scope = new ResourceScope(), starts = [], stops = [], reads = [], changed = [];
  let count = 0, nativeState = { phase: "idle", session_id: null, owner: null };
  let holdReads = false;
  const transport = {
    createId: () => `${prefix}-${++count}`,
    start: (owner, preempt = false) => {
      const request = { owner, preempt, ...deferred() };
      starts.push(request);
      // Restores complete immediately; tests still inspect exact owner and flag.
      if (!preempt) request.resolve(null);
      return request.promise;
    },
    stop: async owner => { stops.push(owner); },
    read: () => {
      const request = deferred();
      reads.push(request);
      if (!holdReads) request.resolve(nativeState);
      return request.promise;
    },
  };
  const input = new AgentSpeechInput(scope, active => changed.push(active), transport);
  return { scope, input, starts, stops, reads, changed,
    get owner() { return starts.find(item => item.preempt)?.owner; },
    event(state, id = starts.at(-1)?.owner.input_session_id) {
      return { caller: "agent", input_session_id: id, state };
    },
    holdReads() { holdReads = true; },
    setState(state) { nativeState = state; },
    async begin(previous = null) {
      const starting = input.start();
      const request = starts.at(-1);
      nativeState = listening(request.owner);
      request.resolve(previous);
      await starting;
      return request.owner;
    },
  };
}

test("each Agent view accepts only its own requested input, even before start returns", async () => {
  const h = harness();
  assert.equal(await h.input.refresh(), false);
  assert.equal(h.reads.length, 0, "an idle view adopted another window's microphone");
  const pending = h.input.start(), owner = h.owner;
  assert.equal(h.input.accepts({ caller: "agent", input_session_id: owner.input_session_id }), true);
  assert.equal(h.input.accepts({ caller: "agent", input_session_id: "other-window" }), false);
  assert.equal(h.input.accepts({ caller: "agent" }), false);
  assert.equal(h.input.accepts({ caller: "native_agent", input_session_id: owner.input_session_id }), false);
  h.input.state(h.event("initializing", owner.input_session_id));
  assert.deepEqual(h.changed, [true]);
  h.setState(listening(owner)); h.starts[0].resolve(null);
  assert.equal(await pending, true);
  h.scope.dispose(); await flush();
  assert.deepEqual(h.stops, [owner]);
});

test("closing while start is queued stops that UUID again after the delayed response", async () => {
  const h = harness(), pending = h.input.start(), owner = h.owner;
  h.scope.dispose(); await flush();
  assert.deepEqual(h.stops, [owner]);
  assert.equal(h.input.accepts(h.event("listening", owner.input_session_id)), false);
  h.starts[0].resolve(live);
  assert.equal(await pending, false);
  assert.deepEqual(h.stops, [owner, owner]);
  assert.equal(h.reads.length, 0);
  assert.deepEqual(h.changed, []);
  assert.deepEqual(h.starts[1].owner, live);
  assert.equal(h.starts[1].preempt, false);
  h.scope.dispose(); await flush();
  assert.equal(h.stops.length, 2);
});

test("idle before the start response still restores the borrowed recording exactly once", async () => {
  const h = harness(), pending = h.input.start(), owner = h.owner;
  h.input.state(h.event("initializing", owner.input_session_id));
  h.input.state(h.event("idle", owner.input_session_id));
  assert.equal(h.starts.length, 1, "restored before the previous owner was known");
  assert.equal(await h.input.start(), false, "idle allowed a duplicate while the first response was pending");
  h.starts[0].resolve(live); await pending;
  h.input.state(h.event("idle", owner.input_session_id));
  h.scope.dispose(); await flush();
  assert.deepEqual(h.changed, [true, false, false]);
  assert.equal(h.starts.filter(item => !item.preempt).length, 1);
  assert.deepEqual(h.starts[1].owner, live);
});

test("a closed recording releases its own input and restores LIVE without future UI updates", async () => {
  const h = harness(), owner = await h.begin(live);
  h.scope.dispose(); await flush();
  assert.deepEqual(h.stops, [owner]);
  assert.deepEqual(h.starts[1].owner, live);
  const before = [...h.changed];
  h.input.state(h.event("listening", owner.input_session_id));
  h.input.error(h.event("idle", owner.input_session_id));
  assert.deepEqual(h.changed, before);
});

test("a new input ignores delayed text, errors, idle and state reads from the old input", async () => {
  const h = harness(), oldOwner = await h.begin();
  h.holdReads();
  const oldRead = h.input.refresh();
  h.input.state(h.event("idle", oldOwner.input_session_id));
  const newStart = h.input.start(), request = h.starts.at(-1), newOwner = request.owner;
  assert.notEqual(oldOwner.input_session_id, newOwner.input_session_id);
  h.input.state(h.event("initializing", newOwner.input_session_id));
  request.resolve(null); await flush();
  h.reads.at(-1).resolve(listening(newOwner)); await newStart;
  const before = [...h.changed];
  h.input.state(h.event("idle", oldOwner.input_session_id));
  h.input.error(h.event("idle", oldOwner.input_session_id));
  assert.equal(h.input.accepts({ caller: "agent", input_session_id: oldOwner.input_session_id, text: "old tail" }), false);
  h.reads[1].resolve({ phase: "idle", session_id: null, owner: null });
  assert.equal(await oldRead, false);
  assert.deepEqual(h.changed, before);
  await h.input.stop();
  assert.deepEqual(h.stops, [newOwner]);
  h.scope.dispose(); await flush();
});

test("pending and active input prevent duplicate starts and foreign errors do not clear it", async () => {
  const h = harness(), pending = h.input.start(), owner = h.owner;
  assert.equal(await h.input.start(), false);
  assert.equal(h.starts.length, 1);
  h.setState(listening(owner)); h.starts[0].resolve(null); await pending;
  assert.equal(await h.input.start(), false);
  const before = [...h.changed];
  h.input.error(h.event("idle", "other-window"));
  assert.deepEqual(h.changed, before);
  h.input.error(h.event("idle", owner.input_session_id));
  assert.equal(h.changed.at(-1), false);
  // The native idle event is required before a new input can replace it.
  assert.equal(await h.input.start(), false);
  h.scope.dispose(); await flush();
});

test("a transient status failure retains recording state and old reads cannot revive idle", async () => {
  const h = harness(), owner = await h.begin();
  h.holdReads();
  const pending = h.input.refresh();
  h.input.state(h.event("idle", owner.input_session_id));
  h.reads.at(-1).reject(new Error("obsolete transport failure"));
  assert.equal(await pending, false);
  assert.equal(h.changed.at(-1), false);
  const originalWarn = console.warn;
  console.warn = () => {};
  try {
    const current = h.input.refresh(), before = [...h.changed];
    h.reads.at(-1).reject(new Error("current transport failure"));
    assert.equal(await current, false);
    assert.deepEqual(h.changed, before);
  } finally { console.warn = originalWarn; }
  h.scope.dispose(); await flush();
});

test("stopping keeps final-tail events available until a new input replaces the UUID", async () => {
  const h = harness(), owner = await h.begin();
  await h.input.stop();
  assert.equal(h.input.accepts({ caller: "agent", input_session_id: owner.input_session_id, text: "last sentence" }), true);
  assert.deepEqual(h.stops, [owner]);
  h.scope.dispose(); await flush();
});

test("an active start failure can retry and never resumes an unreported previous owner", async () => {
  const h = harness(), failed = h.input.start();
  h.starts[0].reject(new Error("model not ready"));
  await assert.rejects(failed, /model not ready/);
  assert.equal(h.changed.at(-1), false);
  assert.equal(h.starts.length, 1);
  const retry = h.input.start(), request = h.starts.at(-1);
  h.setState(listening(request.owner)); request.resolve(null);
  assert.equal(await retry, true);
  h.scope.dispose(); await flush();
});


test("two Agent windows cannot consume each other's text or stop identities", async () => {
  const first = harness("main"), second = harness("panel");
  const firstOwner = await first.begin(live), secondOwner = await second.begin(live);
  const firstFinal = { caller: "agent", input_session_id: firstOwner.input_session_id, text: "main tail" };
  const secondFinal = { caller: "agent", input_session_id: secondOwner.input_session_id, text: "panel speech" };
  assert.equal(first.input.accepts(firstFinal), true);
  assert.equal(first.input.accepts(secondFinal), false);
  assert.equal(second.input.accepts(firstFinal), false);
  assert.equal(second.input.accepts(secondFinal), true);
  const before = [...second.changed];
  first.scope.dispose(); await flush();
  second.input.state({ ...firstFinal, state: "idle" });
  assert.deepEqual(second.changed, before);
  assert.deepEqual(first.stops, [firstOwner]);
  assert.deepEqual(second.stops, []);
  second.scope.dispose(); await flush();
  assert.deepEqual(second.stops, [secondOwner]);
});

test("a read completing after closure does not publish state or duplicate the borrowed return", async () => {
  const h = harness(), owner = await h.begin(live);
  h.holdReads();
  const reading = h.input.refresh(), before = [...h.changed];
  h.scope.dispose(); await flush();
  h.reads.at(-1).resolve(listening(owner));
  assert.equal(await reading, false);
  assert.deepEqual(h.changed, before);
  assert.equal(h.starts.filter(item => !item.preempt).length, 1);
});

test("a rejected borrowed return cannot preempt a replacement or create an unhandled failure", async () => {
  const scope = new ResourceScope(), calls = [];
  const transport = {
    createId: () => "input-a",
    read: async () => ({ phase: "idle", owner: null, session_id: null }),
    stop: async owner => { calls.push({ kind: "stop", owner }); },
    start: (owner, preempt = false) => {
      calls.push({ kind: "start", owner, preempt });
      if (preempt) return Promise.resolve(live);
      throw new Error("another input owns the microphone");
    },
  };
  const input = new AgentSpeechInput(scope, () => {}, transport);
  assert.equal(await input.start(), true);
  input.state({ caller: "agent", input_session_id: "input-a", state: "idle" });
  scope.dispose(); await flush();
  const returns = calls.filter(item => item.kind === "start" && !item.preempt);
  assert.equal(returns.length, 1);
  assert.deepEqual(returns[0].owner, live);
  assert.equal(returns[0].preempt, false);
});
