import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { appendSttFinal, mergeSttText } = await loadTypeScript("src/lib/speechDraft.ts");

test("stopping after multiple VAD segments keeps every sentence in the draft", () => {
  const base = "Existing instructions";
  let committed = "";
  committed = appendSttFinal(committed, "First sentence.");
  assert.equal(mergeSttText(base, committed, "Second"), "Existing instructions\nFirst sentence. Second");
  committed = appendSttFinal(committed, "Second sentence.");
  committed = appendSttFinal(committed, "Final sentence.");
  assert.equal(mergeSttText(base, committed, ""), "Existing instructions\nFirst sentence. Second sentence. Final sentence.");
});

test("silence preserves existing text and whitespace does not replace earlier finals", () => {
  assert.equal(appendSttFinal("Earlier speech", "  "), "Earlier speech");
  assert.equal(mergeSttText("Typed draft", "", ""), "Typed draft");
  assert.equal(mergeSttText("Typed draft\n", " speech ", " tail "), "Typed draft\nspeech tail");
  assert.equal(mergeSttText("", "speech", ""), "speech");
});

test("identical speech from separate final segments remains in the submitted draft", () => {
  for (const phrase of ["はい。", "Yes.", "对。", "123"]) {
    let committed = "";
    for (let segment = 0; segment < 3; segment++) committed = appendSttFinal(committed, phrase);
    assert.equal(mergeSttText("", committed, ""), [phrase, phrase, phrase].join(" "));
  }
});
