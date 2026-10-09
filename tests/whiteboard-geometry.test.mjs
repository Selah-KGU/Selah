import test from "node:test";
import assert from "node:assert/strict";
import { loadWhiteboardLayout } from "./load-whiteboard-layout.mjs";
import { boardFixture, layoutCases } from "./fixtures/whiteboard-layout-cases.mjs";

const [before, current] = await Promise.all([loadWhiteboardLayout(true), loadWhiteboardLayout()]);
const copy = result => structuredClone(result);

test("shared layout preserves every geometry field, topic, chip and source against the frozen implementation", () => {
  const options = [undefined, {}, { fallbackBoardTitle: "タイトル | 🙂", externalNodeLabel: "外部,A" },
    { topicIds: [] }, { topicIds: ["n0"] }, { topicIds: ["n1", "n2"] },
    { topicIds: ["n2", "n0", "n0"] }, { topicIds: ["missing"] }];
  for (const [index, board] of layoutCases().entries()) {
    const original = copy(board);
    assert.deepEqual(copy(current.topics(board)), copy(before.topics(board)), `topics: case ${index}`);
    for (const opts of options) {
      const previous = before.compute(board, opts);
      const result = current.compute(board, opts);
      assert.deepEqual(copy(result), copy(previous), `geometry: case ${index}, ${JSON.stringify(opts)}`);
      assert.equal(JSON.stringify(result), JSON.stringify(previous), `all JSON bytes: case ${index}`);
    }
    assert.deepEqual(board, original, `input remains unchanged: case ${index}`);
  }
});

test("late labels score against all edges, preserving raw indices after invalid edges and self loops", () => {
  for (const hierarchy of ["legacy", "explicit", "backend"]) {
    for (const labelledAt of [0, 1, 15, 59, 60]) {
      const board = boardFixture({ count: 24, edges: 60, labelled: 0, hierarchy });
      if (labelledAt < 60) board.edges[labelledAt].label = "最後の関連 🙂";
      board.edges.splice(2, 0, null, { from: "missing", to: "n0", label: "ignored" }, { from: "n0", to: "n0", label: "self" });
      assert.deepEqual(copy(current.compute(board)), copy(before.compute(board)));
    }
  }
});

test("dense boards and reversed / duplicate edges keep exact label choices and tie order", () => {
  const board = boardFixture({ count: 75, edges: 160 });
  board.edges.push(...board.edges.slice(0, 40).map(edge => ({ from: edge.to, to: edge.from, label: edge.label })));
  for (const opts of [undefined, { topicIds: ["n0"] }, { topicIds: ["n0", "n2"] }]) {
    assert.deepEqual(copy(current.compute(board, opts)), copy(before.compute(board, opts)));
  }
});
