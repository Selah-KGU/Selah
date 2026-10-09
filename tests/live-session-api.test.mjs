import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const api = await loadTypeScript("src/lib/liveSessionApi.ts");
const course = (name) => ({ course_name: name, course_code: name, day: 1, period: 1,
  room: "101", teacher: "teacher", time_label: "9:00-10:00", is_free_note: false });
function compact(page) { return { ...page, whiteboard_table_version: 1, whiteboards: [] }; }
function environment(t, demo, responses = {}) {
  const oldStorage = globalThis.localStorage, oldWindow = globalThis.window;
  const values = new Map([["selah-demo-mode", demo ? "1" : "0"]]);
  const calls = [];
  globalThis.localStorage = {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };
  // Use the actual bundled Tauri invoke wrapper, with its native transport
  // replaced at the boundary. This test never sends commands to a running app.
  globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => {
    calls.push({ command, args });
    if (responses[command] instanceof Error) throw responses[command];
    if (command in responses) return responses[command];
    return command === "live_generate_overall_summary" ? "summary" : undefined;
  } } };
  t.after(() => {
    if (oldStorage === undefined) delete globalThis.localStorage;
    else globalThis.localStorage = oldStorage;
    if (oldWindow === undefined) delete globalThis.window;
    else globalThis.window = oldWindow;
  });
  return { values, calls };
}

test("startup reads only native activity and preserves errors without a full-history fallback", async t => {
  const { calls } = environment(t, false, { live_has_active_session: true });
  assert.equal(await api.liveHasActiveSession(), true);
  assert.deepEqual(calls, [{ command: "live_has_active_session", args: {} }]);
});

test("inactive and failed activity reads remain distinct", async t => {
  const responses = { live_has_active_session: false };
  const { calls } = environment(t, false, responses);
  assert.equal(await api.liveHasActiveSession(), false);
  responses.live_has_active_session = new Error("owner unavailable");
  await assert.rejects(api.liveHasActiveSession(), /owner unavailable/);
  assert.deepEqual(calls.map(call => call.command), ['live_has_active_session', 'live_has_active_session']);
});

test("demo activity follows the stored session without native IPC or losing its full history", async t => {
  const { calls } = environment(t, true);
  assert.equal(await api.liveHasActiveSession(), false);
  const started = await api.liveStartSession(course('demo'));
  for (let i = 0; i < 150; i++) await api.liveAppendTranscript(`完整 ${i}`);
  const full = await api.liveGetSession();
  assert.equal(await api.liveHasActiveSession(), true);
  assert.deepEqual(await api.liveGetSession(), full);
  await api.liveCancelSession(started.session_id);
  assert.equal(await api.liveHasActiveSession(), false);
  assert.deepEqual(calls, []);
});

test("page recovery, course preview and start use native surface commands without full-history reads", async t => {
  const page = { active: true, session_id: "fixture", transcript_line_count: 10000,
    visible_lines: [{ text: "complete tail 🌕", at: "10:00:00" }], pending_from_line: 9999, summaries: [] };
  const { calls } = environment(t, false, { live_get_surface_compact: compact(page), live_peek_day_surface_compact: compact(page), live_start_surface_compact: compact(page) });
  const input = course("A");
  assert.deepEqual(await api.liveGetSurface(), page);
  assert.deepEqual(await api.livePeekDaySurface(input), page);
  assert.deepEqual(await api.liveStartSurface(input), page);
  assert.deepEqual(calls, [
    { command: "live_get_surface_compact", args: {} },
    { command: "live_peek_day_surface_compact", args: { course: input } },
    { command: "live_start_surface_compact", args: { course: input } },
  ]);
});

test("failed surface reads and start remain errors and never retry a mutation through another command", async t => {
  const { calls } = environment(t, false, { live_get_surface_compact: new Error("read failed"),
    live_peek_day_surface_compact: new Error("preview failed"), live_start_surface_compact: new Error("start failed") });
  await assert.rejects(api.liveGetSurface(), /read failed/);
  await assert.rejects(api.livePeekDaySurface(course("A")), /preview failed/);
  await assert.rejects(api.liveStartSurface(course("A")), /start failed/);
  assert.equal(calls.length, 3);
  assert.deepEqual(calls.map(item => item.command), ["live_get_surface_compact", "live_peek_day_surface_compact", "live_start_surface_compact"]);
});

test("native surface finish forwards ownership once and returns the projected save reply", async t => {
  const saved = { saved: true, path: "/fixture/授業.md", summary_markdown: "全体要約 🌕",
    snapshot: { active: false, session_id: "recording-a", transcript_line_count: 10000, visible_lines: [], summaries: [] },
    todos_pending: true };
  const { calls } = environment(t, false, { live_finish_surface_compact: { ...saved, snapshot: compact(saved.snapshot) } });
  assert.deepEqual(await api.liveFinishSurface("recording-a"), saved);
  assert.deepEqual(calls, [{ command: "live_finish_surface_compact", args: { sessionId: "recording-a" } }]);
});

test("failed surface finish never falls back to a second finish request", async t => {
  const { calls } = environment(t, false, { live_finish_surface_compact: new Error("save failed") });
  await assert.rejects(api.liveFinishSurface("recording-a"), /save failed/);
  assert.deepEqual(calls, [{ command: "live_finish_surface_compact", args: { sessionId: "recording-a" } }]);
});

test("demo surface finish preserves ownership checks and returns only the display and preview", async t => {
  const { calls } = environment(t, true);
  const started = await api.liveStartSurface(course("A"));
  for (let i = 0; i < 150; i++) await api.liveAppendTranscript(`全文 ${i} 👩🏽‍💻`);
  const before = await api.liveGetSession();
  await assert.rejects(api.liveFinishSurface("old-recording"), /切り替わりました/);
  assert.deepEqual(await api.liveGetSession(), before);
  const saved = await api.liveFinishSurface(started.session_id);
  assert.equal(saved.saved, true);
  assert.equal(saved.snapshot.active, false);
  assert.equal(saved.snapshot.session_id, started.session_id);
  assert.equal(saved.snapshot.transcript_line_count, 153);
  assert.deepEqual(saved.snapshot.visible_lines, before.transcript_lines.slice(-120));
  assert.equal("markdown" in saved, false);
  assert.equal("transcript_lines" in saved.snapshot, false);
  assert.equal(saved.summary_markdown, "全文 147 👩🏽‍💻 / 全文 148 👩🏽‍💻 / 全文 149 👩🏽‍💻");
  assert.equal((await api.liveGetSession()).active, false);
  assert.deepEqual(calls, []);
});

test("demo surface reads share the native projection while full demo records remain complete", async t => {
  const { calls } = environment(t, true);
  const input = course("A");
  const started = await api.liveStartSurface(input);
  for (let i = 0; i < 150; i++) await api.liveAppendTranscript(`全文 ${i} 中文 👩🏽‍💻`);
  const full = await api.liveGetSession();
  const view = await api.liveGetSurface();
  assert.equal(view.session_id, started.session_id);
  assert.equal(view.transcript_line_count, full.transcript_lines.length);
  assert.deepEqual(view.visible_lines, full.transcript_lines.slice(-120));
  assert.equal(view.visible_lines.length, 120);
  assert.equal("transcript_lines" in view, false);
  assert.deepEqual(await api.livePeekDaySurface(input), view);
  assert.equal((await api.livePeekDaySurface(course("B"))).transcript_line_count, 0);
  await assert.rejects(api.liveStartSurface(course("B")), /使用中/);
  const saved = await api.liveFinishSession(started.session_id);
  assert.equal(saved.snapshot.transcript_lines.length, 153);
  assert.match(saved.markdown, /全文 0 中文 👩🏽‍💻/);
  assert.match(saved.markdown, /全文 149 中文 👩🏽‍💻/);
  assert.deepEqual(calls, []);
});

test("native finish, cancel and overall-summary commands forward the requested recording ID", async t => {
  const { calls } = environment(t, false);
  await api.liveFinishSession("recording-a");
  await api.liveCancelSession("recording-b");
  await api.liveGenerateOverallSummary("recording-c");
  assert.deepEqual(calls, [
    { command: "live_finish_session", args: { sessionId: "recording-a" } },
    { command: "live_cancel_session", args: { sessionId: "recording-b" } },
    { command: "live_generate_overall_summary", args: { sessionId: "recording-c" } },
  ]);
});

test("demo recording IDs persist across reads and a completed result retains the saved transcript", async t => {
  const { calls } = environment(t, true);
  const started = await api.liveStartSession(course("A"));
  assert.ok(started.session_id);
  assert.equal((await api.liveGetSession()).session_id, started.session_id);
  await assert.rejects(api.liveStartSession(course("B")), /使用中/);
  const saved = await api.liveFinishSession(started.session_id);
  assert.equal(saved.saved, true);
  assert.equal(saved.snapshot.session_id, started.session_id);
  assert.equal(saved.snapshot.active, false);
  assert.deepEqual(saved.snapshot.transcript_lines, started.transcript_lines);
  assert.match(saved.markdown, /A/);
  assert.equal((await api.liveGetSession()).active, false);
  assert.deepEqual(calls, []);
});

test("old demo finish, cancel and summary requests cannot mutate a replacement recording", async t => {
  environment(t, true);
  const old = await api.liveStartSession(course("A"));
  await api.liveCancelSession(old.session_id);
  const replacement = await api.liveStartSession(course("B"));
  assert.notEqual(old.session_id, replacement.session_id);
  for (const operation of [api.liveFinishSession, api.liveCancelSession, api.liveGenerateOverallSummary]) {
    await assert.rejects(operation(old.session_id), /切り替わりました/);
    await assert.rejects(operation(""), /切り替わりました/);
  }
  assert.deepEqual(await api.liveGetSession(), replacement);
});

test("demo finish revalidates ownership after its asynchronous flush", async t => {
  environment(t, true);
  const old = await api.liveStartSession(course("A"));
  const pending = api.liveFinishSession(old.session_id).then(() => null, error => error);
  // Both operations run their synchronous mutation before the finisher's await
  // continuation: the finish request must not clear this newly started B.
  const canceled = api.liveCancelSession(old.session_id);
  const newRecording = api.liveStartSession(course("B"));
  await canceled;
  const replacement = await newRecording;
  assert.match((await pending).message, /切り替わりました/);
  assert.deepEqual(await api.liveGetSession(), replacement);
});

test("an old active demo snapshot receives one stable recording ID during migration", async t => {
  const { values } = environment(t, true);
  values.set("selah-demo-live-session", JSON.stringify({ active: true, course: course("legacy"),
    started_at: "2026-10-07T01:00:00Z", transcript_lines: [{ at: "10:00", text: "legacy speech" }],
    pending_lines: [], summaries: [] }));
  const first = await api.liveGetSession();
  const second = await api.liveGetSession();
  assert.ok(first.session_id);
  assert.equal(second.session_id, first.session_id);
  assert.equal(JSON.parse(values.get("selah-demo-live-session")).session_id, first.session_id);
  assert.equal(second.transcript_lines[0].text, "legacy speech");
  await api.liveCancelSession(first.session_id);
  assert.equal((await api.liveGetSession()).active, false);
});
