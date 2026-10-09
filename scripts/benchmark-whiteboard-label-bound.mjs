import assert from "node:assert/strict";
import { loadWhiteboardLayout } from "../tests/load-whiteboard-layout.mjs";
import { loadWhiteboardScoring } from "../tests/load-whiteboard-scoring.mjs";
import { boardFixture } from "../tests/fixtures/whiteboard-layout-cases.mjs";
const [before, after, measuredBefore, measuredAfter] = await Promise.all([
  loadWhiteboardLayout("label-bound"), loadWhiteboardLayout(), loadWhiteboardScoring(true), loadWhiteboardScoring(),
]);
const option = { fallbackBoardTitle: "知識整理", externalNodeLabel: "外部" };
const median = values => values.sort((a, b) => a - b)[4];
let last;
function time(engine, board, iterations) {
  const start = performance.now();
  for (let i = 0; i < iterations; i++) last = engine.compute(board, option);
  return (performance.now() - start) / iterations;
}
console.log("Baseline: actual shared layout immediately before nonnegative-score candidate bounds.");
for (const config of [
  { count: 18, edges: 36, labelled: 1, iterations: 30 },
  { count: 75, edges: 160, labelled: 1, iterations: 8 },
  { count: 96, edges: 192, labelled: 1, iterations: 6 },
  { count: 75, edges: 160, labelled: 0.25, iterations: 20 },
  { count: 75, edges: 160, labelled: 0, iterations: 80 },
]) {
  const board = boardFixture(config);
  const old = before.compute(board, option), current = after.compute(board, option);
  assert.deepEqual(current, old);
  assert.equal(JSON.stringify(current), JSON.stringify(old));
  const times = [[], []];
  for (let sample = -3; sample < 9; sample++) {
    for (const index of sample % 2 ? [1, 0] : [0, 1]) {
      const ms = time(index ? after : before, board, config.iterations);
      if (sample >= 0) times[index].push(ms);
    }
  }
  assert.equal(JSON.stringify(last), JSON.stringify(old));
  measuredBefore.reset(); measuredAfter.reset();
  assert.deepEqual(measuredAfter.layout.compute(board, option), measuredBefore.layout.compute(board, option));
  console.log(JSON.stringify({ ...config, msPerCompleteLayout: times.map(median),
    rectangleTests: [measuredBefore.stats.overlaps, measuredAfter.stats.overlaps],
    preciseSegmentTests: [measuredBefore.stats.intersections, measuredAfter.stats.intersections],
    candidateRects: [measuredBefore.stats.candidateRects, measuredAfter.stats.candidateRects],
    renderedNodes: current.nodes.length, renderedEdges: current.edges.length, completeJsonBytesMatch: true }));
}
console.log("Nine alternating batch medians after three warmups; actual uninstrumented layout engine. Operation counts are separate injected runs and excluded from timing. Includes complete normalization, forest, chips, geometry and label choices. Input creation, wrapper cache, DOM/WebKit, IPC, model/recording, output comparison/destruction, application RSS and GPU are excluded; graph content and candidate offsets/order unchanged; no universal speedup claim.");
