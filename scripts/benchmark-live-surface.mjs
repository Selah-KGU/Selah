import assert from "node:assert/strict";
import { loadTypeScript } from "../tests/load-typescript.mjs";

const before = await loadTypeScript("tests/fixtures/live-transcript-before.ts");
const current = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const line = (i) => ({ at: "10:00:00", text: `授業 ${i} 中文・日本語 👩🏽‍💻` });
const updates = 500;
for (const count of [1000, 10000, 50000]) {
  const lines = Array.from({ length: count }, (_, i) => line(i));
  const full = { active: true, session_id: "fixture", course: null, started_at: null,
    transcript_lines: lines, pending_lines: lines, summaries: [] };
  const deltas = Array.from({ length: updates }, (_, i) => ({ session_id: "fixture", line_count: count + i + 1, line: line(count + i) }));
  function measure(mod, initial, window) {
    globalThis.gc?.();
    let snapshot = initial;
    const begin = performance.now();
    for (const delta of deltas) {
      const result = mod.applyTranscriptDelta(snapshot, delta);
      assert.equal(result.needsResync, false);
      snapshot = result.snapshot;
      assert.equal(window(snapshot).length, 120);
    }
    const ms = performance.now() - begin;
    return { ms, snapshot };
  }
  const times = [[], []];
  for (let i = 0; i < 10; i++) {
    let old, next;
    const runOld = () => measure(before, full, (snapshot) => snapshot.transcript_lines.slice(-120));
    const runNew = () => measure(current, current.liveSurfaceSnapshot(full), (snapshot) => snapshot.visible_lines);
    if (i % 2 === 0) { old = runOld(); next = runNew(); }
    else { next = runNew(); old = runOld(); }
    assert.equal(next.snapshot.transcript_line_count, old.snapshot.transcript_lines.length);
    assert.deepEqual(next.snapshot.visible_lines, old.snapshot.transcript_lines.slice(-120));
    if (i > 0) { times[0].push(old.ms); times[1].push(next.ms); }
  }
  for (const values of times) values.sort((a, b) => a - b);
  console.log(`${count} existing lines + ${updates} deltas: ${times[0][4].toFixed(3)} -> ${times[1][4].toFixed(3)} ms (nine-run median, Node, state update + visible window only)`);
}
console.log("Excludes fixture creation, IPC/JSON, backend persistence, Svelte/DOM/WebKit rendering and GPU. Full wire history remains intact.");
