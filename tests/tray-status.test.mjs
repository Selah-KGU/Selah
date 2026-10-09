import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from 'node:fs/promises';
import { loadTypeScript } from "./load-typescript.mjs";
const flush = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const timetable = name => ({ raw: { kgc_entries_current: [{ day: 3, period: 3, name, is_cancelled: false }] } });
// Only the native communication boundary is replaced. These tests run the
// actual tray module, stores, timers and Tauri invoke/listen wrappers.
globalThis.localStorage = { getItem: key => key === "selah-theme" ? "light" : null, setItem() {}, removeItem() {} };
globalThis.document = { documentElement: { setAttribute() {} } };
globalThis.window = { __TAURI_INTERNALS__: { invoke: async () => null } };
const tray = await loadTypeScript("tests/fixtures/tray-status.ts");

function environment(t, options = {}) {
  const previousWindow = globalThis.window, previousStorage = globalThis.localStorage;
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval", "Date"], now: new Date(2026, 9, 7, 12).getTime() });
  const calls = [], registrations = [], unregistered = [], scheduleReads = [], metadataReads = [];
  const callbacks = new Map(), storage = new Map([["selah-theme", "light"]]);
  let callbackId = 0;
  globalThis.localStorage = { getItem: key => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  globalThis.window = {
    __TAURI_INTERNALS__: {
      transformCallback: callback => { callbacks.set(++callbackId, callback); return callbackId; },
      invoke: async (command, args) => {
        calls.push({ command, args });
        if (command === "plugin:event|listen") {
          const registration = { args, callback: callbacks.get(args.handler), ...deferred() };
          registrations.push(registration);
          if (!options.holdListeners) registration.resolve(args.handler);
          return registration.promise;
        }
        if (command === "get_schedule_snapshot") {
          const read = deferred(); scheduleReads.push(read);
          if (!options.holdSchedule) read.resolve(timetable("backend"));
          return read.promise;
        }
        if (command === "live_get_tray_status") {
          const read = deferred(); metadataReads.push(read);
          if (!options.holdMetadata) read.resolve({ active: true, listening: true, started_at: "2026-10-07 11:45:00" });
          return read.promise;
        }
        return null;
      },
    },
    __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (event, id) => unregistered.push({ event, id }) },
  };
  tray.invalidateCache();
  t.after(async () => {
    tray.stopTrayStatus();
    registrations.forEach(item => item.resolve(item.args.handler));
    scheduleReads.forEach(item => item.resolve(timetable("cleanup")));
    metadataReads.forEach(item => item.resolve({ active: false, listening: false, started_at: null }));
    await flush();
    tray.invalidateCache();
    globalThis.window = previousWindow; globalThis.localStorage = previousStorage;
  });
  return { calls, registrations, unregistered, scheduleReads, metadataReads,
    writes: () => calls.filter(call => call.command === "set_tray_status_items"),
    fire: (index, payload = {}) => registrations[index].callback({ payload, id: registrations[index].args.handler }),
    async tick(ms = 300) { t.mock.timers.tick(ms); await flush(); },
  };
}

test("the actual tray reads one metadata command and reuses managed timetable data", async t => {
  const h = environment(t);
  tray.replaceCacheEntry("schedule_data", timetable("cached course"));
  tray.startTrayStatus(); await h.tick();
  assert.deepEqual(h.registrations.map(item => item.args.event),
    ["live-session-updated", "stt-state", "live-surface-saved", "live-surface-compact-saved"]);
  assert.equal(h.scheduleReads.length, 0);
  assert.equal(h.metadataReads.length, 1);
  assert.equal(h.calls.some(call => ["live_get_session", "stt_get_stream_state", "stt_is_running"].includes(call.command)), false);
  assert.equal(h.writes()[0].args.items[0], "Live記録中 15分");
  assert.ok(h.writes()[0].args.items.some(item => item.includes("cached course")));
  for (let i = 0; i < 200; i++) h.fire(0);
  await h.tick();
  assert.equal(h.metadataReads.length, 2);
  assert.equal(h.writes().length, 1, "identical items reset the native cycling index");
});

test('native worker metadata objects and string errors preserve the paused tray and its last good display', async t => {
  const wire=JSON.parse(await readFile('tests/fixtures/status-read-wire.json','utf8'));
  const h=environment(t,{holdMetadata:true});
  tray.replaceCacheEntry('schedule_data',timetable('cached course'));
  tray.startTrayStatus();await h.tick();h.metadataReads[0].resolve(wire.live_active);await h.tick();
  assert.equal(h.writes()[0].args.items[0],'Live一時停止 15分');
  const written=h.writes().length;h.fire(0);await h.tick();h.metadataReads[1].reject(wire.live_error);await h.tick();
  assert.equal(h.writes().length,written);
  h.fire(0);await h.tick();h.metadataReads[2].resolve(wire.live_get_tray_status);await h.tick();
  assert.ok(!h.writes().at(-1).args.items.some(item=>item.startsWith('Live')));
});

test("a restarted tray disposes old late registrations and ignores old queued callbacks", async t => {
  const h = environment(t, { holdListeners: true });
  tray.replaceCacheEntry("schedule_data", timetable("current"));
  tray.startTrayStatus(); tray.stopTrayStatus(); tray.startTrayStatus();
  assert.equal(h.registrations.length, 8);
  await h.tick(); assert.equal(h.metadataReads.length, 1);
  h.registrations.slice(0, 4).forEach(item => item.resolve(item.args.handler));
  await flush();
  assert.deepEqual(h.unregistered.map(item => item.id).sort(), [1, 2, 3, 4]);
  h.fire(0); h.fire(1, { caller: "live" }); h.fire(2); h.fire(3);
  await h.tick(); assert.equal(h.metadataReads.length, 1);
  h.registrations.slice(4).forEach(item => item.resolve(item.args.handler)); await flush();
  h.fire(7); await h.tick(); assert.equal(h.metadataReads.length, 2);
});

test("old initial schedules and a current read superseded by a cache push cannot replace the current timetable", async t => {
  const h = environment(t, { holdSchedule: true });
  tray.startTrayStatus(); tray.stopTrayStatus(); tray.startTrayStatus();
  assert.equal(h.scheduleReads.length, 2);
  tray.replaceCacheEntry("schedule_data", timetable("fresh cache"));
  h.scheduleReads[0].resolve(timetable("obsolete old run"));
  h.scheduleReads[1].resolve(timetable("obsolete before push"));
  await flush(); await h.tick();
  const items = h.writes().at(-1).args.items;
  assert.ok(items.some(item => item.includes("fresh cache")));
  assert.ok(items.every(item => !item.includes("obsolete")));
});

test("a stopped run's late metadata cannot publish or change the new run's task status", async t => {
  const h = environment(t, { holdMetadata: true });
  tray.replaceCacheEntry("schedule_data", timetable("current"));
  tray.startTrayStatus(); await h.tick();
  tray.stopTrayStatus(); tray.startTrayStatus(); await h.tick();
  assert.equal(h.metadataReads.length, 2, "the new run waited on a closed run's metadata read");
  h.metadataReads[1].resolve({ active: true, listening: false, started_at: "2026-10-07 12:00:00" }); await flush();
  const task = structuredClone(tray.getTaskSnapshot().find(item => item.key === "tray_status"));
  const writes = structuredClone(h.writes());
  h.metadataReads[0].resolve({ active: true, listening: true, started_at: "2026-10-07 11:45:00" }); await flush();
  assert.deepEqual(h.writes(), writes);
  assert.deepEqual(tray.getTaskSnapshot().find(item => item.key === "tray_status"), task);
  assert.equal(h.writes().at(-1).args.items[0], "Live一時停止 0分");
});
