import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
import { loadWhiteboardLayout } from "./load-whiteboard-layout.mjs";
import { boardFixture } from "./fixtures/whiteboard-layout-cases.mjs";

const after = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const before = await loadTypeScript("tests/fixtures/live-board-identity-before.ts");
const impl = process.env.SELAH_BOARD_IDENTITY_BEFORE ? before : after;
const clone = value => JSON.parse(JSON.stringify(value));
const freeze = value => {
  if (value && typeof value === "object") { Object.values(value).forEach(freeze); Object.freeze(value); }
  return value;
};
const chunk = (board, index = 0) => ({ title: `要約 ${index}`, range_label: `${index}:00`, body: `全文 🙂 ${index}`,
  line_count: index, terms: [{ term: "講義", explanation: "用語全文", source_excerpt: "出处", external_source: "https://example.invalid" }],
  ...(board === undefined ? {} : { whiteboard: board }) });
const session = (summaries = [], changes = {}) => ({ update_revision: 10, active: true, session_id: "recording-a",
  course: { course_name: "講義" }, started_at: "2026-10-08 10:00:00", transcript_line_count: 1000,
  visible_lines: [{ text: "最後の全文", at: "10:00" }], pending_from_line: 900,
  summaries, next_summary_at_ms: 100, summarizing: false, finish_phase: null, finish_revision: 0, ...changes });
const update = (current, latest, changes = {}) => ({ update_revision: current.update_revision + 1,
  active: true, session_id: current.session_id, course: current.course, started_at: current.started_at,
  transcript_line_count: 1000, pending_line_count: 0, summary_count: current.summaries.length + 1,
  latest_summary: latest, next_summary_at_ms: 200, summarizing: false, finish_phase: null, finish_revision: 0, ...changes });

// Assertions cover the actual event and recovery paths, not only a comparator.
test("32 separately parsed notifications retain one complete board and immutable metadata", () => {
  const original = freeze(boardFixture({ count: 96, edges: 192 }));
  let current = freeze(session([chunk(original)]));
  const initial = current, inputs = [];
  for (let index = 1; index < 32; index++) {
    const latest = freeze(chunk(clone(original), index));
    const event = freeze(update(current, latest));
    const wire = JSON.stringify(event);
    inputs.push(event);
    const result = impl.applyLiveSessionUpdate(current, event);
    assert.equal(result.needsResync, false);
    assert.deepEqual(result, before.applyLiveSessionUpdate(current, event));
    assert.equal(result.snapshot.summaries.at(-1).whiteboard, original);
    assert.equal(result.snapshot.summaries.at(-1).terms, latest.terms);
    assert.notEqual(result.snapshot.summaries.at(-1), latest);
    assert.equal(JSON.stringify(event), wire);
    assert.equal(result.snapshot.visible_lines, initial.visible_lines);
    current = freeze(result.snapshot);
  }
  assert.equal(new Set(current.summaries.map(row => row.whiteboard)).size, 1);
  assert.equal(new Set(inputs.map(event => event.latest_summary.whiteboard)).size, 31);
  const reachable = new Set(), pending = [current];
  while (pending.length) {
    const value = pending.pop();
    if (!value || typeof value !== "object" || reachable.has(value)) continue;
    reachable.add(value); pending.push(...Object.values(value));
  }
  for (const event of inputs) {
    assert.equal(reachable.has(event), false);
    assert.equal(reachable.has(event.latest_summary.whiteboard), false);
    for (const node of event.latest_summary.whiteboard.nodes) assert.equal(reachable.has(node), false);
  }
  assert.equal(initial.summaries.length, 1);
});

test("any changed field, unknown JSON data, or node/edge order keeps the new board", () => {
  const original = { ...boardFixture({ count: 8, edges: 12 }),
    future: JSON.parse('{"__proto__":{"source":"古い"},"constructor":"safe","data":[true,null,1,"🙂"]}') };
  original.nodes[0].source_excerpt = "原文";
  const current = freeze(session([chunk(freeze(original))]));
  const changes = [];
  for (const key of Object.keys(original)) changes.push(board => { delete board[key]; });
  for (const key of ["title", "layout", "normalized_by", "schema_version"]) changes.push(board => { board[key] = `${board[key]} changed`; });
  for (const key of Object.keys(original.nodes[0])) changes.push(board => { board.nodes[0][key] += "変更"; });
  for (const key of Object.keys(original.edges[0])) changes.push(board => { board.edges[0][key] += "変更"; });
  changes.push(board => board.nodes.reverse(), board => board.edges.reverse(),
    board => { board.future.__proto__.source = "新しい"; }, board => { board.future.data.reverse(); },
    board => { board.future.added = null; }, board => { board.nodes[1].future = { nested: [null, false] }; });
  for (const change of changes) {
    const board = clone(original); change(board);
    const latest = freeze(chunk(board, 1));
    const event = freeze(update(current, latest));
    const result = impl.applyLiveSessionUpdate(current, event);
    assert.equal(result.snapshot.summaries.at(-1), latest);
    assert.equal(result.snapshot.summaries.at(-1).whiteboard, board);
    assert.deepEqual(result, before.applyLiveSessionUpdate(current, event));
  }
  const reordered = Object.fromEntries(Object.entries(clone(original)).reverse());
  const result = impl.applyLiveSessionUpdate(current, update(current, chunk(reordered, 2)));
  assert.equal(result.snapshot.summaries.at(-1).whiteboard, original);
});

test("nearest non-null board includes an empty board and never shares across recordings", () => {
  const a = freeze(boardFixture()), empty = freeze({ title: "", nodes: [], edges: [] });
  const current = freeze(session([chunk(a), chunk(null), chunk(empty), chunk(undefined)]));
  const latest = freeze(chunk(clone(empty), 4));
  const reused = impl.applyLiveSessionUpdate(current, update(current, latest)).snapshot;
  assert.equal(reused.summaries.at(-1).whiteboard, empty);
  const changed = chunk(clone(a), 4);
  assert.equal(impl.applyLiveSessionUpdate(current, update(current, changed)).snapshot.summaries.at(-1), changed);
  for (const start of [current, { ...current, active: false }]) {
    const replaced = impl.applyLiveSessionUpdate(start, update(start, latest, { session_id: "recording-b", summary_count: 1 }));
    assert.equal(replaced.snapshot.summaries[0], latest);
  }
  for (const absent of [null, undefined]) {
    const missing = freeze(chunk(absent, 4));
    assert.equal(impl.applyLiveSessionUpdate(current, update(current, missing)).snapshot.summaries.at(-1), missing);
  }
});

test("recovery reuses known and consecutive boards but preserves every incoming chunk field", () => {
  const a = freeze(boardFixture({ seed: 1 })), b = freeze(boardFixture({ seed: 2 }));
  const current = freeze(session([chunk(a), chunk(b, 1)]));
  const incoming = freeze(session([chunk(clone(a)), chunk(clone(b), 1), chunk(null, 2), chunk(clone(b), 3),
    chunk(clone(a), 4), chunk(clone(a), 5)], { update_revision: 11 }));
  const wire = JSON.stringify(incoming);
  const merged = impl.mergeLiveSnapshot(current, incoming);
  assert.deepEqual(merged, before.mergeLiveSnapshot(current, incoming));
  assert.equal(merged.summaries[0].whiteboard, a);
  assert.equal(merged.summaries[1].whiteboard, b);
  assert.equal(merged.summaries[3].whiteboard, b);
  assert.notEqual(merged.summaries[4].whiteboard, a);
  assert.equal(merged.summaries[5].whiteboard, merged.summaries[4].whiteboard);
  assert.equal(merged.summaries[2], incoming.summaries[2]);
  assert.equal(merged.summaries[3].terms, incoming.summaries[3].terms);
  assert.equal(JSON.stringify(incoming), wire);
  const replacement = impl.mergeLiveSnapshot(current, { ...incoming, session_id: "recording-b" });
  assert.notEqual(replacement.summaries[0].whiteboard, a);
  assert.equal(replacement.summaries[3].whiteboard, replacement.summaries[1].whiteboard);
});

test("768 generated update/recovery sequences have exactly the predecessor's complete JSON values", () => {
  for (let seed = 1; seed <= 32; seed++) {
    const base = boardFixture({ count: 8 + seed, edges: seed * 2, seed });
    let old = freeze(session([chunk(base)])), next = old;
    for (let index = 1; index <= 24; index++) {
      const latestBoard = index % 7 === 0 ? null : index % 5 === 0 ? { ...clone(base), title: `変更 ${index}` } : clone(base);
      const event = freeze(update(next, chunk(latestBoard, index), {
        ...(index % 4 === 0 ? { transcript_line_count: 1002, pending_line_count: 2 } : {}),
        ...(index % 9 === 0 ? { finish_phase: "saving_record", finish_revision: index } : {}),
      }));
      const wire = JSON.stringify(event);
      const actual = impl.applyLiveSessionUpdate(next, event), expected = before.applyLiveSessionUpdate(old, event);
      assert.deepEqual(clone(actual), clone(expected));
      assert.equal(JSON.stringify(event), wire);
      next = actual.snapshot; old = expected.snapshot;
      if (index % 3 === 0) {
        const recovery = freeze({ ...clone(next), summaries: [...clone(next.summaries), chunk(clone(base), index + 100)] });
        next = impl.mergeLiveSnapshot(next, recovery);
        old = before.mergeLiveSnapshot(old, recovery);
        assert.deepEqual(clone(next), clone(old));
      }
      const stale = { ...event, update_revision: 1 };
      assert.equal(impl.applyLiveSessionUpdate(next, stale).snapshot, next);
    }
  }
});

test("excessively deep or cyclic non-wire data conservatively retains its own board", () => {
  const deep = () => { const board = boardFixture(); let cursor = board; for (let i = 0; i < 100; i++) cursor = cursor.deep = {}; return board; };
  const cyclic = () => { const board = boardFixture(); board.self = board; return board; };
  for (const factory of [deep, cyclic]) {
    const current = session([chunk(factory())]), latest = chunk(factory(), 1);
    assert.equal(impl.applyLiveSessionUpdate(current, update(current, latest)).snapshot.summaries.at(-1), latest);
  }
});

test("separate notifications reuse both actual cached layouts and changed content still recomputes", async () => {
  const engine = await loadWhiteboardLayout();
  let calls = 0;
  globalThis.window = { WhiteboardLayout: { ...engine, compute(...args) { calls++; return engine.compute(...args); } } };
  try {
    const { computeWhiteboardLayout } = await loadTypeScript("src/lib/whiteboardLayout.ts");
    const board = boardFixture({ count: 24, edges: 48 });
    let current = session([chunk(board)]);
    const options = [undefined, { topicIds: ["n0"] }];
    const layouts = options.map(option => computeWhiteboardLayout(board, option));
    for (let index = 1; index < 32; index++) {
      current = impl.applyLiveSessionUpdate(current, update(current, chunk(clone(board), index))).snapshot;
      for (let i = 0; i < 2; i++) assert.equal(computeWhiteboardLayout(current.summaries.at(-1).whiteboard, options[i]), layouts[i]);
    }
    assert.equal(calls, 2);
    const changed = { ...clone(board), title: "New board" };
    current = impl.applyLiveSessionUpdate(current, update(current, chunk(changed))).snapshot;
    assert.notEqual(computeWhiteboardLayout(current.summaries.at(-1).whiteboard), layouts[0]);
    assert.equal(calls, 3);
  } finally { delete globalThis.window; }
});
