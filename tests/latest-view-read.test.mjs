import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { LatestViewRead, ResourceScope } = await loadTypeScript("tests/fixtures/latest-view-read.ts");
function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function harness() {
  const scope = new ResourceScope();
  const requests = [], applied = [], errors = [];
  const read = new LatestViewRead(scope, (key) => {
    const request = { key, ...deferred() };
    requests.push(request);
    return request.promise;
  }, (value, key) => applied.push({ value, key }), error => errors.push(error.message));
  return { scope, read, requests, applied, errors };
}

test("course previews apply only the latest request, including A to B to A", async () => {
  const h = harness();
  const oldA = h.read.refresh("A"), oldB = h.read.refresh("B"), latestA = h.read.refresh("A");
  h.requests[2].resolve("latest A preview");
  assert.equal(await latestA, true);
  h.requests[0].resolve("old A preview");
  h.requests[1].resolve("old B preview");
  assert.deepEqual(await Promise.all([oldA, oldB]), [false, false]);
  assert.deepEqual(h.applied, [{ value: "latest A preview", key: "A" }]);
  h.scope.dispose();
});

test("a pushed state or selection without a new read invalidates the pending result", async () => {
  const h = harness();
  const pending = h.read.refresh("schedule");
  h.read.invalidate();
  h.requests[0].resolve("old schedule before cache event");
  assert.equal(await pending, false);
  assert.deepEqual(h.applied, []);
  const next = h.read.refresh("schedule");
  h.requests[1].resolve("current schedule");
  assert.equal(await next, true);
  assert.deepEqual(h.applied, [{ value: "current schedule", key: "schedule" }]);
  h.scope.dispose();
});

test("disposal rejects late success and failure without publishing notices or further IPC", async () => {
  const h = harness();
  const success = h.read.refresh("config"), failure = h.read.refresh("stt");
  h.scope.dispose();
  h.requests[0].resolve("late settings");
  h.requests[1].reject(new Error("late error"));
  assert.deepEqual(await Promise.all([success, failure]), [false, false]);
  assert.equal(await h.read.refresh("after close"), false);
  assert.equal(h.requests.length, 2);
  assert.deepEqual(h.applied, []);
  assert.deepEqual(h.errors, []);
});

test("obsolete errors cannot clear a newer successful readiness check but current errors can retry", async () => {
  const h = harness();
  const older = h.read.refresh("config");
  const newer = h.read.refresh("config");
  h.requests[1].resolve("ready");
  assert.equal(await newer, true);
  h.requests[0].reject(new Error("obsolete settings failed"));
  assert.equal(await older, false);
  assert.deepEqual(h.errors, []);
  const current = h.read.refresh("config");
  h.requests[2].reject(new Error("current failure"));
  await assert.rejects(current, /current failure/);
  assert.deepEqual(h.errors, ["current failure"]);
  const retried = h.read.refresh("config");
  h.requests[3].resolve("ready after retry");
  assert.equal(await retried, true);
  h.scope.dispose();
});
