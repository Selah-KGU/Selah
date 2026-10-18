import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { build } from "esbuild";
import { resolve } from "node:path";

let instance = 0;
async function fixture(cache = {}) {
  const source = await readFile("src/lib/views/Timetable.svelte", "utf8");
  const load = source.match(/async function loadCachedExtras\(\)[\s\S]*?\n  }/)[0];
  const result = await build({
    stdin: { contents: `
      const cache = ${JSON.stringify(cache)};
      export const calls = [];
      export let examEntries = [], favoriteEntries = [];
      const favSlots = [], dayLabels = [];
      function debugLog() {}
      const console = { warn() {} };
      async function getDataCache(key) {
        calls.push(["cache", key]);
        return cache[key] ?? null;
      }
      async function cachedBackendFetch(key) {
        calls.push(["network", key]);
        throw new Error("KGC requires login");
      }
      async function fetchSyllabusFavorites() {
        calls.push(["network", "favorites"]);
        throw new Error("KGC requires login");
      }
      export ${load}
    `, loader: "ts" },
    bundle: true, write: false, format: "esm", platform: "node",
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}#timetable-${++instance}`);
}

test("a timetable without optional caches does not request KGC authentication", async () => {
  const h = await fixture();
  await h.loadCachedExtras();
  assert.deepEqual(h.calls, [["cache", "exam_timetable"], ["cache", "syllabus_favorites"]]);
  assert.deepEqual(h.examEntries, []);
  assert.deepEqual(h.favoriteEntries, []);
});

test("saved exams and favorites remain visible without a KGC session", async () => {
  const exam = { course_name: "試験", day: "月", period: 1 };
  const favorite = { course_title: "お気に入り", day_period: "火2" };
  const h = await fixture({
    exam_timetable: JSON.stringify({ entries: [exam] }),
    syllabus_favorites: JSON.stringify({ entries: [favorite] }),
  });
  await h.loadCachedExtras();
  assert.deepEqual(h.examEntries, [exam]);
  assert.deepEqual(h.favoriteEntries, [favorite]);
  assert.ok(h.calls.every(([kind]) => kind === "cache"));
});

test("a damaged optional cache does not trigger KGC login or prevent other overlays", async () => {
  const favorite = { course_title: "お気に入り", day_period: "火2" };
  const h = await fixture({
    exam_timetable: "{invalid",
    syllabus_favorites: JSON.stringify({ entries: [favorite] }),
  });
  await h.loadCachedExtras();
  assert.deepEqual(h.examEntries, []);
  assert.deepEqual(h.favoriteEntries, [favorite]);
  assert.ok(h.calls.every(([kind]) => kind === "cache"));
});

async function syncFixture(expired = false) {
  const source = await readFile("src/lib/api.ts", "utf8");
  const functions = ["syncScheduleData", "withSessionGuard"].map(name => {
    const match = source.match(new RegExp(`(?:export )?async function ${name}[^]*?\\n}\\n`));
    assert.ok(match, `Missing production function ${name}`);
    return match[0];
  }).join("\n");
  const result = await build({
    stdin: { contents: `
      import { universitySessionLifetime } from ${JSON.stringify(resolve("src/lib/sessionLifetime.ts"))};
      export const calls = [];
      let expired = ${expired}, mailRequestRevision = 0;
      const _isDemo = () => false;
      async function invoke(command) {
        calls.push(["invoke", command]);
        if (expired) { expired = false; throw new Error("Luna session expired"); }
        return { raw: { luna_courses: [{ name: "Updated Luna course" }], kgc_entries_current: [] } };
      }
      async function syncSession(service, trigger) {
        calls.push(["recovery", service, trigger]);
        return { results: [{ service, recovered: true }] };
      }
      ${functions}
    `, loader: "ts", resolveDir: process.cwd() },
    bundle: true, write: false, format: "esm", platform: "node",
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}#timetable-sync-${++instance}`);
}

test("manual timetable refresh returns Luna updates with no KGC session", async () => {
  const h = await syncFixture();
  const schedule = await h.syncScheduleData();
  assert.equal(schedule.raw.luna_courses[0].name, "Updated Luna course");
  assert.deepEqual(h.calls, [["invoke", "sync_schedule_data"]]);
});

test("manual timetable refresh recovers Luna alone after its session expires", async () => {
  const h = await syncFixture(true);
  const schedule = await h.syncScheduleData();
  assert.equal(schedule.raw.luna_courses[0].name, "Updated Luna course");
  assert.deepEqual(h.calls, [
    ["invoke", "sync_schedule_data"],
    ["recovery", "luna", "request_failure"],
    ["invoke", "sync_schedule_data"],
  ]);
});

async function fullRefreshFixture(failPrimary = false) {
  const source = await readFile("src/lib/api.ts", "utf8");
  const fn = source.match(/export async function refreshAllData[^]*?\n}\n/)[0];
  const result = await build({
    stdin: { contents: `
      export const calls = [];
      const _isDemo = () => false, get = value => value;
      const authState = { authenticated:true }, reloginInProgress = false, sessionExpired = false;
      export let status = { items:[], refreshingCount:0 };
      const cacheStatus = { update: fn => status = fn(status) };
      const getRefreshSequence = () => [{ key:'grades', label:'Grades', platform:'KGC' }];
      const isAiReady = async () => false, invalidateCache = () => {}, updateTask = () => {};
      export let completeKgc;
      async function syncScheduleData() {
        calls.push('luna');
        if (${failPrimary}) throw new Error('Luna unavailable');
        return { raw:{ luna_courses:[{ name:'Fresh Luna' }] } };
      }
      function replaceCacheEntry(key, value) { calls.push(['published', key, value.raw.luna_courses[0].name]); }
      function refreshBackendManagedCache(key) { calls.push(key); return new Promise(resolve => { completeKgc=resolve; }); }
      ${fn}
    `, loader:'ts' }, bundle:true, write:false, format:'esm', platform:'node',
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#full-${++instance}`);
}

test('full refresh publishes Luna while KGC is still waiting', async () => {
  const h = await fullRefreshFixture();
  const refresh = h.refreshAllData();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(h.calls, ['luna', ['published', 'schedule_data', 'Fresh Luna'], 'grades']);
  assert.equal(h.status.items.find(item => item.key === 'schedule_sync').status, 'done');
  assert.equal(h.status.items.find(item => item.key === 'grades').status, 'running');
  h.completeKgc({}); await refresh;
});

test('full refresh reports a real Luna failure instead of calling the cached grid fresh', async () => {
  const h = await fullRefreshFixture(true);
  const refresh = h.refreshAllData();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(h.calls, ['luna', 'grades']);
  assert.equal(h.status.items.find(item => item.key === 'schedule_sync').status, 'error');
  h.completeKgc({}); await refresh;
});
