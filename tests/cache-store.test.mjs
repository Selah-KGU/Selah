import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

// The cache layer can initialize without DOM, theme or authentication stores.
const disk = new Map();
globalThis.localStorage = {
  getItem: key => disk.get(key) ?? null,
  setItem: (key, value) => disk.set(key, value),
  removeItem: key => disk.delete(key),
};
const { cachedFetch, refreshCache, getCached, invalidateCache, onCacheUpdate } = await loadTypeScript("src/lib/cacheStore.ts");
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
test.beforeEach(() => invalidateCache());

test("a generic fetch cannot restore a cleared cache", async () => {
  const response = deferred();
  const pending = cachedFetch("favorites", () => response.promise);
  const rejected = assert.rejects(pending, /Cache invalidated/);
  invalidateCache();
  response.resolve(["old"]);
  await rejected;
  assert.equal(getCached("favorites"), null);
});

test("finishing an invalidated fetch does not remove the newer in-flight request", async () => {
  const oldResponse = deferred(), newResponse = deferred();
  const old = cachedFetch("favorites", () => oldResponse.promise);
  const rejected = assert.rejects(old, /Cache invalidated/);
  invalidateCache("favorites");
  const fresh = cachedFetch("favorites", () => newResponse.promise);
  oldResponse.resolve(["old"]);
  await rejected;
  let extraReads = 0;
  const duplicate = cachedFetch("favorites", async () => { extraReads++; return ["duplicate"]; });
  newResponse.resolve(["fresh"]);
  const [a, b] = await Promise.all([fresh, duplicate]);
  assert.equal(extraReads, 0);
  assert.equal(a, b);
});

test("callers waiting on a failed background refresh share one recovery fetch", async () => {
  const response = deferred();
  const warn = console.warn;
  console.warn = () => {};
  try {
    const background = refreshCache("favorites", () => response.promise);
    let calls = 0;
    const fetcher = async () => { calls++; return ["recovered"]; };
    const waiting = Array.from({ length: 100 }, () => cachedFetch("favorites", fetcher));
    response.reject(new Error("temporary offline"));
    await background;
    const values = await Promise.all(waiting);
    assert.equal(calls, 1);
    assert.equal(new Set(values).size, 1);
  } finally { console.warn = warn; }
});

test("a canceled background refresh completes without writing or notifying", async () => {
  let notifications = 0;
  const unsubscribe = onCacheUpdate("favorites", () => notifications++);
  const response = deferred();
  const pending = refreshCache("favorites", () => response.promise);
  invalidateCache();
  response.resolve(["old"]);
  await pending;
  assert.equal(getCached("favorites"), null);
  assert.equal(notifications, 0);
  unsubscribe();
});
