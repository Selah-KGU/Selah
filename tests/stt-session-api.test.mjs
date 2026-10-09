import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from 'node:fs/promises';
import { loadTypeScript } from "./load-typescript.mjs";

const api = await loadTypeScript("src/lib/sttSessionApi.ts");
const state = (phase, caller = "live", liveId = "recording-a") => ({ phase, session_id: 12,
  owner: { caller, live_session_id: liveId } });
function nativeBoundary(t, reply) {
  const previous = globalThis.window;
  const calls = [];
  globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => {
    calls.push({ command, args });
    return reply(command, args);
  } } };
  t.after(() => {
    if (previous === undefined) delete globalThis.window;
    else globalThis.window = previous;
  });
  return calls;
}

test("one native read returns the coherent phase, microphone and recording owner", async t => {
  const expected = state("initializing");
  const calls = nativeBoundary(t, () => expected);
  assert.deepEqual(await api.getSttStreamState(), expected);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "stt_get_stream_state");
});

test('the native worker IPC object reaches the existing STT API without a JSON-string wrapper', async t => {
  const wire=JSON.parse(await readFile('tests/fixtures/status-read-wire.json','utf8'));
  const calls=nativeBoundary(t,()=>wire.stt_get_stream_state);
  assert.deepEqual(await api.getSttStreamState(),api.idleSttStreamState());
  assert.equal(calls[0].command,'stt_get_stream_state');assert.equal(calls.length,1);
});

test("Agent returns a borrowed LIVE microphone to the captured recording UUID", async t => {
  const borrowed = { caller: "live", live_session_id: "recording-a" };
  const calls = nativeBoundary(t, command => command === "stt_start_stream" ? borrowed : null);
  const previous = await api.startSttStream({ caller: "agent", live_session_id: null, input_session_id: "input-a" }, true);
  assert.deepEqual(previous, borrowed);
  await api.startSttStream(previous);
  assert.deepEqual(calls, [
    { command: "stt_start_stream", args: { caller: "agent", preempt: true, inputSessionId: "input-a" } },
    { command: "stt_start_stream", args: { caller: "live", preempt: false, liveSessionId: "recording-a" } },
  ]);
});

test("native shortcut ownership returns without inventing a LIVE recording ID", async t => {
  const owner = { caller: "native_agent", live_session_id: null };
  const calls = nativeBoundary(t, () => null);
  await api.startSttStream(owner);
  assert.deepEqual(calls[0].args, { caller: "native_agent", preempt: false });
});

test("LIVE recovery distinguishes initialization from capture and rejects old owners", () => {
  assert.equal(api.liveSttPhase(state("initializing"), "recording-a"), "initializing");
  assert.equal(api.liveSttPhase(state("listening"), "recording-a"), "listening");
  for (const phase of ["initializing", "listening", "stopping", "idle"]) {
    assert.equal(api.liveSttPhase(state(phase), "recording-b"), "idle");
    assert.equal(api.liveSttPhase(state(phase), null), "idle");
    assert.equal(api.liveSttPhase(state(phase, "agent", null), "recording-a"), "idle");
  }
});

test("a microphone remains owned while stopping but cannot appear actively recording", () => {
  const stopping = state("stopping");
  assert.equal(api.ownsSttStream(stopping, "live", "recording-a"), true);
  assert.equal(api.isSttStreamActive(stopping, "live", "recording-a"), false);
  assert.equal(api.liveSttPhase(stopping, "recording-a"), "idle");
  assert.equal(api.isSttStreamActive(state("initializing", "agent", null), "agent"), true);
  assert.equal(api.isSttStreamActive(state("listening", "agent", null), "agent"), true);
  assert.equal(api.isSttStreamActive(state("stopping", "agent", null), "agent"), false);
  assert.equal(api.ownsSttStream(api.idleSttStreamState(), "agent"), false);
});

test("state read failures remain errors so the view can retain its last reliable state", async t => {
  nativeBoundary(t, () => { throw new Error("native transport lost"); });
  await assert.rejects(api.getSttStreamState(), /native transport lost/);
});


test("Agent stop forwards the exact input identity instead of stopping every Agent", async t => {
  const calls = nativeBoundary(t, () => undefined);
  await api.stopSttStream({ caller: "agent", live_session_id: null, input_session_id: "input-a" });
  assert.deepEqual(calls, [{ command: "stt_stop_stream", args: { caller: "agent", inputSessionId: "input-a" } }]);
});
