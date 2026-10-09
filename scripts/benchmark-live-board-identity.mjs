// Actual event/recovery algorithms and actual layout/cache; no app, IPC or mic.
import assert from "node:assert/strict";
import { loadTypeScript } from "../tests/load-typescript.mjs";
import { loadWhiteboardLayout } from "../tests/load-whiteboard-layout.mjs";
import { boardFixture } from "../tests/fixtures/whiteboard-layout-cases.mjs";

const before = await loadTypeScript("tests/fixtures/live-board-identity-before.ts");
const after = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const { computeWhiteboardLayout } = await loadTypeScript("src/lib/whiteboardLayout.ts");
const engine = await loadWhiteboardLayout();
const options = [undefined, { topicIds: ["n0"], fallbackBoardTitle: "知識整理", externalNodeLabel: "外部" }];
const clone = value => JSON.parse(JSON.stringify(value));
const median = values => values.sort((a, b) => a - b)[Math.floor(values.length / 2)];
const initial = () => ({ active: true, session_id: "benchmark", update_revision: 0, course: null, started_at: null,
  summaries: [], transcript_line_count: 1000, visible_lines: [{ text: "Full final line", at: "10:00" }],
  pending_from_line: 0, finish_phase: null, finish_revision: 0, summarizing: false, next_summary_at_ms: null });
function fixtures({ count, changeEvery = 32 }) {
  const board = boardFixture({ count, edges: count * 2 });
  return Array.from({ length: 32 }, (_, index) => ({ active: true, session_id: "benchmark", update_revision: index + 1,
    course: null, started_at: null, transcript_line_count: 1000, pending_line_count: 0, summary_count: index + 1,
    finish_phase: null, finish_revision: 0, summarizing: false, next_summary_at_ms: null,
    latest_summary: { title: `段 ${index}`, range_label: "10:00-10:05", body: `Full summary 🙂 ${index}`, line_count: index,
      terms: [{ term: "講義", explanation: "Complete term" }], whiteboard: { ...clone(board), title: `Board ${Math.floor(index / changeEvery)}` } },
  }));
}
function run(impl, inputs, withLayout) {
  let calls = 0;
  globalThis.window = { WhiteboardLayout: { ...engine, compute(...args) { calls++; return engine.compute(...args); } } };
  let snapshot = initial();
  const layouts = [];
  const start = performance.now();
  for (const update of inputs) {
    const result = impl.applyLiveSessionUpdate(snapshot, update);
    if (result.needsResync) throw new Error("unexpected resync");
    snapshot = result.snapshot;
    if (withLayout) {
      const board = snapshot.summaries.at(-1).whiteboard;
      for (const option of options) layouts.push(computeWhiteboardLayout(board, option));
    }
  }
  const ms = performance.now() - start;
  delete globalThis.window;
  const distinctBoards = [...new Set(snapshot.summaries.map(chunk => chunk.whiteboard))];
  return { ms, snapshot, layouts, calls, boardCount: distinctBoards.length,
    distinctBoardJsonBytes: distinctBoards.reduce((bytes, board) => bytes + Buffer.byteLength(JSON.stringify(board)), 0) };
}
console.log("Baseline: immediately preceding actual update/merge algorithms; 32 independently parsed notifications per sequence.");
for (const config of [{ count: 24 }, { count: 96 }, { count: 96, changeEvery: 8 }, { count: 96, changeEvery: 1 }]) {
  const inputs = fixtures(config);
  const old = run(before, clone(inputs), true), current = run(after, clone(inputs), true);
  assert.deepEqual(current.snapshot, old.snapshot);
  assert.deepEqual(current.layouts, old.layouts);
  const times = [[], []], updateTimes = [[], []];
  for (let sample = -3; sample < 9; sample++) {
    for (const index of sample % 2 === 0 ? [0, 1] : [1, 0]) {
      // Parse/preparation happens outside each timed call; every run uses fresh identities.
      const combined = run(index ? after : before, clone(inputs), true);
      const updateOnly = run(index ? after : before, clone(inputs), false);
      if (sample >= 0) { times[index].push(combined.ms); updateTimes[index].push(updateOnly.ms); }
    }
  }
  console.log(JSON.stringify({ ...config, chunks: inputs.length,
    updateAndTwoLayoutsMs: times.map(median), updatesOnlyMs: updateTimes.map(median),
    layoutComputations: [old.calls, current.calls], distinctBoards: [old.boardCount, current.boardCount],
    distinctBoardJsonBytes: [old.distinctBoardJsonBytes, current.distinctBoardJsonBytes], completeValuesAndLayoutsMatch: true }));
}
console.log("Nine alternating medians after three warmups. Times include actual update/compare and cached overview/topic layouts. JSON parsing, fixture generation, output comparisons/destruction, DOM/WebKit, IPC, app RSS and GPU excluded. Distinct JSON bytes describe represented data, not heap usage. New/changed boards still compute their layouts; comparison adds update-only work.");
