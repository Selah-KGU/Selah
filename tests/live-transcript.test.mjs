import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { liveSurfaceSnapshot, applyTranscriptDelta, isCurrentLiveSessionEvent, isCurrentLiveSttEvent, mergeLiveSnapshot } = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const line = (i) => ({ at: "10:00:00", text: `line ${i}` });
const fullSession = () => ({
  session_id: "recording-a", active: true, course: null, started_at: "2026-10-07 10:00:00",
  transcript_lines: [], pending_lines: [], summaries: [{ body: "summary", whiteboard: { nodes: [] } }],
});
const delta = (i, id = "recording-a") => ({ session_id: id, line_count: i, line: line(i) });

const session = () => liveSurfaceSnapshot(fullSession());

test("delayed STT captions, idle and errors are accepted only by their owning recording", () => {
  const current = session();
  assert.equal(isCurrentLiveSttEvent(current, { caller: "live", live_session_id: "recording-a" }), true);
  assert.equal(isCurrentLiveSttEvent(current, { caller: "live", live_session_id: "recording-old" }), false);
  assert.equal(isCurrentLiveSttEvent(current, { caller: "agent", live_session_id: "recording-a" }), false);
  assert.equal(isCurrentLiveSttEvent(current, { caller: "live" }), false);
  assert.equal(isCurrentLiveSttEvent({ ...current, active: false }, { caller: "live", live_session_id: "recording-a" }), false);
});

test("summary errors arriving after cancel or restart cannot affect the next recording", () => {
  const current = session();
  assert.equal(isCurrentLiveSessionEvent(current, { session_id: "recording-a" }), true);
  assert.equal(isCurrentLiveSessionEvent({ ...current, session_id: "recording-b" }, { session_id: "recording-a" }), false);
  assert.equal(isCurrentLiveSessionEvent({ ...current, active: false }, { session_id: "recording-a" }), false);
  assert.equal(isCurrentLiveSessionEvent(current, {}), false);
  assert.equal(isCurrentLiveSttEvent({ ...current, session_id: undefined }, { caller: "live" }), false);
});

test("1000 speech deltas retain summaries and exact counts with the visible window", () => {
  let snapshot = session();
  const summaries = snapshot.summaries;
  const board = summaries[0].whiteboard;
  for (let i = 1; i <= 1000; i++) {
    const result = applyTranscriptDelta(snapshot, delta(i));
    assert.equal(result.needsResync, false);
    snapshot = result.snapshot;
    assert.equal(snapshot.summaries, summaries);
    assert.equal(snapshot.summaries[0].whiteboard, board);
    assert.equal(snapshot.transcript_line_count, i);
    assert.deepEqual(snapshot.visible_lines, Array.from({ length: Math.min(i, 120) }, (_, j) => line(Math.max(1, i - 119) + j)));
  }
});

test("duplicate deltas and previous-session deltas cannot duplicate or contaminate speech", () => {
  const first = applyTranscriptDelta(session(), delta(1)).snapshot;
  assert.equal(applyTranscriptDelta(first, delta(1)).snapshot, first);
  assert.equal(applyTranscriptDelta(first, delta(2, "old-session")).snapshot, first);
});

test("separate repeated utterances remain visible while replaying a delta is ignored", () => {
  let snapshot = session();
  for (let count = 1; count <= 3; count++) {
    const update = { session_id: "recording-a", line_count: count,
      seq: count * 4, line: { at: "10:00:00", text: "はい。" } };
    snapshot = applyTranscriptDelta(snapshot, update).snapshot;
    assert.equal(applyTranscriptDelta(snapshot, update).snapshot, snapshot);
  }
  assert.equal(snapshot.transcript_line_count, 3);
  assert.deepEqual(snapshot.visible_lines.map(line => line.text), ["はい。", "はい。", "はい。"]);
});

test("a missed event requests recovery without inventing transcript lines", () => {
  const current = session();
  const result = applyTranscriptDelta(current, delta(3));
  assert.equal(result.snapshot, current);
  assert.equal(result.needsResync, true);
});

test("an older summary snapshot cannot roll back newer speech", () => {
  const current = applyTranscriptDelta(session(), delta(1)).snapshot;
  const incoming = { ...fullSession(), summaries: [...current.summaries, { body: "new summary" }], summarizing: false };
  const merged = mergeLiveSnapshot(current, incoming);
  assert.equal(merged.visible_lines, current.visible_lines);
  assert.equal(merged.pending_from_line, 0);
  assert.equal(merged.summaries, incoming.summaries);
});

test("metadata changes apply even when line and summary counts are unchanged", () => {
  const current = session();
  const merged = mergeLiveSnapshot(current, { ...fullSession(), summarizing: true, next_summary_at_ms: 123 });
  assert.equal(merged.summarizing, true);
  assert.equal(merged.next_summary_at_ms, 123);
  assert.equal(merged.summaries, current.summaries);
});

test("finish clears the session and restart accepts a different session", () => {
  const current = applyTranscriptDelta(session(), delta(1)).snapshot;
  const ended = { ...fullSession(), session_id: null, active: false, summaries: [] };
  assert.deepEqual(mergeLiveSnapshot(current, ended), liveSurfaceSnapshot(ended));
  const next = { ...fullSession(), session_id: "recording-b" };
  assert.deepEqual(mergeLiveSnapshot(current, next), liveSurfaceSnapshot(next));
});
