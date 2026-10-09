import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
import { loadWhiteboardLayout } from "./load-whiteboard-layout.mjs";

globalThis.window = { WhiteboardLayout: {
  compute: () => null,
  topics: () => [],
} };
const { computeWhiteboardLayout } = await loadTypeScript("src/lib/whiteboardLayout.ts");
const layout = await loadWhiteboardLayout();

test("preview and topic layout retain separate cache entries", () => {
  let calls = 0;
  window.WhiteboardLayout.compute = () => ({ nodes: [], edges: [], title: String(++calls) });
  const board = { nodes: [] };
  const preview = computeWhiteboardLayout(board);
  const topic = computeWhiteboardLayout(board, { topicIds: ["a"] });
  for (let i = 0; i < 100; i++) {
    assert.equal(computeWhiteboardLayout(board), preview);
    assert.equal(computeWhiteboardLayout(board, { topicIds: ["a"] }), topic);
  }
  assert.equal(calls, 2);
});

test("a failing layout degrades to null and does not repeat on each speech tick", () => {
  let calls = 0;
  window.WhiteboardLayout.compute = () => { calls++; throw new Error("test layout failure"); };
  const board = { nodes: [] };
  const warn = console.warn;
  console.warn = () => {};
  try {
    for (let i = 0; i < 100; i++) assert.equal(computeWhiteboardLayout(board), null);
    assert.equal(calls, 1);
  } finally {
    console.warn = warn;
  }
});

test("delimiter-containing fallback titles and source labels cannot collide", () => {
  window.WhiteboardLayout = layout;
  const board = { nodes: [{ id: "a", label: "A" }, { id: "b", label: "B" }] };
  const options = [
    { fallbackBoardTitle: "A|B", externalNodeLabel: "C" },
    { fallbackBoardTitle: "A", externalNodeLabel: "B|C" },
    { fallbackBoardTitle: "日本語 | 中文 🙂", externalNodeLabel: "引用,外部|\"\\" },
  ];
  for (const opts of options) {
    const result = computeWhiteboardLayout(board, opts);
    assert.deepEqual(structuredClone(result), structuredClone(layout.compute(board, opts)));
    assert.equal(result.title, opts.fallbackBoardTitle);
    assert.equal(result.nodes[0].sourceLabel, opts.externalNodeLabel);
    assert.equal(computeWhiteboardLayout(board, { ...opts }), result);
  }
});

test("comma-containing topic IDs retain the requested nodes rather than another cached selection", () => {
  window.WhiteboardLayout = layout;
  const board = { normalized_by: "backend", nodes: [
    ...["a,b", "a", "b", "a|b", ""].map(id => ({ id, label: `topic ${id}`, role: "main" })),
    { id: "child", label: "child", role: "branch", parent_id: "a,b" },
  ], edges: [{ from: "a,b", to: "child", label: "related" }] };
  const selections = [["a,b"], ["a", "b"], ["a|b"], [], [""]];
  const results = selections.map(topicIds => computeWhiteboardLayout(board, { topicIds }));
  assert.deepEqual(Array.from(results[0].nodes, n => n.id), ["a,b", "child"]);
  assert.deepEqual(Array.from(results[1].nodes, n => n.id), ["a", "b"]);
  for (const [index, topicIds] of selections.entries()) {
    assert.deepEqual(structuredClone(results[index]), structuredClone(layout.compute(board, { topicIds })));
    assert.equal(computeWhiteboardLayout(board, { topicIds: [...topicIds] }), results[index]);
  }
});

test("equivalent empty options share the default layout while the per-board cache stays bounded", () => {
  let calls = 0;
  window.WhiteboardLayout = { ...layout, compute: (board, opts) => { calls++; return layout.compute(board, opts); } };
  const board = { nodes: [{ id: "a", label: "A" }, { id: "b", label: "B" }] };
  const first = computeWhiteboardLayout(board);
  for (const opts of [{}, { topicIds: [] }, { fallbackBoardTitle: "", externalNodeLabel: "" }]) {
    assert.equal(computeWhiteboardLayout(board, opts), first);
  }
  assert.equal(calls, 1);
  const variants = Array.from({ length: 8 }, (_, i) => ({ fallbackBoardTitle: `title ${i}` }));
  const results = variants.map(opts => computeWhiteboardLayout(board, opts));
  for (const [index, opts] of variants.entries()) assert.equal(computeWhiteboardLayout(board, opts), results[index]);
  assert.equal(calls, 9);
  assert.notEqual(computeWhiteboardLayout(board), first);
  assert.equal(calls, 10, "the oldest ninth variant was evicted");
  assert.notEqual(computeWhiteboardLayout(structuredClone(board)), first);
  assert.equal(calls, 11, "a different immutable board has its own cache");
});
