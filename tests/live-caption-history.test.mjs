import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { applyRefinedCaption } = await loadTypeScript("src/lib/views/live/liveCaptionRefine.ts");

test("failed and unchanged refinements do not duplicate originals in translation history", () => {
  const items = [];
  const source = "期待値を定義します。\n次に分散を説明します。";
  for (const ready of [true, false]) {
    const result = applyRefinedCaption(items, { segment_id: 1, source,
      corrected: "期待値を定義します。 次に分散を説明します。", ready });
    assert.equal(result.items, items);
    assert.equal(result.followDelta, 0);
  }
});

test("useful corrections and complete translations still appear once", () => {
  const corrected = applyRefinedCaption([], { segment_id: 1, source: "確立を定義します。",
    corrected: "確率を定義します。", ready: true });
  assert.equal(corrected.items.length, 1);
  const translated = applyRefinedCaption(corrected.items, { segment_id: 1,
    source: "確立を定義します。", corrected: "確率を定義します。",
    translation: "定义概率。", ready: true });
  assert.equal(translated.items.length, 1);
  assert.equal(translated.items[0].translation, "定义概率。");
  assert.equal(translated.followDelta, 1);
  const lateFallback = applyRefinedCaption(translated.items, { segment_id: 1,
    source: "確立を定義します。", corrected: "確立を定義します。", ready: false });
  assert.equal(lateFallback.items, translated.items);
});

test("one inference batch renders complete source and translation pairs separately", () => {
  const sentences = [
    { source: "最後に、条件付き確率の関係を整理してください。", corrected: "最後に、条件付き確率の関係を整理してください。", translation: "最后请整理条件概率的关系。" },
    { source: "第一の話題は確率です。", corrected: "第一の話題は確率です。", translation: "第一个话题是概率。" },
    { source: "サイコロを一回投げます。", corrected: "サイコロを一回投げます。", translation: "掷一次骰子。" },
  ];
  const event = { segment_id: 42, source: sentences.map(s=>s.source).join("\n"),
    corrected: sentences.map(s=>s.corrected).join(" "), translation: sentences.map(s=>s.translation).join(" "),
    sentences, ready: true };
  const result = applyRefinedCaption([], event);
  assert.equal(result.items.length, 3);
  assert.equal(result.followDelta, 3);
  for (const [index, sentence] of sentences.entries()) {
    assert.equal(result.items[index].segment_id, 42);
    assert.equal(result.items[index].sentence_index, index);
    assert.equal(result.items[index].corrected, sentence.corrected);
    assert.equal(result.items[index].translation, sentence.translation);
  }
  const replay = applyRefinedCaption(result.items, event);
  assert.equal(replay.items.length, 3);
  assert.equal(replay.followDelta, 0);
  const fallback = applyRefinedCaption(replay.items, { segment_id: 42, source: event.source,
    corrected: event.source, ready: false });
  assert.equal(fallback.items, replay.items);
});

test("sentence history stays bounded with distinct keys inside a batch", () => {
  let items = [];
  for (let batch = 1; batch <= 18; batch++) {
    items = applyRefinedCaption(items, { segment_id: batch, ready: true, sentences: Array.from({ length: 4 }, (_,i)=>({
      source: `Sentence ${batch}/${i}.`, corrected: `Sentence ${batch}/${i}.`, translation: `Translation ${batch}/${i}.`,
    })) }).items;
  }
  assert.equal(items.length, 48);
  assert.equal(new Set(items.map(i=>`${i.segment_id}:${i.sentence_index}`)).size,48);
  assert.deepEqual([...new Set(items.map(i=>i.segment_id))],Array.from({length:12},(_,i)=>i+7));
});

test("one rejected sentence does not hide valid independent sentence translations", () => {
  const result=applyRefinedCaption([], {segment_id:42,ready:true,translation:null,sentences:[
    {source:"関係を整理してください。",corrected:"関係を整理してください。",translation:"请整理关系。"},
    {source:"第一の話題は確率です。",corrected:"第一の話題は確率です。",translation:"第一个话题是概率。"},
    {source:"一から六までです。",corrected:"一から六までです。",translation:null},
  ]});
  assert.equal(result.items.length,2);
  assert.deepEqual(result.items.map(item=>item.sentence_index),[0,1]);
  assert.deepEqual(result.items.map(item=>item.translation),["请整理关系。","第一个话题是概率。"]);
});
