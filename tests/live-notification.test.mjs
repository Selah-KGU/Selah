import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { loadTypeScript } from "./load-typescript.mjs";
import { boardFixture } from "./fixtures/whiteboard-layout-cases.mjs";

const { applyLiveSessionNotification: apply } = await loadTypeScript("src/lib/views/live/liveNotification.ts");
const { applyLiveSessionUpdate: plain, mergeLiveSnapshot } = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const clone = value => JSON.parse(JSON.stringify(value));
const freeze = value => { if (value && typeof value === "object") { Object.values(value).forEach(freeze); Object.freeze(value); } return value; };
const chunk = (board, index = 0) => ({ title: `区間 ${index}`, range_label: "10:00-10:05", body: `全文 ${index} 🙂`, line_count: index,
  terms: [{ term: "講義", explanation: "完全な説明" }], ...(board === undefined ? {} : { whiteboard: board }) });
const state = (board = boardFixture()) => ({ active: true, session_id: "recording", update_revision: 10, course: null, started_at: null,
  transcript_line_count: 1000, visible_lines: [{ text: "完全な最後の字幕", at: "10:00" }], pending_from_line: 900,
  finish_phase: null, finish_revision: 0, next_summary_at_ms: 100, summarizing: false, summaries: [chunk(board)] });
const event = (current, changes = {}) => ({ whiteboard_delta_version: 1, update_revision: current.update_revision + 1,
  active: true, session_id: current.session_id, course: current.course, started_at: current.started_at,
  transcript_line_count: current.transcript_line_count, pending_line_count: 0, summary_count: current.summaries.length + 1,
  next_summary_at_ms: 200, summarizing: false, finish_phase: null, finish_revision: 0,
  latest_summary: { ...chunk(undefined, current.summaries.length), whiteboard_from_summary: current.summaries.length - 1 }, ...changes });

test("eight actual Rust capture wires restore the complete legacy update result", async () => {
  const fixtures = JSON.parse(await readFile("tests/fixtures/live-notification-wire.json", "utf8"));
  assert.equal(fixtures.length, 8);
  for (const fixture of fixtures) {
    freeze(fixture);
    const input = JSON.stringify(fixture);
    const actual = apply(fixture.current, fixture.notification);
    assert.deepEqual(clone(actual), clone(plain(fixture.current, fixture.legacy_update)));
    assert.equal(actual.needsResync, false);
    assert.equal(JSON.stringify(fixture), input);
    assert.equal("whiteboard_delta_version" in actual.snapshot, false);
    for (const chunk of actual.snapshot.summaries) assert.equal("whiteboard_from_summary" in chunk, false);
    if ("whiteboard_from_summary" in (fixture.notification.latest_summary ?? {})) {
      const reference = fixture.notification.latest_summary.whiteboard_from_summary;
      assert.equal(actual.snapshot.summaries.at(-1).whiteboard, fixture.current.summaries[reference].whiteboard);
      assert.equal("whiteboard" in fixture.notification.latest_summary, false);
    }
  }
});

test("768 generated references and new boards preserve all values, history and finish progress", () => {
  for (let seed = 1; seed <= 32; seed++) {
    let current = freeze(state(boardFixture({ count: seed + 3, edges: seed * 2, seed })));
    for (let index = 0; index < 24; index++) {
      const wire = event(current);
      let legacy;
      if (index % 5 === 0) {
        wire.latest_summary = chunk({ ...boardFixture({ count: seed + 3, edges: seed * 2, seed }), title: `新板書 ${index}` }, index);
        legacy = { ...wire };
      } else {
        const { whiteboard_from_summary: reference, ...latest } = wire.latest_summary;
        legacy = { ...wire, latest_summary: { ...latest, whiteboard: current.summaries[reference].whiteboard } };
      }
      delete legacy.whiteboard_delta_version;
      if (index % 7 === 0) { wire.finish_phase = legacy.finish_phase = "saving_final"; wire.finish_revision = legacy.finish_revision = index + 1; }
      freeze(wire); const input = JSON.stringify(wire);
      const actual = apply(current, wire);
      assert.deepEqual(clone(actual), clone(plain(current, legacy)));
      assert.equal(actual.needsResync, false);
      assert.equal(actual.snapshot.visible_lines, current.visible_lines);
      assert.equal(JSON.stringify(wire), input);
      assert.equal("whiteboard_delta_version" in actual.snapshot, false);
      assert.equal("whiteboard_from_summary" in actual.snapshot.summaries.at(-1), false);
      current = freeze(actual.snapshot);
    }
  }
});

test("a lost chunk applies ordered metadata and requests recovery without inventing a board", () => {
  const current = freeze(state());
  const missing = event(current, { update_revision: 20, summary_count: 3,
    latest_summary: { ...chunk(undefined, 2), whiteboard_from_summary: 1 } });
  const result = apply(current, freeze(missing));
  assert.equal(result.needsResync, true);
  assert.equal(result.snapshot.update_revision, 20);
  assert.equal(result.snapshot.summaries, current.summaries);
  assert.equal(result.snapshot.pending_from_line, 1000);
  const lostAgain = apply(result.snapshot, missing);
  assert.equal(lostAgain.snapshot, result.snapshot);
  assert.equal(lostAgain.needsResync, false);
  const read = { ...clone(current), update_revision: 19,
    summaries: [clone(current.summaries[0]), chunk(clone(current.summaries[0].whiteboard), 1), chunk(clone(current.summaries[0].whiteboard), 2)] };
  const recovered = mergeLiveSnapshot(result.snapshot, read);
  assert.equal(recovered.update_revision, 20);
  assert.equal(recovered.pending_from_line, 1000);
  assert.equal(recovered.summaries.length, 3);
  assert.ok(recovered.summaries.every(chunk => chunk.whiteboard === current.summaries[0].whiteboard));
  const next = apply(recovered, event(recovered));
  assert.equal(next.needsResync, false);
  assert.equal(next.snapshot.summaries.length, 4);
});

test("replacement, stale and duplicate notifications never resolve references against the wrong recording", () => {
  const current = freeze(state());
  const stale = event(current, { update_revision: 5, latest_summary: { whiteboard_from_summary: 9000 } });
  assert.equal(apply(current, stale).snapshot, current);
  assert.equal(apply(current, stale).needsResync, false);
  assert.equal(apply({ ...current, update_revision: 0 }, stale, 10).needsResync, false);
  const replacement = apply(current, event(current, { session_id: "next-recording", summary_count: 2 }));
  assert.equal(replacement.needsResync, true);
  assert.equal(replacement.snapshot.session_id, "next-recording");
  assert.deepEqual(replacement.snapshot.summaries, []);
  assert.deepEqual(replacement.snapshot.visible_lines, []);
  const appended = apply(current, event(current)).snapshot;
  const duplicate = apply(appended, event(current, { update_revision: 13 }));
  assert.equal(duplicate.needsResync, false);
  assert.equal(duplicate.snapshot.summaries, appended.summaries);
  assert.equal(duplicate.snapshot.update_revision, 13);
  const ended = apply(appended, { ...event(appended), active: false, session_id: null, latest_summary: undefined });
  assert.equal(ended.snapshot.active, false);
  assert.deepEqual(ended.snapshot.summaries, []);
});

test("invalid versions, references, conflicting inline boards and counts request recovery without poisoning state", () => {
  const current = freeze(state());
  for (const reference of [-1, 0.5, 1, 2, Number.MAX_SAFE_INTEGER + 1, NaN, Infinity, "0", null, undefined]) {
    const wire = event(current, { latest_summary: { ...chunk(undefined), whiteboard_from_summary: reference } });
    const result = apply(current, wire);
    assert.equal(result.snapshot, current);
    assert.equal(result.needsResync, true);
  }
  for (const changes of [
    { whiteboard_delta_version: 0 }, { whiteboard_delta_version: 2 }, { whiteboard_delta_version: "1" },
    { latest_summary: [] }, { latest_summary: 7 },
    { latest_summary: { ...chunk(null), whiteboard_from_summary: 0 } },
    { update_revision: NaN }, { transcript_line_count: -1 }, { pending_line_count: 1001 }, { summary_count: 0.5 },
  ]) {
    const result = apply(current, event(current, changes));
    assert.equal(result.snapshot, current);
    assert.equal(result.needsResync, true);
  }
  assert.equal(apply(current, null).needsResync, true);
  const missingVersion = event(current);
  delete missingVersion.whiteboard_delta_version;
  assert.equal(apply(current, missingVersion).snapshot, current);
  assert.equal(apply(current, missingVersion).needsResync, true);
});

test("legacy complete notifications and metadata-only events retain their existing behavior", () => {
  const current = freeze(state());
  for (const changes of [
    { latest_summary: chunk(boardFixture({ seed: 9 }), 1) },
    { summary_count: 1, latest_summary: undefined, summarizing: true },
    { summary_count: 2, latest_summary: undefined },
  ]) {
    const wire = event(current, changes), legacy = { ...wire };
    delete legacy.whiteboard_delta_version;
    assert.deepEqual(apply(current, legacy), plain(current, legacy));
    assert.deepEqual(apply(current, wire), plain(current, legacy));
  }
  const noBoard = { ...current, summaries: [chunk(null)] };
  const unresolved = apply(noBoard, event(noBoard));
  assert.equal(unresolved.needsResync, true);
  assert.equal(unresolved.snapshot.summaries, noBoard.summaries);
});
