import assert from "node:assert/strict";
import { performance } from "node:perf_hooks";
import { loadTypeScript } from "../tests/load-typescript.mjs";

const { createMarkdownRenderer } = await loadTypeScript("src/lib/markdownRenderer.ts");
const { TextStreamBuffer } = await loadTypeScript("src/lib/textStreamBuffer.ts");
const chunks = Array.from({ length: 1000 }, (_, index) => `**項目 ${index}**：説明と例。\n\n`);
const expected = chunks.join("");

function run(batched, workload = chunks) {
  // Parsing/output retention comparison only: no DOMPurify or browser DOM.
  const renderer = createMarkdownRenderer(html => html, { maxEntries: 256, maxBytes: Infinity });
  let source = "", html = "", renderPasses = 0;
  let now = 0, nextId = 0;
  const timers = new Map();
  const buffer = new TextStreamBuffer(text => {
    source += text;
    html = renderer.renderTransient(source);
    renderPasses++;
  }, (callback, delay) => {
    const id = ++nextId;
    timers.set(id, { callback, due: now + delay });
    return () => timers.delete(id);
  });
  const started = performance.now();
  for (const text of workload) {
    if (batched) {
      buffer.append(text);
      now++;
      for (const [id, timer] of timers) {
        if (timer.due <= now) {
          timers.delete(id);
          timer.callback();
        }
      }
    } else {
      source += text;
      html = renderer.render(source);
      renderPasses++;
    }
  }
  if (batched) buffer.flush();
  const elapsedMs = performance.now() - started;
  const result = { elapsedMs, renderPasses, entries: renderer.size, estimatedBytes: renderer.estimatedBytes, source, html };
  buffer.dispose();
  return result;
}

run(false, chunks.slice(0, 100));
run(true, chunks.slice(0, 100));
const legacy = [], current = [];
for (let round = 0; round < 5; round++) {
  const order = round % 2 === 0 ? [false, true] : [true, false];
  const results = new Map(order.map(batched => [batched, run(batched)]));
  assert.equal(results.get(false).source, expected);
  assert.equal(results.get(true).source, expected);
  assert.equal(results.get(false).html, results.get(true).html);
  legacy.push(results.get(false));
  current.push(results.get(true));
}
const median = rows => [...rows].sort((a, b) => a.elapsedMs - b.elapsedMs)[Math.floor(rows.length / 2)];
for (const [label, rows] of [["per_token_prefix_cache", legacy], ["batched_uncached_prefixes", current]]) {
  const { elapsedMs, renderPasses, entries, estimatedBytes } = median(rows);
  console.log(JSON.stringify({ label, elapsedMs, renderPasses, cachedEntries: entries, estimatedCacheBytes: estimatedBytes }));
}
console.log("Workload: 1000 chunks at simulated 1 ms intervals; warmup + 5 alternating rounds, median duration.");
console.log("Exact final source/HTML equality verified. This does not measure DOMPurify, DOM layout, application RSS or GPU behavior.");
