import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
const { expandLiveSurface, expandLiveSurfaceSave } = await loadTypeScript("src/lib/liveBoardTransport.ts");

function board(seed) {
  return { title: `板書 ${seed} 👩🏽‍💻`, schema_version: 1, normalized_by: "backend", layout: "grid",
    nodes: [{ id: "__proto__", label: "引用\"", detail: "日本語の全文\n".repeat(seed + 1), source_excerpt: "字幕の原文", external_source: "ref", parent_id: "", source_type: "lecture", kind: "concept", role: "main", node_type: "structure" }],
    edges: [{ from: "__proto__", to: "constructor", label: "関連" }] };
}
function wire(seed = 0) {
  return { whiteboard_table_version: 1, whiteboards: [board(seed), { title: "empty", nodes: [], edges: [] }],
    active: false, session_id: "fixture", update_revision: seed, finish_revision: seed, finish_phase: "saving_final",
    course: { course_name: "授業", day: 1, period: 2 }, started_at: "10:00:00", transcript_line_count: 10000,
    visible_lines: [{ text: "全文 👩🏽‍💻", at: "10:00:01" }], pending_from_line: 9999,
    next_summary_at_ms: null, summarizing: false,
    summaries: Array.from({ length: 32 }, (_, index) => ({ title: `段 ${index}`, range_label: "10:00-10:05", body: "本文の全文", line_count: index,
      terms: [{ term: "用語", explanation: "意味", source_excerpt: "原文", external_source: "ref" }], ...(index % 3 === 0 ? {} : { whiteboard_ref: index % 3 - 1 }) })) };
}

test("compact snapshots preserve every field and share carried board objects in 32 Unicode histories", () => {
  for (let seed = 0; seed < 32; seed++) {
    const input = wire(seed), original = JSON.stringify(input);
    const { whiteboard_table_version, whiteboards, ...expected } = input;
    expected.summaries = input.summaries.map(({ whiteboard_ref, ...chunk }) => whiteboard_ref === undefined ? chunk : { ...chunk, whiteboard: whiteboards[whiteboard_ref] });
    const page = expandLiveSurface(input);
    assert.deepEqual(page, expected);
    assert.equal(page.summaries[1].whiteboard, input.whiteboards[0]);
    assert.equal(page.summaries[1].whiteboard, page.summaries[4].whiteboard);
    assert.equal(page.summaries[2].whiteboard, input.whiteboards[1]);
    assert.equal("whiteboard" in page.summaries[0], false);
    assert.equal("whiteboard_ref" in page.summaries[1], false);
    assert.equal("whiteboards" in page, false);
    assert.equal("whiteboard_table_version" in page, false);
    assert.equal(page.visible_lines, input.visible_lines);
    assert.equal(page.summaries[1].terms, input.summaries[1].terms);
    assert.equal(JSON.stringify(input), original);
  }
});

test("separate replies retain their own versions and decode empty boards and saves without an interning cache", () => {
  const first = wire(1), second = JSON.parse(JSON.stringify(first));
  const a = expandLiveSurface(first), b = expandLiveSurface(second);
  assert.notEqual(a.summaries[1].whiteboard, b.summaries[1].whiteboard);
  const saved = { saved: true, path: "/fixture/授業.md", summary_markdown: "要約の全文", snapshot: first, todos_pending: true, suggested_todos: [{ title: "提出" }] };
  const original = JSON.stringify(saved), result = expandLiveSurfaceSave(saved);
  assert.equal(result.snapshot.summaries[1].whiteboard, first.whiteboards[0]);
  assert.equal(result.path, saved.path); assert.equal(result.summary_markdown, saved.summary_markdown);
  assert.equal(result.suggested_todos, saved.suggested_todos);
  assert.equal(JSON.stringify(saved), original);
  const empty = { ...first, summaries: [], whiteboards: [] };
  assert.deepEqual(expandLiveSurface(empty).summaries, []);
});

test("malformed table versions, boards and references fail without silently losing citations", () => {
  for (const ref of [-1, 2, NaN, Infinity, 0.5, "0", null, Number.MAX_SAFE_INTEGER + 1]) {
    const input = wire(); input.summaries[1].whiteboard_ref = ref;
    assert.throws(() => expandLiveSurface(input), /参照/);
  }
  for (const changed of [ { whiteboard_table_version: 2 }, { whiteboards: null }, { summaries: null }, { whiteboards: [null] }, { whiteboards: [[]] }, { whiteboards: [42] }, { summaries: [null] }, { summaries: [[]] }, { summaries: [{ whiteboard: board(0) }] } ]) {
    assert.throws(() => expandLiveSurface({ ...wire(), ...changed }), /形式/);
  }
});
