import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { loadTypeScript } from "./load-typescript.mjs";

const disk = new Map();
globalThis.localStorage = {
  getItem: key => disk.get(key) ?? null,
  setItem: (key, value) => disk.set(key, value),
  removeItem: key => disk.delete(key),
};
globalThis.document = { documentElement: { setAttribute() {} } };
let batchHandler;
let commands = [];
let overrides = new Map();
globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => {
  commands.push({ command, args });
  if (overrides.has(command)) return overrides.get(command)(args);
  if (command === "get_frontend_cache_batch") return batchHandler(args);
  if (command === "get_data_cache") {
    const row = backend.get(args.key);
    return row ? JSON.stringify(row.data) : null;
  }
  if (command === "get_data_cache_updated_at") return backend.get(args.key)?.updatedAt ?? 42;
  if (command === "get_schedule_snapshot") return structuredClone(schedule);
  if (command === "backend_refresh_now") return [];
  return null;
} } };
const stores = await loadTypeScript("tests/fixtures/backend-cache.ts");
const { syncBackendManagedKeys: sync, getCached, getCacheStamp, invalidateCache,
  knownRawRevision, readRawCache, replaceCacheEntry, onCacheUpdate,
  cachedBackendFetch, refreshBackendManagedCache, refreshCache } = stores;

let backend, requests, scheduleRevision, schedule, scheduleContext;
test.beforeEach(() => {
  invalidateCache();
  backend = new Map();
  requests = [];
  commands = [];
  overrides = new Map();
  scheduleRevision = 1;
  scheduleContext = "day-one";
  schedule = { raw: { luna_counts: [] }, ai_result: null };
  batchHandler = args => {
    requests.push(args);
    const rows = args.queries.map(query => {
      const row = backend.get(query.key);
      const revision = row?.revision ?? 0;
      const unchanged = query.knownRevision === revision;
      return { key: query.key, revision, unchanged, updated_at: row?.updatedAt ?? 42,
        ...(!unchanged && row ? { json: JSON.stringify(row.data) } : {}) };
    });
    const liveRevision = backend.get("live_generated_todo")?.revision ?? 0;
    const stamp = `opaque:${scheduleRevision}:${liveRevision}:${scheduleContext}`;
    const unchanged = args.knownScheduleStamp === stamp;
    return { rows, schedule_revision: scheduleRevision, live_todo_revision: liveRevision,
      schedule_stamp: stamp,
      schedule_updated_at: 42, live_todo_updated_at: 42, schedule_unchanged: unchanged,
      ...(args.includeSchedule && !unchanged ? { schedule: structuredClone(schedule) } : {}) };
  };
});
const put = (key, data, revision) => backend.set(key, { data, revision });
const putFresh = (key, data, revision) => backend.set(key, { data, revision, updatedAt: Math.floor(Date.now() / 1000) });
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};

test("native batch replies update the cache once and unchanged replies retain parsed identity", async () => {
  const wire = JSON.parse(await readFile(new URL("./fixtures/cache-batch-reply-wire.json", import.meta.url), "utf8"));
  const responses = [wire.notification_batch, wire.notification_unchanged_batch];
  batchHandler = args => {
    requests.push(args);
    assert.equal(args.includeSchedule, false);
    return responses.shift();
  };
  const updates = [];
  const unsubscribe = onCacheUpdate("notifications", fresh => updates.push(fresh));
  try {
    await sync(["notifications"]);
    const row = wire.notification_batch.rows[0];
    const first = getCached("notifications");
    assert.deepEqual(first, JSON.parse(row.json));
    assert.equal(readRawCache("notifications"), first);
    assert.equal(knownRawRevision("notifications"), row.revision);
    assert.equal(getCacheStamp("notifications"), row.revision);
    await sync(["notifications"]);
    assert.equal(getCached("notifications"), first);
    assert.deepEqual(updates, [first]);
    assert.equal(requests[1].queries[0].knownRevision, row.revision);
    assert.equal(responses.length, 0);
  } finally { unsubscribe(); }
});

test("native schedule read errors retain the displayed snapshot and permit recovery", async () => {
  const errors = JSON.parse(await readFile(new URL("./fixtures/schedule-read-error-wire.json", import.meta.url), "utf8"));
  await sync(["schedule_data"]);
  const first = getCached("schedule_data");
  const stamp = getCacheStamp("schedule_data");
  const updates = [];
  const unsubscribe = onCacheUpdate("schedule_data", value => updates.push(value));
  const originalBatch = batchHandler;
  try {
    for (const wire of errors) {
      batchHandler = async () => { throw wire.batch_error; };
      await assert.rejects(sync(["schedule_data"]), error => error === wire.batch_error);
      assert.equal(getCached("schedule_data"), first);
      assert.equal(getCacheStamp("schedule_data"), stamp);
    }
    assert.deepEqual(updates, []);
    batchHandler = originalBatch;
    schedule.raw.luna_counts = [{ luna_id: "recovered", reports: 2 }];
    scheduleRevision++;
    await sync(["schedule_data"]);
    assert.deepEqual(getCached("schedule_data").raw.luna_counts, schedule.raw.luna_counts);
    assert.equal(updates.length, 1);
  } finally { unsubscribe(); }
});

test("100 concurrent cold reads share one backend batch and the same parsed value", async () => {
  putFresh("mail_inbox", [{ id: "fresh" }], 1);
  const values = await Promise.all(Array.from({ length: 100 }, () => cachedBackendFetch("mail_inbox")));
  assert.equal(new Set(values).size, 1);
  assert.equal(requests.length, 1);
  assert.equal(commands.filter(call => call.command === "get_data_cache").length, 0);
});

test("same-second SQLite updates replace a fresh localStorage snapshot on cold start", async () => {
  const old = [{ course_name: "Course", content_name: "old" }];
  const fresh = [{ course_name: "Course", content_name: "new" }];
  disk.set("selah_cache_luna_todo", JSON.stringify({ v: 1, data: old, ts: Date.now() }));
  putFresh("luna_todo", fresh, 2);
  assert.deepEqual(await cachedBackendFetch("luna_todo"), fresh);
});

test("a synchronous disk preview still gets validated against SQLite", async () => {
  disk.set("selah_cache_luna_updates", JSON.stringify({ v: 1, data: [{ id: "old" }], ts: Date.now() }));
  assert.deepEqual(getCached("luna_updates"), [{ id: "old" }]);
  putFresh("luna_updates", [{ id: "new" }], 2);
  assert.deepEqual(await cachedBackendFetch("luna_updates"), [{ id: "new" }]);
});

test("a response started before cache invalidation cannot restore cleared data", async () => {
  putFresh("mail_inbox", [{ id: "previous-user" }], 1);
  const started = deferred();
  const release = deferred();
  const realBatch = batchHandler;
  batchHandler = async args => {
    const response = realBatch(args);
    started.resolve();
    await release.promise;
    return response;
  };
  const pending = sync(["mail_inbox"]);
  await started.promise;
  invalidateCache();
  release.resolve();
  await pending;
  assert.equal(getCached("mail_inbox"), null);
  assert.equal(readRawCache("mail_inbox"), null);
});

test("an invalidation before the queued read starts cancels that old request", async () => {
  putFresh("mail_inbox", [{ id: "old" }], 1);
  const pending = sync(["mail_inbox"]);
  invalidateCache();
  await pending;
  assert.equal(getCached("mail_inbox"), null);
  assert.equal(requests.length, 0);
});

test("invalidating one key does not drop unrelated responses in the same batch", async () => {
  putFresh("mail_inbox", [{ id: "mail" }], 1);
  putFresh("luna_updates", [{ id: "luna" }], 2);
  const started = deferred(), release = deferred(), realBatch = batchHandler;
  batchHandler = async args => {
    const response = realBatch(args);
    started.resolve(); await release.promise; return response;
  };
  const pending = sync(["mail_inbox", "luna_updates"]);
  await started.promise;
  invalidateCache("mail_inbox");
  release.resolve(); await pending;
  assert.equal(getCached("mail_inbox"), null);
  assert.deepEqual(getCached("luna_updates"), [{ id: "luna" }]);
});

test("a canceled cold read rejects without restarting network work", async () => {
  putFresh("mail_inbox", [{ id: "old" }], 1);
  const started = deferred(), release = deferred(), realBatch = batchHandler;
  batchHandler = async args => {
    const response = realBatch(args);
    started.resolve(); await release.promise; return response;
  };
  const pending = cachedBackendFetch("mail_inbox");
  const rejected = assert.rejects(pending, /Cache invalidated/);
  await started.promise;
  invalidateCache();
  release.resolve(); await rejected;
  assert.equal(getCached("mail_inbox"), null);
  assert.equal(commands.filter(call => call.command === "backend_refresh_now").length, 0);
});

test("manual refreshes deduplicate network work and retain backend content versions", async () => {
  putFresh("mail_inbox", [{ id: "fresh" }], 8);
  const values = await Promise.all([refreshBackendManagedCache("mail_inbox"), refreshBackendManagedCache("mail_inbox")]);
  assert.equal(values[0], values[1]);
  assert.equal(commands.filter(call => call.command === "backend_refresh_now").length, 1);
  assert.equal(requests.length, 1);
  assert.equal(getCacheStamp("mail_inbox"), 8);
  await cachedBackendFetch("mail_inbox");
  assert.equal(requests.length, 1);
});

test("a forced refresh during an automatic refresh runs one forced follow-up", async () => {
  put("mail_inbox", [{ id: "stale" }], 1);
  const automatic = deferred(), started = deferred();
  overrides.set("backend_refresh_now", async args => {
    if (!args.force) { started.resolve(); await automatic.promise; }
    else putFresh("mail_inbox", [{ id: "forced" }], 2);
    return [];
  });
  assert.deepEqual(await cachedBackendFetch("mail_inbox"), [{ id: "stale" }]);
  await started.promise;
  const forced = Array.from({ length: 50 }, () => refreshBackendManagedCache("mail_inbox"));
  automatic.resolve();
  const results = await Promise.all(forced);
  assert.ok(results.every(value => value[0].id === "forced"));
  const calls = commands.filter(call => call.command === "backend_refresh_now");
  assert.deepEqual(calls.map(call => call.args.force), [false, true]);
});

test("an empty notifications parse keeps the preview and requests a forced retry", async () => {
  const preview = { entries: [{ id: "notice" }] };
  disk.set("selah_cache_notifications", JSON.stringify({ v: 1, data: preview, ts: Date.now() }));
  putFresh("notifications", { entries: [] }, 2);
  const retry = deferred();
  overrides.set("backend_refresh_now", async args => { retry.resolve(args); return []; });
  assert.deepEqual(await cachedBackendFetch("notifications"), preview);
  assert.equal((await retry.promise).force, true);
  // Let the background retry finish before the next test clears shared state.
  await refreshBackendManagedCache("notifications");
  assert.deepEqual(getCached("notifications"), preview);
});

test("an IPC failure retains the disk preview while network retry fails", async () => {
  const preview = [{ id: "offline-preview" }];
  disk.set("selah_cache_luna_updates", JSON.stringify({ v: 1, data: preview, ts: Date.now() }));
  batchHandler = async () => { throw new Error("WebView IPC unavailable"); };
  const retry = deferred();
  overrides.set("backend_refresh_now", async () => { retry.resolve(); throw new Error("offline"); });
  assert.deepEqual(await cachedBackendFetch("luna_updates"), preview);
  await retry.promise;
  assert.deepEqual(getCached("luna_updates"), preview);
});

test("invalidation during mail URL repair cannot restore generated raw tasks or write a repair", async () => {
  const prefix = "A".repeat(24), suffix = "Z".repeat(16);
  putFresh("luna_todo", [], 1);
  putFresh("detail_generated_todo", [{ id: "detail", title: "Task", source_url: `mail://${prefix}short${suffix}` }], 2);
  const started = deferred(), release = deferred();
  overrides.set("get_data_cache", async () => {
    started.resolve(); await release.promise;
    return JSON.stringify([{ id: `${prefix}full-middle${suffix}` }]);
  });
  const pending = sync(["luna_todo"]);
  await started.promise;
  invalidateCache();
  release.resolve(); await pending;
  assert.equal(getCached("luna_todo"), null);
  assert.equal(readRawCache("detail_generated_todo"), null);
  assert.equal(commands.filter(call => call.command === "save_data_cache").length, 0);
});

test("same-second content updates propagate, unchanged data retains identity without notification", async () => {
  const updates = [];
  const unsubscribe = onCacheUpdate("mail_inbox", fresh => updates.push(fresh));
  try {
    put("mail_inbox", [{ id: "first" }], 1);
    await sync(["mail_inbox"]);
    const first = getCached("mail_inbox");
    await sync(["mail_inbox"]);
    assert.equal(getCached("mail_inbox"), first);
    assert.equal(updates.length, 1);
    assert.equal(requests[1].queries[0].knownRevision, 1);
    put("mail_inbox", [{ id: "second" }], 2);
    await sync(["mail_inbox"]);
    assert.deepEqual(getCached("mail_inbox"), [{ id: "second" }]);
    assert.equal(updates.length, 2);
  } finally { unsubscribe(); }
});

test("an unversioned local replacement cannot masquerade as the backend revision", async () => {
  put("mail_inbox", [{ id: "backend" }], 1);
  await sync(["mail_inbox"]);
  replaceCacheEntry("mail_inbox", [{ id: "local" }]);
  assert.equal(knownRawRevision("mail_inbox"), null);
  assert.equal(getCacheStamp("mail_inbox"), null);
  await sync(["mail_inbox"]);
  assert.equal(requests[1].queries[0].knownRevision, null);
  assert.deepEqual(getCached("mail_inbox"), [{ id: "backend" }]);
});

test("a generic refresh also clears its old backend content claim", async () => {
  putFresh("mail_inbox", [{ id: "backend" }], 1);
  await sync(["mail_inbox"]);
  await refreshCache("mail_inbox", async () => [{ id: "local" }]);
  assert.equal(knownRawRevision("mail_inbox"), null);
  await sync(["mail_inbox"]);
  assert.equal(requests[1].queries[0].knownRevision, null);
  assert.deepEqual(getCached("mail_inbox"), [{ id: "backend" }]);
});

test("deleting generated tasks removes merged rows while preserving canonical Luna tasks", async () => {
  const canonical = [{ course_name: "Course", content_name: "Canonical", deadline: "", url: "luna://1" }];
  put("luna_todo", canonical, 1);
  put("live_generated_todo", [{ id: "live", title: "Live task", course_name: "Course" }], 2);
  put("detail_generated_todo", [{ id: "detail", title: "Detail task", course_name: "Course" }], 3);
  await sync(["luna_todo"]);
  assert.equal(getCached("luna_todo").length, 3);
  backend.delete("live_generated_todo");
  backend.delete("detail_generated_todo");
  await sync(["luna_todo"]);
  assert.deepEqual(getCached("luna_todo"), canonical);
  assert.deepEqual(readRawCache("live_generated_todo"), []);
  assert.deepEqual(readRawCache("detail_generated_todo"), []);
  backend.delete("luna_todo");
  await sync(["luna_todo"]);
  assert.deepEqual(getCached("luna_todo"), []);
});

test("a changed schedule revision refreshes counts even when the timestamp is unchanged", async () => {
  await sync(["schedule_data"]);
  const first = getCached("schedule_data");
  await sync(["schedule_data"]);
  assert.equal(getCached("schedule_data"), first);
  schedule.raw.luna_counts = [{ luna_id: "course", reports: 2 }];
  scheduleRevision++;
  await sync(["schedule_data"]);
  assert.deepEqual(getCached("schedule_data").raw.luna_counts, [{ luna_id: "course", reports: 2 }]);
  assert.equal(getCacheStamp("schedule_data"), "opaque:2:0:day-one");
});

test("a backend context change invalidates the schedule without a SQLite revision change", async () => {
  await sync(["schedule_data"]);
  scheduleContext = "day-two:ai-expired";
  schedule.ai_stale = true;
  await sync(["schedule_data"]);
  assert.equal(getCached("schedule_data").ai_stale, true);
  assert.equal(getCacheStamp("schedule_data"), "opaque:1:0:day-two:ai-expired");
});

test("UI and database cache aliases share the same content version", async () => {
  put("exam_timetable", { entries: [{ course_name: "Course" }] }, 7);
  await sync(["exams"]);
  await sync(["exams"]);
  assert.equal(requests[1].queries[0].key, "exam_timetable");
  assert.equal(requests[1].queries[0].knownRevision, 7);
});
