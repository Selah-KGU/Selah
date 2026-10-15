import test from "node:test";
import assert from "node:assert/strict";
import { loadLiveStartOwner } from "./load-live-start-owner.mjs";

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

test("an older failed toggle cannot overwrite the latest translation preference", async () => {
  const f = await loadLiveStartOwner();
  const pending = Array.from({ length: 3 }, deferred);
  let request = 0;
  f.configure({ captionTranslation: () => pending[request++].promise });
  const first = f.toggleCaptionTranslation();
  const second = f.toggleCaptionTranslation();
  const third = f.toggleCaptionTranslation();
  pending[1].resolve();
  pending[2].resolve();
  await Promise.all([second, third]);
  pending[0].reject(new Error("obsolete request failed"));
  await first;
  assert.equal(f.state.value.captionTranslation, false);
  assert.equal(f.state.value.storedCaptionTranslation, "0");
  assert.equal(f.state.value.notice, null);
  f.dispose();
});

test("a current failed toggle restores its preference and explains the failure", async () => {
  const f = await loadLiveStartOwner();
  f.configure({ captionTranslation: async () => { throw new Error("offline"); } });
  await f.toggleCaptionTranslation();
  assert.equal(f.state.value.captionTranslation, true);
  assert.equal(f.state.value.storedCaptionTranslation, "1");
  assert.equal(f.state.value.notice.kind, "error");
  f.dispose();
});

test("a failed toggle from a closed page cannot rewrite the saved preference", async () => {
  const f = await loadLiveStartOwner();
  const pending = deferred();
  f.configure({ captionTranslation: () => pending.promise });
  const toggle = f.toggleCaptionTranslation();
  f.dispose();
  pending.reject(new Error("closed page"));
  await toggle;
  assert.equal(f.state.value.storedCaptionTranslation, "0");
  assert.equal(f.state.value.notice, null);
});
