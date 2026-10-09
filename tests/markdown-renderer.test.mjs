import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { RenderedTextCache } = await loadTypeScript("src/lib/renderedTextCache.ts");
const { createMarkdownRenderer, globalLineBreaks, renderWithGlobalParser } =
  await loadTypeScript("tests/fixtures/markdown-renderer.ts");

test("the byte budget bounds retained source and HTML even when the entry limit is not reached", () => {
  const cache = new RenderedTextCache(text => `<p>${text}</p>`, { maxEntries: 256, maxBytes: 200 });
  for (let index = 0; index < 100; index++) {
    const text = `${index} ${"字".repeat(20)}`;
    assert.equal(cache.render(text), `<p>${text}</p>`);
    assert.ok(cache.estimatedBytes <= 200);
    assert.ok(cache.size < 256);
  }
  cache.clear();
  assert.equal(cache.estimatedBytes, 0);
  assert.equal(cache.size, 0);
});

test("recently read messages remain cached while the least recently used message is evicted", () => {
  const calls = [];
  const cache = new RenderedTextCache(text => { calls.push(text); return text; }, { maxEntries: 2 });
  cache.render("first"); cache.render("second");
  cache.render("first"); cache.render("third");
  cache.render("first"); cache.render("second");
  assert.deepEqual(calls, ["first", "second", "third", "second"]);
});

test("oversized answers still render fully without displacing useful cached messages", () => {
  const cache = new RenderedTextCache(text => text.toUpperCase(), { maxBytes: 32 });
  cache.render("small");
  const retained = cache.estimatedBytes;
  const large = "entire answer ".repeat(1000);
  assert.equal(cache.render(large), large.toUpperCase());
  assert.equal(cache.size, 1);
  assert.equal(cache.estimatedBytes, retained);
});

test("transient prefixes never accumulate in the permanent message cache", () => {
  const cache = new RenderedTextCache(text => text, { maxBytes: 200 });
  cache.render("complete message");
  const retained = cache.estimatedBytes;
  for (let i = 1; i <= 1000; i++) assert.equal(cache.renderTransient("t".repeat(i)), "t".repeat(i));
  assert.equal(cache.size, 1);
  assert.equal(cache.estimatedBytes, retained);
});

test("parser failure leaves the existing cache usable and retries the failing input", () => {
  let failures = 0;
  const cache = new RenderedTextCache(text => {
    if (text === "bad") { failures++; throw new Error("parse failure"); }
    return text;
  });
  cache.render("good");
  assert.throws(() => cache.render("bad"));
  assert.throws(() => cache.render("bad"));
  assert.equal(cache.render("good"), "good");
  assert.equal(cache.size, 1);
  assert.equal(failures, 2);
});

test("Agent and LIVE renderers do not alter the global document parser's line break policy", () => {
  globalLineBreaks(false);
  const renderer = createMarkdownRenderer(html => html);
  assert.match(renderer.render("first\nsecond"), /<br>/);
  assert.doesNotMatch(renderWithGlobalParser("first\nsecond"), /<br>/);
  globalLineBreaks(true);
  assert.match(renderer.renderTransient("third\nfourth"), /<br>/);
  globalLineBreaks(false);
});

test("both cached and transient HTML pass through the supplied sanitizer before being returned", () => {
  const inputs = [];
  const renderer = createMarkdownRenderer(html => { inputs.push(html); return "sanitized result"; });
  const source = "# Heading\n\n<script>anything</script>\n\n| A | B |\n|---|---|\n| 1 | 2 |";
  assert.equal(renderer.render(source), "sanitized result");
  assert.equal(renderer.render(source), "sanitized result");
  assert.equal(renderer.renderTransient(source), "sanitized result");
  assert.equal(inputs.length, 2);
  assert.match(inputs[0], /<h1>Heading<\/h1>/);
  assert.match(inputs[0], /<table>/);
});
