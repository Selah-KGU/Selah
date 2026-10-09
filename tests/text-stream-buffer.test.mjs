import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { TextStreamBuffer } = await loadTypeScript("src/lib/textStreamBuffer.ts");
const { ResourceScope } = await loadTypeScript("src/lib/resourceScope.ts");
const { createMarkdownRenderer } = await loadTypeScript("src/lib/markdownRenderer.ts");

function setup(t, emit) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const scope = new ResourceScope();
  const buffer = new TextStreamBuffer(emit, (callback, delay) => scope.schedule(callback, delay));
  scope.own(() => buffer.dispose());
  return { scope, buffer };
}

test("1000 fast tokens become one presentation update with all characters intact", t => {
  const text = [];
  const { scope, buffer } = setup(t, chunk => text.push(chunk));
  const tokens = Array.from({ length: 1000 }, (_, i) => `${i}字\n`);
  for (const token of tokens) buffer.append(token);
  assert.equal(text.length, 0);
  t.mock.timers.tick(48);
  assert.deepEqual(text, [tokens.join("")]);
  scope.dispose();
});

test("terminal flush preserves a sub-interval tail and cancels its delayed replay", t => {
  const text = [];
  const { scope, buffer } = setup(t, chunk => text.push(chunk));
  buffer.append("beginning ");
  t.mock.timers.tick(48);
  buffer.append("last "); buffer.append("sentence");
  buffer.flush();
  t.mock.timers.tick(100);
  assert.deepEqual(text, ["beginning ", "last sentence"]);
  scope.dispose();
});

test("switching conversations discards only the obsolete unpresented batch", t => {
  const text = [];
  const { scope, buffer } = setup(t, chunk => text.push(chunk));
  buffer.append("old conversation");
  buffer.clear();
  buffer.append("new conversation");
  t.mock.timers.tick(48);
  assert.deepEqual(text, ["new conversation"]);
  scope.dispose();
});

test("closing the view prevents scheduled or future tokens from updating its state", t => {
  const text = [];
  const { scope, buffer } = setup(t, chunk => text.push(chunk));
  buffer.append("pending");
  scope.dispose();
  buffer.append("late"); buffer.flush();
  t.mock.timers.tick(100);
  assert.deepEqual(text, []);
});

test("batching preserves whitespace, Unicode and the final Markdown output without caching prefixes", t => {
  let raw = "", html = "", updates = 0;
  const renderer = createMarkdownRenderer(value => value);
  const { scope, buffer } = setup(t, chunk => {
    raw += chunk;
    html = renderer.renderTransient(raw);
    updates++;
  });
  const source = "# 回答\n\n**文章** 😀\n次の行\n\n```ts\nconst result = 42;\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n".repeat(12);
  for (const character of source) {
    buffer.append(character);
    t.mock.timers.tick(1);
  }
  buffer.flush();
  assert.equal(raw, source);
  assert.equal(html, renderer.renderTransient(source));
  assert.ok(updates <= Math.ceil([...source].length / 48));
  assert.equal(renderer.size, 0);
  assert.equal(renderer.estimatedBytes, 0);
  scope.dispose();
});
