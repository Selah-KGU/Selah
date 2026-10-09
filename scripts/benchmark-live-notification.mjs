// Compare the supported complete and reference wire paths through actual UI logic.
import assert from "node:assert/strict";
import { loadTypeScript } from "../tests/load-typescript.mjs";
import { loadWhiteboardLayout } from "../tests/load-whiteboard-layout.mjs";
import { boardFixture } from "../tests/fixtures/whiteboard-layout-cases.mjs";
const { applyLiveSessionNotification } = await loadTypeScript("src/lib/views/live/liveNotification.ts");
const { computeWhiteboardLayout } = await loadTypeScript("src/lib/whiteboardLayout.ts");
const engine = await loadWhiteboardLayout();
const clone = value => JSON.parse(JSON.stringify(value));
const median = values => values.sort((a, b) => a - b)[4];
const options = [undefined, { topicIds: ["n0"], fallbackBoardTitle: "知識整理", externalNodeLabel: "外部" }];
function makeCase(count) {
  const board = boardFixture({ count, edges: count * 2 });
  const current = { active: true, session_id: "benchmark", update_revision: 10, course: null, started_at: null,
    summaries: [{ title: "Initial", range_label: "10:00", body: "Full initial summary", line_count: 0, whiteboard: board }],
    transcript_line_count: 1000, visible_lines: [{ at: "10:00", text: "Complete final transcript line" }],
    pending_from_line: 900, finish_phase: null, finish_revision: 0, next_summary_at_ms: 100, summarizing: false };
  const full = [], compact = [];
  for (let index = 0; index < 32; index++) {
    const metadata = { active: true, session_id: "benchmark", update_revision: index + 11, course: null, started_at: null,
      transcript_line_count: 1000, pending_line_count: 0, summary_count: index + 2,
      finish_phase: null, finish_revision: 0, next_summary_at_ms: 200, summarizing: false };
    const chunk = { title: `段 ${index}`, range_label: "10:00-10:05", body: `Full summary 🙂 ${index}`, line_count: index,
      terms: [{ term: "講義", explanation: "Complete term explanation", source_excerpt: "原文" }] };
    full.push(JSON.stringify({ ...metadata, latest_summary: { ...chunk, whiteboard: board } }));
    compact.push(JSON.stringify({ whiteboard_delta_version: 1, ...metadata,
      latest_summary: { ...chunk, whiteboard_from_summary: index } }));
  }
  return { current, full, compact };
}
function run(initial, strings) {
  let snapshot = clone(initial), calls = 0;
  globalThis.window = { WhiteboardLayout: { ...engine, compute(...args) { calls++; return engine.compute(...args); } } };
  // Both consumers already display the complete initial board. Asset loading,
  // initial parsing/layout and input creation are outside the event timing.
  const initialLayouts = options.map(option => computeWhiteboardLayout(snapshot.summaries[0].whiteboard, option));
  let result;
  const start = performance.now();
  for (const string of strings) {
    result = applyLiveSessionNotification(snapshot, JSON.parse(string));
    if (result.needsResync) throw new Error("unexpected resync");
    snapshot = result.snapshot;
    for (let i = 0; i < options.length; i++) {
      const layout = computeWhiteboardLayout(snapshot.summaries.at(-1).whiteboard, options[i]);
      if (layout !== initialLayouts[i]) throw new Error("cached layout was replaced");
    }
  }
  const ms = performance.now() - start;
  delete globalThis.window;
  return { ms, snapshot, layouts: initialLayouts, calls };
}
console.log("32 complete JSON notifications vs version-1 carried references; both use the same actual current UI update and layout cache.");
for (const count of [24, 96]) {
  const fixture = makeCase(count);
  const full = run(fixture.current, fixture.full), compact = run(fixture.current, fixture.compact);
  assert.deepEqual(full.snapshot, compact.snapshot);
  assert.deepEqual(full.layouts, compact.layouts);
  assert.equal(full.calls, 2); assert.equal(compact.calls, 2);
  const times = [[], []];
  for (let sample = -3; sample < 9; sample++) {
    for (const index of sample % 2 ? [1, 0] : [0, 1]) {
      const result = run(fixture.current, index ? fixture.compact : fixture.full);
      if (sample >= 0) times[index].push(result.ms);
    }
  }
  console.log(JSON.stringify({ nodes: count, edges: count * 2, notifications: 32,
    jsonParseUpdateAndCachedLayoutsMs: times.map(median),
    jsonBytes: [fixture.full, fixture.compact].map(strings => strings.reduce((sum, string) => sum + Buffer.byteLength(string), 0)),
    initialLayoutComputations: [full.calls, compact.calls], completeValuesAndLayoutsMatch: true }));
}
console.log("Nine alternating medians after three warmups. Includes JSON.parse, reference validation/board comparison, state updates and actual cached overview/topic lookup. Excludes native encoding, IPC dispatch, initial board parsing/layout, fixture generation, result comparison/destruction, DOM/WebKit, app RSS and GPU. Synthetic wire content; native byte checks separately use actual Rust captures.");
