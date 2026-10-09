import test from "node:test";
import assert from "node:assert/strict";
import { loadWhiteboardLayout } from "./load-whiteboard-layout.mjs";
import { loadWhiteboardScoring } from "./load-whiteboard-scoring.mjs";
import { boardFixture, layoutCases } from "./fixtures/whiteboard-layout-cases.mjs";
import { duplicateBoard, reservedBoard, prototypeBoard, phantomBoard } from "./fixtures/whiteboard-identity-cases.mjs";
const [before, after, oldScore, newScore] = await Promise.all([
  loadWhiteboardLayout("label-bound"), loadWhiteboardLayout(), loadWhiteboardScoring(true),
  loadWhiteboardScoring(!!process.env.SELAH_WHITEBOARD_BOUND_BEFORE),
]);
const clone = value => structuredClone(value);
const options = [undefined, {}, { fallbackBoardTitle: "タイトル | 🙂", externalNodeLabel: "外部,A" },
  { topicIds: [] }, { topicIds: ["n0"] }, { topicIds: ["n1", "n2"] },
  { topicIds: ["n2", "n0", "n0"] }, { topicIds: ["missing"] }];

test("candidate bounds preserve every layout byte, topic, chip and source across current normalization cases", () => {
  const cases = [...layoutCases(), duplicateBoard(), reservedBoard(), prototypeBoard(), phantomBoard()];
  for (const [index, board] of cases.entries()) {
    const initial = clone(board);
    assert.deepEqual(after.topics(board), before.topics(board), `topics ${index}`);
    for (const option of options) {
      const old = before.compute(board, option), result = after.compute(board, option);
      assert.deepEqual(result, old, `complete layout ${index}`);
      assert.equal(JSON.stringify(result), JSON.stringify(old), `all bytes ${index}`);
    }
    assert.deepEqual(board, initial, `immutable input ${index}`);
  }
});

test("dense late, duplicate, reversed and absent labels keep raw-index tie rules", () => {
  for (let seed = 1; seed <= 32; seed++) {
    const board = boardFixture({ count: [3, 12, 24, 75][seed % 4], edges: seed * 5,
      seed, hierarchy: ["backend", "explicit", "legacy"][seed % 3], labelled: [0, 0.25, 1][seed % 3] });
    board.edges.push(...board.edges.slice(0, 8).map(edge => ({ ...edge, from: edge.to, to: edge.from })));
    board.edges.splice(1, 0, null, { from: "n0", to: "n0", label: "自己" }, { from: "missing", to: "n0", label: "無効" });
    board.edges.at(-1).label = "最後の長いラベル 中文・日本語 👩🏽‍💻";
    for (const option of options.slice(0, 4)) {
      const result = after.compute(board, option), old = before.compute(board, option);
      assert.deepEqual(result, old);
      assert.equal(JSON.stringify(result), JSON.stringify(old));
    }
  }
});

test("2048 direct collision cases keep the chosen position and occupied rectangles exactly", () => {
  let state = 1;
  const random = max => { state = (Math.imul(state, 1664525) + 1013904223) >>> 0; return state % max; };
  for (let index = 0; index < 2048; index++) {
    const point = () => ({ x: random(201) - 50, y: random(201) - 50 });
    const rect = () => { const { x, y } = point(), w = random(20), h = random(15); return { x1: x, y1: y, x2: x + w, y2: y + h }; };
    const segments = Array.from({ length: random(12) }, () => {
      const a = point(), b = point();
      return { x1: a.x, y1: a.y, x2: b.x, y2: b.y, minX: Math.min(a.x, b.x), maxX: Math.max(a.x, b.x), minY: Math.min(a.y, b.y), maxY: Math.max(a.y, b.y) };
    });
    const occupied = Array.from({ length: random(8) }, rect), nodes = Array.from({ length: random(10) }, rect);
    const args = [random(201) - 50, random(201) - 50, point(), point(), random(15) + 0.2, 3.3, occupied, nodes,
      index % 3 ? segments : null, random(segments.length + 1) - 1, index];
    const previous = clone(args), current = clone(args);
    assert.deepEqual(newScore.scoreLabel(...current), oldScore.scoreLabel(...previous));
    assert.deepEqual(current[6], previous[6], "only the same winning rectangle is appended");
    assert.deepEqual(args[6], occupied, "source occupancy is not mutated by the clones");
  }
  // Equal +/- normal penalties must retain the first positive normal choice.
  const tied = [50, 50, { x: 20, y: 50 }, { x: 80, y: 50 }, 8, 3.3,
    [{ x1: 46, y1: 48.35, x2: 54, y2: 51.65 }], [], null, -1, 1];
  assert.deepEqual(newScore.scoreLabel(...clone(tied)), { x: 50, y: 53.8 });
  assert.deepEqual(newScore.scoreLabel(...clone(tied)), oldScore.scoreLabel(...clone(tied)));
});

test("actual scoring eliminates provably losing work rather than dropping graph content", () => {
  const open = [50, 50, { x: 20, y: 50 }, { x: 80, y: 50 }, 8, 3.3, [], [], null, -1, 0];
  oldScore.reset(); newScore.reset();
  assert.deepEqual(newScore.scoreLabel(...clone(open)), oldScore.scoreLabel(...clone(open)));
  assert.equal(oldScore.stats.candidateRects, 35);
  assert.equal(newScore.stats.candidateRects, 1);
  for (const config of [{ count: 18, edges: 36 }, { count: 75, edges: 160 }, { count: 96, edges: 192 }]) {
    const board = boardFixture(config);
    oldScore.reset(); newScore.reset();
    const result = newScore.layout.compute(board), previous = oldScore.layout.compute(board);
    assert.deepEqual(result, previous);
    assert.equal(JSON.stringify(result), JSON.stringify(previous));
    assert.ok(newScore.stats.overlaps < oldScore.stats.overlaps, "fewer actual rectangle tests");
    assert.ok(newScore.stats.intersections < oldScore.stats.intersections, "fewer actual precise segment tests");
    assert.ok(newScore.stats.candidateRects <= oldScore.stats.candidateRects);
  }
});
