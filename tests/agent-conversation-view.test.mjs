import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { ResourceScope, AgentConversationView, LatestViewRead } = await loadTypeScript("tests/fixtures/agent-conversation-view.ts");
function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function harness() {
  const scope = new ResourceScope();
  const reads = [], listeners = [], selected = [], shared = [], events = [], applied = [];
  let displayed = [];
  const view = new AgentConversationView({
    scope,
    loadMessages(id) {
      const read = { id, ...deferred() };
      reads.push(read);
      return read.promise;
    },
    listen(id, receive) {
      const listener = { id, receive, releases: 0, ...deferred() };
      listener.bind = () => listener.resolve(() => listener.releases++);
      listeners.push(listener);
      return listener.promise;
    },
    select(id, share) { selected.push(id); if (share) shared.push(id); displayed = []; },
    applyMessages(rows) { applied.push(rows); displayed = rows; },
    receive(event) { events.push(event); },
  });
  return { scope, view, reads, listeners, selected, shared, events, applied, get displayed() { return displayed; } };
}

test("selection immediately clears previous history and waits for both messages and the native subscription", async () => {
  const h = harness();
  const pending = h.view.select("A");
  let finished = false;
  void pending.then(() => { finished = true; });
  assert.deepEqual(h.selected, ["A"]);
  h.reads[0].resolve(["history A"]);
  await Promise.resolve();
  assert.deepEqual(h.displayed, ["history A"]);
  assert.equal(finished, false);
  // Repeated readiness requests share the same work instead of issuing IPC again.
  assert.equal(h.view.select("A"), pending);
  h.listeners[0].bind();
  assert.equal(await pending, true);
  assert.equal(await h.view.select("A"), true);
  assert.equal(h.reads.length, 1);
  h.scope.dispose();
});

test("out-of-order history and registration cannot overwrite or subscribe an older conversation", async () => {
  const h = harness();
  const first = h.view.select("A");
  const second = h.view.select("B");
  h.reads[1].resolve(["history B"]);
  h.listeners[1].bind();
  assert.equal(await second, true);
  h.reads[0].resolve(["history A"]);
  h.listeners[0].bind();
  assert.equal(await first, false);
  h.listeners[0].receive("stale A token");
  h.listeners[1].receive("B token");
  assert.deepEqual(h.displayed, ["history B"]);
  assert.deepEqual(h.events, ["B token"]);
  assert.equal(h.listeners[0].releases, 1);
  h.scope.dispose();
  assert.equal(h.listeners[1].releases, 1);
});

test("A to B to A distinguishes an earlier request with the same conversation ID", async () => {
  const h = harness();
  const first = h.view.select("A");
  const initialRead = h.view.capture();
  const second = h.view.select("B");
  const third = h.view.select("A");
  assert.equal(initialRead(), false);
  h.reads[2].resolve(["latest A"]);
  h.listeners[2].bind();
  assert.equal(await third, true);
  h.reads[0].resolve(["old A"]);
  h.listeners[0].bind();
  h.reads[1].resolve(["old B"]);
  h.listeners[1].bind();
  assert.deepEqual(await Promise.all([first, second]), [false, false]);
  h.listeners[0].receive("old A");
  h.listeners[2].receive("latest A");
  assert.deepEqual(h.applied, [["latest A"]]);
  assert.deepEqual(h.events, ["latest A"]);
  h.scope.dispose();
});

test("closing during history and listener registration discards both late results", async () => {
  const h = harness();
  const selected = h.view.select("A");
  const current = h.view.capture();
  h.scope.dispose();
  h.listeners[0].receive("queued after close");
  h.reads[0].resolve(["late history"]);
  h.listeners[0].bind();
  assert.equal(await selected, false);
  assert.equal(current(), false);
  assert.deepEqual(h.applied, []);
  assert.deepEqual(h.events, []);
  assert.equal(h.listeners[0].releases, 1);
  assert.equal(await h.view.select("B"), false);
  assert.equal(h.reads.length, 1);
});

test("a same-conversation recovery read cannot erase a newer local answer", async () => {
  const h = harness();
  const selected = h.view.select("A");
  h.reads[0].resolve(["history"]);
  h.listeners[0].bind();
  await selected;
  const recovery = h.view.reload();
  h.view.invalidateMessages(); // The production send path calls this before appending a new turn.
  h.reads[1].resolve(["snapshot before new answer"]);
  assert.equal(await recovery, false);
  assert.deepEqual(h.applied, [["history"]]);
  const newer = h.view.reload();
  h.reads[2].resolve(["complete new answer"]);
  assert.equal(await newer, true);
  assert.deepEqual(h.displayed, ["complete new answer"]);
  h.scope.dispose();
});

test("recovery rechecks the turn predicate after IO and only the latest recovery may apply", async () => {
  const h = harness();
  const selected = h.view.select("A");
  h.reads[0].resolve(["history"]);
  h.listeners[0].bind();
  await selected;
  let turnCurrent = true;
  const recovery = h.view.reload(() => turnCurrent);
  turnCurrent = false;
  h.reads[1].resolve(["old turn"]);
  assert.equal(await recovery, false);
  const older = h.view.reload();
  const newer = h.view.reload();
  h.reads[3].resolve(["newer recovery"]);
  assert.equal(await newer, true);
  h.reads[2].resolve(["older recovery"]);
  assert.equal(await older, false);
  assert.deepEqual(h.displayed, ["newer recovery"]);
  h.scope.dispose();
});

test("deleting the selection invalidates reads, removes its stream, and clears the visible state", async () => {
  const h = harness();
  const selected = h.view.select("A");
  h.listeners[0].bind();
  await Promise.resolve();
  h.view.clear();
  h.reads[0].resolve(["deleted history"]);
  assert.equal(await selected, false);
  h.listeners[0].receive("deleted stream");
  assert.deepEqual(h.selected, ["A", null]);
  assert.deepEqual(h.applied, []);
  assert.deepEqual(h.events, []);
  assert.equal(h.listeners[0].releases, 1);
  h.scope.dispose();
});

test("obsolete load failures are discarded while a current listener failure prevents readiness and permits retry", async () => {
  const h = harness();
  const old = h.view.select("A");
  const current = h.view.select("B");
  h.reads[0].reject(new Error("obsolete history failed"));
  h.listeners[0].bind();
  assert.equal(await old, false);
  h.reads[1].resolve(["B history"]);
  h.listeners[1].reject(new Error("current listener failed"));
  await assert.rejects(current, /current listener failed/);
  const retry = h.view.select("B");
  h.reads[2].resolve(["B retried"]);
  h.listeners[2].bind();
  assert.equal(await retry, true);
  h.scope.dispose();
});

test("adopting a shared conversation event does not publish the event back to other views", async () => {
  const h = harness();
  const adopted = h.view.select("shared", false);
  h.reads[0].resolve(["shared history"]);
  h.listeners[0].bind();
  assert.equal(await adopted, true);
  assert.deepEqual(h.selected, ["shared"]);
  assert.deepEqual(h.shared, []);
  const local = h.view.select("local");
  h.reads[1].resolve(["local history"]);
  h.listeners[1].bind();
  assert.equal(await local, true);
  assert.deepEqual(h.shared, ["local"]);
  h.scope.dispose();
});

test("a history failure also removes a listener which finishes registering after the failure", async () => {
  const h = harness();
  const selected = h.view.select("A");
  h.reads[0].reject(new Error("history failed"));
  await assert.rejects(selected, /history failed/);
  h.listeners[0].receive("failed selection event");
  h.listeners[0].bind();
  // Wait for the owned registration and its late-release continuation.
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(h.events, []);
  assert.equal(h.listeners[0].releases, 1);
  const retry = h.view.select("A");
  h.reads[1].resolve(["retried history"]);
  h.listeners[1].bind();
  assert.equal(await retry, true);
  h.scope.dispose();
});

test("a deletion discards pending history and registration without publishing an empty shared selection", async () => {
  const h = harness();
  const pending = h.view.select("A");
  const current = h.view.capture();
  assert.equal(h.view.deleted("A"), true);
  assert.equal(current(), false);
  assert.equal(h.view.deleted("A"), false);
  h.reads[0].resolve(["deleted A"]);
  h.listeners[0].bind();
  assert.equal(await pending, false);
  h.listeners[0].receive("late deleted A token");
  assert.deepEqual(h.selected, ["A", null]);
  assert.deepEqual(h.shared, ["A"]);
  assert.deepEqual(h.applied, []);
  assert.deepEqual(h.events, []);
  assert.equal(h.listeners[0].releases, 1);
  h.scope.dispose();
});

test("an older conversation deletion cannot invalidate a newer selection or its recovery", async () => {
  const h = harness();
  const old = h.view.select("A");
  const newer = h.view.select("B");
  const current = h.view.capture();
  assert.equal(h.view.deleted("A"), false);
  assert.equal(current(), true);
  h.reads[1].resolve(["B history"]);
  h.listeners[1].bind();
  assert.equal(await newer, true);
  const recovery = h.view.reload();
  assert.equal(h.view.deleted("A"), false);
  h.reads[2].resolve(["B complete answer"]);
  assert.equal(await recovery, true);
  h.reads[0].resolve(["deleted A history"]);
  h.listeners[0].bind();
  assert.equal(await old, false);
  h.listeners[1].receive("B token");
  assert.deepEqual(h.displayed, ["B complete answer"]);
  assert.deepEqual(h.selected, ["A", "B"]);
  assert.deepEqual(h.shared, ["A", "B"]);
  assert.deepEqual(h.events, ["B token"]);
  assert.equal(h.listeners[1].releases, 0);
  h.scope.dispose();
});

test("deleting a ready conversation releases its stream and leaves explicit clearing shareable", async () => {
  const h = harness();
  const selected = h.view.select("A", false);
  h.reads[0].resolve(["A history"]);
  h.listeners[0].bind();
  await selected;
  assert.equal(h.view.deleted("A"), true);
  h.listeners[0].receive("A token after delete");
  assert.equal(h.listeners[0].releases, 1);
  assert.deepEqual(h.events, []);
  assert.deepEqual(h.shared, []);
  h.view.clear();
  assert.deepEqual(h.shared, [null]);
  h.scope.dispose();
});

test("reordered shared selection notifications use the newest durable pointer, and local intent invalidates reads", async () => {
  const h = harness();
  const snapshots = [];
  const read = new LatestViewRead(h.scope, () => {
    const snapshot = deferred();
    snapshots.push(snapshot);
    return snapshot.promise;
  }, (id) => {
    if (id) void h.view.select(id, false);
    else h.view.clear(false);
  });
  const older = read.refresh();
  const newer = read.refresh();
  snapshots[1].resolve("B");
  assert.equal(await newer, true);
  h.reads[0].resolve(["B history"]);
  h.listeners[0].bind();
  await h.view.select("B", false);
  snapshots[0].resolve("deleted A");
  assert.equal(await older, false);
  assert.deepEqual(h.selected, ["B"]);
  const pendingEmpty = read.refresh();
  read.invalidate(); // Local selection or deletion supersedes pending shared IO.
  const local = h.view.select("C");
  snapshots[2].resolve(null);
  assert.equal(await pendingEmpty, false);
  h.reads[1].resolve(["C history"]);
  h.listeners[1].bind();
  await local;
  assert.deepEqual(h.selected, ["B", "C"]);
  assert.deepEqual(h.shared, ["C"]);
  h.scope.dispose();
});
