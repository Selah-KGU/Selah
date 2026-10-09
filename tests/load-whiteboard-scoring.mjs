// Test/benchmark-only counters and access to the actual private scorer.
// Product source has no telemetry, exports or injected wrappers.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { compileFunction } from "node:vm";
export async function loadWhiteboardScoring(before = false) {
  const file = before === "intersection" ? "./fixtures/whiteboard-intersection-before.js"
    : before ? "./fixtures/whiteboard-label-bound-before.js" : "../static/whiteboard-layout.js";
  let source = await readFile(new URL(file, import.meta.url), "utf8");
  const stats = { overlaps: 0, intersections: 0, candidateRects: 0, orientations: 0 };
  for (const [marker, replacement] of [
    ["function rectOverlapArea(a, b) {", "function rectOverlapArea(a, b) { stats.overlaps++;"],
    ["function rectSegmentIntersect(rect, seg) {", "function rectSegmentIntersect(rect, seg) { stats.intersections++;"],
    ...["d1", "d2", "d3", "d4"].map(name => [`var ${name} =`, `stats.orientations++; var ${name} =`]),
    ["var rect = { x1: x - lw / 2, y1: y - lh / 2, x2: x + lw / 2, y2: y + lh / 2 };", "stats.candidateRects++; var rect = { x1: x - lw / 2, y1: y - lh / 2, x2: x + lw / 2, y2: y + lh / 2 };"],
    ["global.WhiteboardLayout = { compute: compute, topics: topics };", "global.WhiteboardLayout = { compute: compute, topics: topics }; global.scoreLabel = placeEdgeLabel; global.cross = segmentsCross; global.intersect = rectSegmentIntersect;"],
  ]) {
    assert.equal(source.split(marker).length, 2, `unique instrumentation marker in ${file}: ${marker}`);
    source = source.replace(marker, replacement);
  }
  const result = compileFunction(source + "\nreturn window;", ["window", "stats"], { filename: file })({}, stats);
  return { layout: result.WhiteboardLayout, scoreLabel: result.scoreLabel, cross: result.cross, intersect: result.intersect, stats,
    reset() { for (const key of Object.keys(stats)) stats[key] = 0; } };
}
