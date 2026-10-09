import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";
import { loadLiveDeck } from "./load-live-deck.mjs";

const { visibleDeckCards } = await loadTypeScript("src/lib/views/live/liveDeck.ts");
const [summary, oldSummary, rail, oldRail] = await Promise.all([
  loadLiveDeck("LiveSummaryCard"), loadLiveDeck("LiveSummaryCard", true),
  loadLiveDeck("LiveRightRail"), loadLiveDeck("LiveRightRail", true),
]);

const oldCards = (items, front) => items.map((item, index) => ({
  item, index, offset: items.length ? (index - front + items.length) % items.length : 0,
})).filter(({ offset }) => offset >= 0 && offset <= 2);

test("visible windows match the old cyclic stack's indices, offsets and DOM order", () => {
  for (const count of [0, 1, 2, 3, 4, 5, 12, 100]) {
    const items = Object.freeze(Array.from({ length: count }, (_, i) => Object.freeze({ id: i })));
    for (let front = -count * 3 - 1; front <= count * 3 + 1; front++) {
      const cards = visibleDeckCards(items, front);
      assert.deepEqual(cards, oldCards(items, front), `${count} items, front ${front}`);
      assert.ok(cards.length <= 3);
      for (const card of cards) assert.equal(card.item, items[card.index]);
    }
  }
  for (const front of [NaN, Infinity, -Infinity]) assert.deepEqual(visibleDeckCards([1, 2], front), []);
});

const noop = () => {};
const entry = count => ({ range_label: "10:00–10:15", body: Array.from(
  { length: count }, (_, i) => `- 要点 ${i} **Unicode 🙂**`,
).join("\n"), isOverall: false });
const html = (module, props) => module.render(module.Component, { props }).body;

test("real summary markup renders only three headlines and keeps the full body and detail entry", () => {
  for (const count of [0, 1, 2, 3, 4, 12, 128]) {
    const chunk = Object.freeze(entry(count));
    const entries = Object.freeze([chunk]);
    const oldCalls = [], newCalls = [];
    const props = { entries, activeIdx: 0, segmentCount: 7, onOpenDetail: noop };
    const before = html(oldSummary, { ...props, renderMd: text => { oldCalls.push(text); return `<p>${text}</p>`; } });
    const after = html(summary, { ...props, renderMd: text => { newCalls.push(text); return `<p>${text}</p>`; } });
    assert.equal((before.match(/class="summary-pt-card/g) ?? []).length, Math.max(count, 1));
    assert.equal((after.match(/class="summary-pt-card/g) ?? []).length, Math.max(Math.min(count, 3), 1));
    assert.deepEqual(newCalls, oldCalls.slice(0, 3));
    for (const marker of ["阶段摘要を開く", "10:00–10:15", "7区間"]) assert.ok(after.includes(marker));
    assert.equal(entries[0], chunk);
    assert.equal(chunk.body, entry(count).body);
  }
  assert.equal(html(summary, { entries: [], activeIdx: -1, segmentCount: 0, renderMd: () => "", onOpenDetail: noop }).includes("summary-stack"), false);
});

function visibleTermMarkup(markup) {
  return [...markup.matchAll(/<button\b([^>]*\bclass="term-card[^>]*?)>([\s\S]*?)<\/button>/g)]
    .filter(([, attrs]) => !/visibility:\s*hidden/.test(attrs))
    .map(([, attrs, body]) => {
      const styles = (attrs.match(/style="([^"]*)"/)?.[1] ?? "").split(";")
        .map(s => s.trim()).filter(Boolean)
        .filter(s => !/^(visibility|pointer-events|transition):/.test(s)).sort();
      return {
        classes: attrs.match(/class="([^"]*)"/)?.[1], styles,
        aria: attrs.match(/aria-hidden="([^"]*)"/)?.[1],
        tab: attrs.match(/tabindex="([^"]*)"/)?.[1],
        body: body.replace(/<!--[\s\S]*?-->/g, "").replace(/\s+/g, " ").trim(),
      };
    });
}

test("real term markup preserves visible content, stack styles, accessibility and navigation across every rotation", () => {
  for (const count of [0, 1, 2, 3, 4, 5, 12, 100]) {
    const terms = Array.from({ length: count }, (_, i) => Object.freeze({
      term: `術語 ${i} 🙂`, explanation: `完整解釋 ${i}`,
      source_excerpt: i % 2 ? `引用 ${i}` : "", external_source: i % 3 ? `外部 ${i}` : "",
    }));
    Object.freeze(terms);
    for (let front = 0; front < Math.max(count, 1); front++) {
      const props = {
        summaryEntries: [], activeSummaryIdx: -1, summarySegmentCount: 0,
        renderMd: text => text, onOpenSummaryDetail: noop, onSelectSegment: noop,
        onOpenOverall: noop, summarizing: false, summaryStatusLabel: "", previewLayout: null,
        activeSummaryTerms: terms, termCardIdx: front,
        termFloatLabels: { title: "用語注釈", source: "引用", externalSource: "出典", previous: "前", next: "次" },
        termStackOffset: i => count ? (i - front + count) % count : 0,
        onOpenWhiteboard: noop, onSelectTermCard: noop, onTermCardPrev: noop, onTermCardNext: noop,
      };
      const before = html(oldRail, props), after = html(rail, props);
      assert.deepEqual(visibleTermMarkup(after), visibleTermMarkup(before), `${count} / ${front}`);
      assert.equal((after.match(/class="term-card(?:\s|")/g) ?? []).length, Math.min(count, 3));
      if (count) assert.ok(after.includes(`${front + 1}/${count}`));
      if (count > 1) assert.ok(after.includes('aria-label="前"') && after.includes('aria-label="次"'));
    }
  }
});

test("cyclic projection reaches every original item without capping or copying the source", () => {
  const items = Object.freeze(Array.from({ length: 10_000 }, (_, i) => Object.freeze({ id: i })));
  for (let front = 0; front < items.length; front++) {
    const cards = visibleDeckCards(items, front);
    assert.equal(cards.find(card => card.offset === 0).item, items[front]);
    assert.equal(cards.length, 3);
  }
});
