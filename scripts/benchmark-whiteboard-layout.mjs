import assert from "node:assert/strict";
import { loadWhiteboardLayout } from "../tests/load-whiteboard-layout.mjs";
import { boardFixture } from "../tests/fixtures/whiteboard-layout-cases.mjs";

const identity = process.argv.includes("--identity");
const [before, current] = await Promise.all([loadWhiteboardLayout(identity ? "identity" : true), loadWhiteboardLayout()]);
console.log(identity ? "Baseline: immediately before display-ID repair and prototype-free indexes." : "Baseline: before shared edge-geometry optimization.");
let last;
function measure(layout, board, options, iterations) {
  const start = performance.now();
  for (let i = 0; i < iterations; i++) last = layout.compute(board, options);
  return (performance.now() - start) / iterations;
}

for (const config of [
  { count: 18, edges: 36, labelled: 1, iterations: 30 },
  { count: 75, edges: 160, labelled: 1, iterations: 12 },
  { count: 75, edges: 160, labelled: 0.25, iterations: 20 },
  { count: 75, edges: 160, labelled: 0, iterations: 100 },
]) {
  const board = boardFixture(config);
  const options = { fallbackBoardTitle: "知識整理", externalNodeLabel: "外部" };
  assert.deepEqual(structuredClone(current.compute(board, options)), structuredClone(before.compute(board, options)));
  for (let i = 0; i < 3; i++) { measure(before, board, options, config.iterations); measure(current, board, options, config.iterations); }
  const times = [[], []];
  for (let i = 0; i < 9; i++) {
    for (const index of i % 2 === 0 ? [0, 1] : [1, 0]) {
      times[index].push(measure(index ? current : before, board, options, config.iterations));
    }
  }
  for (const values of times) values.sort((a, b) => a - b);
  assert.deepEqual(structuredClone(last), structuredClone(before.compute(board, options)));
  console.log(`${config.count} nodes, ${config.edges} edges, ${config.labelled * 100}% labelled: ${times[0][4].toFixed(3)} -> ${times[1][4].toFixed(3)} ms/layout (nine alternating batch medians)`);
}
console.log("Actual shared JS layout, Node with separate window objects; input creation, cache, DOM/WebKit, IPC, recording, application RSS and GPU are excluded.");
