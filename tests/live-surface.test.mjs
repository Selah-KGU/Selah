import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
import { readFile } from "node:fs/promises";

const current = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const before = await loadTypeScript("tests/fixtures/live-transcript-before.ts");
const line = (i) => ({ at: "10:00:00", text: `授業 ${i} 中文・日本語 👩🏽‍💻\n\"引用\"` });
const chunk = (i) => ({ title: `要約 ${i}`, body: "段落 🌕", line_count: 40, whiteboard: { nodes: [{ id: `${i}`, title: "板書" }] } });
const full = (count = 0, changes = {}) => {
  const lines = Array.from({ length: count }, (_, i) => line(i + 1));
  return { active: true, session_id: "recording-a", course: { course_name: "授業" },
    started_at: "2026-10-08 10:00:00", update_revision: 1, finish_revision: 0,
    finish_phase: null, summarizing: false, next_summary_at_ms: 100,
    transcript_lines: lines, pending_lines: lines.slice(), summaries: [], ...changes };
};
// Independent expectation for the product's pre-existing 120-line display.
function projection(wire) {
  const { transcript_lines, pending_lines, ...metadata } = wire;
  return { ...metadata, transcript_line_count: transcript_lines.length,
    visible_lines: transcript_lines.slice(Math.max(0, transcript_lines.length - 120)),
    pending_from_line: wire.pending_from_line ?? transcript_lines.length - pending_lines.length };
}
function reachable(root) {
  const seen = new Set(); const todo = [root];
  while (todo.length) {
    const item = todo.pop();
    if (item == null || typeof item !== "object" || seen.has(item)) continue;
    seen.add(item); todo.push(...Object.values(item));
  }
  return seen;
}

test("a long full recovery projects only the display window without retaining hidden record arrays", () => {
  const wire = full(10000, { summaries: [chunk(1)] });
  wire.pending_lines = wire.transcript_lines.slice(9000);
  const original = JSON.stringify(wire);
  const view = current.liveSurfaceSnapshot(wire);
  assert.deepEqual(view, projection(wire));
  assert.equal(view.transcript_line_count, 10000);
  assert.equal(view.pending_from_line, 9000);
  const graph = reachable(view);
  assert.equal(graph.has(wire.transcript_lines), false);
  assert.equal(graph.has(wire.pending_lines), false);
  for (const item of wire.transcript_lines.slice(0, -120)) assert.equal(graph.has(item), false);
  assert.equal(view.visible_lines[0], wire.transcript_lines[9880]);
  assert.equal(view.summaries, wire.summaries);
  assert.equal(JSON.stringify(wire), original);
});

test("20000 accepted deltas bound display state and preserve previously captured versions", () => {
  let view = current.liveSurfaceSnapshot(full(0, { summaries: [chunk(1)] }));
  const summaries = view.summaries; const board = summaries[0].whiteboard;
  const captured = [];
  for (let i = 1; i <= 20000; i++) {
    const result = current.applyTranscriptDelta(view, { session_id: "recording-a", line_count: i, line: line(i) });
    assert.equal(result.needsResync, false);
    view = result.snapshot;
    assert.equal(view.transcript_line_count, i);
    assert.equal(view.visible_lines.length, Math.min(i, 120));
    assert.equal(view.visible_lines[0].text, line(Math.max(1, i - 119)).text);
    assert.equal(view.summaries, summaries); assert.equal(view.summaries[0].whiteboard, board);
    assert.equal("transcript_lines" in view, false); assert.equal("pending_lines" in view, false);
    if ([119, 120, 121, 4096].includes(i)) captured.push([view, JSON.stringify(view)]);
  }
  for (const [version, json] of captured) assert.equal(JSON.stringify(version), json);
  assert.deepEqual(view.visible_lines, Array.from({ length: 120 }, (_, i) => line(19881 + i)));
  assert.ok(JSON.stringify(view).length < 16000);
});

test("deltas, delayed summaries, gaps, recovery and restart match the original displayed result", () => {
  let old = full(); let view = current.liveSurfaceSnapshot(old); let revision = 1;
  const compare = () => assert.deepEqual(view, projection(old));
  const delta = (update) => {
    const was = before.applyTranscriptDelta(old, update);
    const now = current.applyTranscriptDelta(view, update);
    assert.equal(now.needsResync, was.needsResync);
    old = was.snapshot; view = now.snapshot; compare();
  };
  const read = (wire, minimum) => {
    old = before.mergeLiveSnapshot(old, wire, minimum);
    view = current.mergeLiveSnapshot(view, wire, minimum); compare();
  };
  const update = (changes = {}) => {
    const event = { update_revision: ++revision, session_id: old.session_id, active: old.active,
      course: old.course, started_at: old.started_at, next_summary_at_ms: 200,
      summarizing: false, finish_phase: null, finish_revision: old.finish_revision ?? 0,
      transcript_line_count: old.transcript_lines.length, pending_line_count: 0,
      summary_count: old.summaries.length, ...changes };
    const was = before.applyLiveSessionUpdate(old, event);
    const now = current.applyLiveSessionUpdate(view, event);
    assert.equal(now.needsResync, was.needsResync);
    old = was.snapshot; view = now.snapshot; compare();
  };
  for (let i = 1; i <= 2500; i++) {
    delta({ session_id: "recording-a", line_count: i, line: line(i) });
    if (i % 100 === 0) {
      update({ transcript_line_count: i - 20, pending_line_count: 5,
        summary_count: old.summaries.length + 1, latest_summary: chunk(i) });
      delta({ session_id: "recording-a", line_count: i, line: line(i) });
      delta({ session_id: "recording-old", line_count: i + 1, line: line(i + 1) });
    }
  }
  delta({ session_id: "recording-a", line_count: 2502, line: line(2502) });
  // New metadata may precede an old full read that contains the missing lines.
  update({ transcript_line_count: 2502, pending_line_count: 2,
    summary_count: old.summaries.length + 1, latest_summary: chunk(2502) });
  read(full(2502, { update_revision: 1, summaries: old.summaries }));
  delta({ session_id: "recording-a", line_count: 2503, line: line(2503) });
  read(full(2503, { finish_revision: 4, finish_phase: "saving_final", summaries: old.summaries, update_revision: ++revision }));
  read(full(2504, { update_revision: revision - 1, finish_revision: 2, finish_phase: "saving_record", summaries: old.summaries }));
  update({ active: false, session_id: null });
  read(full(2504, { active: false, update_revision: revision, summaries: [chunk(1)] }));
  update({ active: false, session_id: null });
  read(full(0, { active: false, session_id: null, course: null, update_revision: ++revision }));
  update({ session_id: "recording-b", active: true, transcript_line_count: 1 });
  read(full(1, { session_id: "recording-b", update_revision: revision }));
  // Cross-record stale reads cannot revive a stopped session.
  read(full(9000, { session_id: "recording-a", update_revision: 1 }));
});

test("metadata and save progress keep the visible window and board identities", async () => {
  const { applyLiveFinishProgress } = await loadTypeScript("src/lib/views/live/liveFinish.ts");
  const view = current.liveSurfaceSnapshot(full(10000, { summaries: [chunk(1)] }));
  const saving = applyLiveFinishProgress(view, { session_id: view.session_id, finish_revision: 4, finish_phase: "saving_final" });
  assert.equal(saving.visible_lines, view.visible_lines);
  const result = current.applyLiveSessionUpdate(saving, { update_revision: 2,
    session_id: view.session_id, active: true, course: view.course, started_at: view.started_at,
    next_summary_at_ms: 200, summarizing: true, finish_phase: "saving_record", finish_revision: 2,
    transcript_line_count: 10000, pending_line_count: 7, summary_count: 1 });
  assert.equal(result.snapshot.visible_lines, view.visible_lines);
  assert.equal(result.snapshot.summaries, view.summaries);
  assert.equal(result.snapshot.finish_phase, "saving_final");
  assert.equal(result.snapshot.pending_from_line, 9993);
  assert.equal(result.needsResync, false);
});

test("saved display preview retains only summary Markdown and releases the full snapshot graph", () => {
  const wire = full(10000);
  const markdown = "# 授業\n\n### 全体要約\n中文 👩🏽‍💻\n\n## 全文転写\n" + "全文 🌕\n".repeat(10000);
  const saved = { saved: true, markdown, snapshot: wire, todos_pending: true, path: "/tmp/fixture.md" };
  const preview = current.liveSavedPreview(saved);
  assert.deepEqual(preview, { summary_markdown: "中文 👩🏽‍💻" });
  assert.equal(reachable(preview).has(wire), false);
  assert.equal(reachable(preview).has(wire.transcript_lines), false);
  assert.equal(current.liveSavedPreview({ ...saved, saved: false }), null);
  assert.equal(wire.transcript_lines.length, 10000);
});

test("native and demo save summaries retain exactly the original preview text", async () => {
  const shared = await loadTypeScript("src/lib/liveSurfaceSnapshot.ts");
  const original = await loadTypeScript("tests/fixtures/live-save-summary-before.ts");
  const extraction = await loadTypeScript("src/lib/liveSavedSummary.ts");
  const fixtures = JSON.parse(await readFile("tests/fixtures/live-save-summary.json", "utf8"));
  for (const fixture of fixtures) {
    assert.equal(original.extractOverallSummary(fixture.markdown), fixture.summary, fixture.name);
    assert.equal(extraction.extractOverallSummary(fixture.markdown), fixture.summary, fixture.name);
    const wire = { saved: true, markdown: fixture.markdown, path: "/fixture/授業.md",
      snapshot: full(10000), suggested_todos: [{ title: "全文 🌕" }], todos_pending: true };
    const json = JSON.stringify(wire);
    const page = shared.liveSurfaceSaveResult(wire);
    assert.deepEqual(page, { saved: true, path: wire.path, snapshot: projection(wire.snapshot),
      summary_markdown: fixture.summary, suggested_todos: wire.suggested_todos, todos_pending: true });
    assert.deepEqual(current.liveSavedPreview(page), current.liveSavedPreview(wire));
    const graph = reachable(page);
    assert.equal(graph.has(wire.snapshot), false);
    assert.equal(graph.has(wire.snapshot.transcript_lines), false);
    assert.equal(graph.has(wire.snapshot.pending_lines), false);
    assert.equal(graph.has(wire.snapshot.transcript_lines[0]), false);
    assert.equal(JSON.stringify(wire), json);
  }
});

test("native thin and legacy full reads recover the same display across gaps and capture order", () => {
  let view = current.liveSurfaceSnapshot(full(130));
  for (const wire of [full(150, { update_revision: 2 }), full(160, { update_revision: 1 }),
    full(0, { active: false, session_id: null, course: null, update_revision: 3 }),
    full(10000, { session_id: "recording-b", update_revision: 4 }),
    full(9000, { update_revision: 2 })]) {
    const thin = projection(wire);
    const was = current.mergeLiveSnapshot(view, wire);
    const now = current.mergeLiveSnapshot(view, thin);
    assert.deepEqual(now, was);
    view = now;
  }
  assert.equal(view.session_id, "recording-b");
  assert.equal(view.transcript_line_count, 10000);
  const stale = current.liveSurfaceSnapshot(full(10, { update_revision: 1 }));
  const newer = current.liveSurfaceSnapshot(full(200, { update_revision: 2 }));
  const merged = current.mergeLiveSnapshot(stale, newer);
  assert.deepEqual(merged, newer);
  assert.equal(merged.visible_lines, newer.visible_lines);

  // A full-record recovery defines summary coverage through pending_lines.
  // An optional client prefix in that full payload must not override it.
  const recovered = full(200, { update_revision: 3, pending_from_line: 1 });
  recovered.pending_lines = recovered.transcript_lines.slice(175);
  const legacy = before.mergeLiveSnapshot(full(10), recovered);
  assert.deepEqual(current.mergeLiveSnapshot(stale, recovered), projection(legacy));
  assert.equal(current.mergeLiveSnapshot(stale, recovered).pending_from_line, 175);
});
