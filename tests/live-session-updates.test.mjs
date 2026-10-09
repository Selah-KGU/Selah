import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { liveSurfaceSnapshot, applyLiveSessionUpdate, applyTranscriptDelta, mergeLiveSnapshot } = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const line = (i) => ({ text: `line ${i}`, at: "10:00:00" });
const chunk = (i) => ({ title: `chunk ${i}`, body: `summary ${i}`, line_count: 3, whiteboard: { nodes: [{ id: `node ${i}` }] } });
const fullSession = (count = 0) => ({
  update_revision: 10, session_id: "recording-a", active: true,
  course: { course_name: "lecture" }, started_at: "2026-10-07 10:00:00",
  transcript_lines: Array.from({ length: count }, (_, i) => line(i + 1)),
  pending_lines: Array.from({ length: count }, (_, i) => line(i + 1)), summaries: [],
  summarizing: false, next_summary_at_ms: 100, finish_phase: null, finish_revision: 0,
});
const session = (count = 0) => liveSurfaceSnapshot(fullSession(count));
const update = (changes = {}) => ({
  update_revision: 11, session_id: "recording-a", active: true,
  course: { course_name: "lecture" }, started_at: "2026-10-07 10:00:00",
  next_summary_at_ms: 200, summarizing: false, finish_phase: null, finish_revision: 0,
  transcript_line_count: 0, pending_line_count: 0, summary_count: 0,
  ...changes,
});

test("metadata alone keeps all transcript, pending and board references without recovery", () => {
  const current = { ...session(1000), summaries: [chunk(1)] };
  const result = applyLiveSessionUpdate(current, update({ transcript_line_count: 1000, pending_line_count: 1000, summary_count: 1, summarizing: true }));
  assert.equal(result.needsResync, false);
  assert.equal(result.snapshot.visible_lines, current.visible_lines);
  assert.equal(result.snapshot.pending_from_line, current.pending_from_line);
  assert.equal("pending_lines" in result.snapshot, false);
  assert.equal(result.snapshot.summaries, current.summaries);
  assert.equal(result.snapshot.summaries[0].whiteboard, current.summaries[0].whiteboard);
  assert.equal(result.snapshot.summarizing, true);
});

test("one new chunk consumes only the covered prefix and preserves speech appended during AI", () => {
  const current = session(5);
  const latest = chunk(1);
  const result = applyLiveSessionUpdate(current, update({ transcript_line_count: 3, pending_line_count: 0, summary_count: 1, latest_summary: latest }));
  assert.equal(result.needsResync, false);
  assert.equal(result.snapshot.visible_lines, current.visible_lines);
  assert.equal(result.snapshot.pending_from_line, 3);
  assert.equal(result.snapshot.summaries[0], latest);
  const duplicate = applyLiveSessionUpdate(result.snapshot, update({ transcript_line_count: 3, pending_line_count: 0, summary_count: 1, latest_summary: latest }));
  assert.equal(duplicate.snapshot, result.snapshot);
  assert.equal(duplicate.needsResync, false);
});

test("a missing earlier chunk requests full recovery without inserting the latest at the wrong index", () => {
  const current = session(3);
  const result = applyLiveSessionUpdate(current, update({ transcript_line_count: 3, summary_count: 2, latest_summary: chunk(2) }));
  assert.equal(result.needsResync, true);
  assert.equal(result.snapshot.summaries, current.summaries);
  assert.equal(result.snapshot.visible_lines, current.visible_lines);
});

test("a lost summary delta is detected by a later metadata-only notification", () => {
  const result = applyLiveSessionUpdate(session(3), update({ transcript_line_count: 3, summary_count: 1 }));
  assert.equal(result.needsResync, true);
  assert.deepEqual(result.snapshot.summaries, []);
});

test("older recovery fills missing speech while keeping the newer consumed prefix and board", () => {
  const latest = chunk(1);
  const result = applyLiveSessionUpdate(session(2), update({ transcript_line_count: 5, pending_line_count: 2, summary_count: 1, latest_summary: latest }));
  assert.equal(result.needsResync, true);
  assert.equal(result.snapshot.pending_from_line, 3);
  const older = { ...fullSession(5), update_revision: 9, next_summary_at_ms: 50 };
  const recovered = mergeLiveSnapshot(result.snapshot, older);
  assert.equal(recovered.update_revision, 11);
  assert.equal(recovered.next_summary_at_ms, 200);
  assert.deepEqual(recovered.visible_lines, older.transcript_lines);
  assert.equal(recovered.pending_from_line, 3);
  assert.equal(recovered.summaries[0], latest);
  const tail = applyTranscriptDelta(recovered, { session_id: "recording-a", line_count: 6, line: line(6) });
  assert.equal(tail.snapshot.pending_from_line, 3);
  assert.equal(tail.snapshot.transcript_line_count, 6);
});

test("recovery deltas inside an already-consumed prefix do not re-add pending speech", () => {
  const waiting = applyLiveSessionUpdate(session(2), update({ transcript_line_count: 5, pending_line_count: 2 })).snapshot;
  const inside = applyTranscriptDelta(waiting, { session_id: "recording-a", line_count: 3, line: line(3) });
  assert.equal(inside.needsResync, false);
  assert.equal(inside.snapshot.pending_from_line, 3);
  assert.equal(inside.snapshot.transcript_line_count, 3);
  const next = applyTranscriptDelta(inside.snapshot, { session_id: "recording-a", line_count: 4, line: line(4) });
  assert.equal(next.snapshot.pending_from_line, 3);
  assert.equal(next.snapshot.transcript_line_count, 4);
});

test("old inactive notifications and old full reads cannot clear or revive a replacement recording", () => {
  const replacement = { ...session(2), session_id: "recording-b", update_revision: 20 };
  const oldEnd = update({ active: false, session_id: null, update_revision: 19 });
  assert.equal(applyLiveSessionUpdate(replacement, oldEnd).snapshot, replacement);
  assert.equal(mergeLiveSnapshot(replacement, fullSession(4)), replacement);
  assert.equal(applyLiveSessionUpdate(replacement, update({ update_revision: 18 })).snapshot, replacement);
  const newerRead = { ...fullSession(1), session_id: "recording-c", update_revision: 21 };
  assert.deepEqual(mergeLiveSnapshot(replacement, newerRead), liveSurfaceSnapshot(newerRead));
});

test("revision watermark survives replacing the displayed record with an unordered course preview", () => {
  const preview = { ...session(5), active: false, session_id: null, update_revision: 0 };
  assert.equal(applyLiveSessionUpdate(preview, update({ update_revision: 19 }), 20).snapshot, preview);
  assert.equal(mergeLiveSnapshot(preview, fullSession(8), 20), preview);
  const next = applyLiveSessionUpdate(preview, update({ update_revision: 21, session_id: "recording-b", transcript_line_count: 3 }), 20);
  assert.equal(next.snapshot.session_id, "recording-b");
  assert.deepEqual(next.snapshot.visible_lines, []);
  assert.deepEqual(next.snapshot.summaries, []);
  assert.equal(next.needsResync, true);
});

test("an event before the start RPC returns can merge that response without rolling back metadata", () => {
  const empty = { ...session(), active: false, session_id: null, update_revision: 0 };
  const notified = applyLiveSessionUpdate(empty, update({ transcript_line_count: 5, pending_line_count: 0, summary_count: 1 })).snapshot;
  const startResponse = { ...fullSession(5), update_revision: 10, pending_lines: [], summaries: [chunk(1)], next_summary_at_ms: 50 };
  const merged = mergeLiveSnapshot(notified, startResponse);
  assert.equal(merged.next_summary_at_ms, 200);
  assert.equal(merged.update_revision, 11);
  assert.equal(merged.transcript_line_count, 5);
  assert.equal(merged.pending_from_line, 5);
  assert.equal(merged.summaries, startResponse.summaries);
});

test("inactive notifications clear active state and then preserve the completed record for display", () => {
  const inactive = update({ active: false, session_id: null, update_revision: 20 });
  const stopped = applyLiveSessionUpdate(session(3), inactive).snapshot;
  assert.equal(stopped.active, false);
  assert.equal(stopped.session_id, null);
  assert.deepEqual(stopped.visible_lines, []);
  const completed = { ...session(3), active: false, update_revision: 19, summaries: [chunk(1)] };
  const displayed = applyLiveSessionUpdate(completed, inactive).snapshot;
  assert.equal(displayed.active, false);
  assert.equal(displayed.visible_lines, completed.visible_lines);
  assert.equal(displayed.summaries, completed.summaries);
  const emptyRead = { ...fullSession(), active: false, session_id: null, course: null, update_revision: 21 };
  assert.equal(mergeLiveSnapshot(displayed, emptyRead).visible_lines, completed.visible_lines);
});

test("a pending update cannot regress newer save progress or recreate its summary timer", () => {
  const saving = { ...session(3), finish_phase: "saving_final", finish_revision: 4, next_summary_at_ms: null };
  const result = applyLiveSessionUpdate(saving, update({ transcript_line_count: 3, summary_count: 1, latest_summary: chunk(1), finish_phase: "saving_record", finish_revision: 2 }));
  assert.equal(result.snapshot.finish_phase, "saving_final");
  assert.equal(result.snapshot.finish_revision, 4);
  assert.equal(result.snapshot.next_summary_at_ms, null);
  assert.equal(result.snapshot.summaries.length, 1);
});

test("invalid counts or revision request recovery without inventing history or poisoning the watermark", () => {
  const current = session();
  for (const invalid of [
    { pending_line_count: 1 }, { transcript_line_count: -1 }, { summary_count: 0.5 },
    { update_revision: NaN }, { session_id: null },
  ]) {
    const result = applyLiveSessionUpdate(current, update(invalid));
    assert.equal(result.snapshot, current);
    assert.equal(result.needsResync, true);
  }
});
