import test from "node:test";
import assert from "node:assert/strict";
import { loadLiveDeck } from "./load-live-deck.mjs";

const { Component, render } = await loadLiveDeck("LiveTranscriptStage");
const lines = Array.from({ length: 30 }, (_, i) => ({ at: "10:00", text: `ORIGINAL_${i}_END` }));
const captions = Array.from({ length: 12 }, (_, i) => ({
  segment_id: i + 1, corrected: `CORRECTION_${i}_END`, translation: `TRANSLATION_${i}_END`, ready: true,
}));
const props = {
  pageLoading: false, hasContent: true,
  snapshot: { active: true, transcript_line_count: lines.length },
  partialText: "", lastSaved: null, showSaveNotif: false,
  visibleLines: lines, hiddenLineCount: 0, renderMd: text => text,
  refinedCaptions: captions, captionTranslation: true, visible: true, onToggleCaptionTranslation: () => {},
};

test("all translated batches remain accessible before the current original and partial", () => {
  for (const partialText of ["", "CURRENT_PARTIAL"]) {
    const html = render(Component, { props: { ...props, partialText } }).body;
    for (const line of lines) assert.equal(html.split(line.text).length - 1, 1);
    for (const caption of captions) assert.ok(html.includes(caption.translation));
    assert.ok(html.includes('aria-label="字幕翻訳の履歴"'));
    assert.ok(html.indexOf("TRANSLATION_11_END") < html.indexOf("ORIGINAL_29_END"));
    if (partialText) assert.ok(html.indexOf("ORIGINAL_29_END") < html.indexOf(partialText));
  }
});

test("disabling translations preserves the complete ordered original transcript", () => {
  const html = render(Component, { props: { ...props, captionTranslation: false } }).body;
  assert.equal(html.includes("TRANSLATION_"), false);
  assert.equal(html.includes("caption-history"), false);
  const positions = lines.map(line => html.indexOf(line.text));
  assert.ok(positions.every((position, index) => position >= 0 && (index === 0 || position > positions[index - 1])));
});
