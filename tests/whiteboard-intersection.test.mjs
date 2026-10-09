import test from "node:test";
import assert from "node:assert/strict";
import { loadWhiteboardLayout } from "./load-whiteboard-layout.mjs";
import { loadWhiteboardScoring } from "./load-whiteboard-scoring.mjs";
import { boardFixture, layoutCases } from "./fixtures/whiteboard-layout-cases.mjs";
import { duplicateBoard, reservedBoard, prototypeBoard, phantomBoard } from "./fixtures/whiteboard-identity-cases.mjs";

const [before, after, oldScorer, scorer] = await Promise.all([
  loadWhiteboardLayout("intersection"), loadWhiteboardLayout(process.env.SELAH_WHITEBOARD_INTERSECTION_BEFORE ? "intersection" : false),
  loadWhiteboardScoring("intersection"), loadWhiteboardScoring(process.env.SELAH_WHITEBOARD_INTERSECTION_BEFORE ? "intersection" : false),
]);
let state = 1;
const random = max => { state = (Math.imul(state,1664525) + 1013904223) >>> 0; return state % max; };

test("32768 crossing cases preserve zero, collinear, reversed and non-finite orientation semantics", () => {
  for (let i = 0; i < 32768; i++) {
    const args = Array.from({length:8}, () => random(201) - 100);
    if (i % 4 === 0) args[2] = args[0];
    if (i % 5 === 0) args[3] = args[1];
    if (i % 7 === 0) [args[4],args[5]] = args.slice(0,2);
    if (i % 11 === 0) [args[6],args[7]] = args.slice(2,4);
    assert.equal(scorer.cross(...args),oldScorer.cross(...args),String(i));
  }
  for (const value of [0,-0,NaN,Infinity,-Infinity,Number.MAX_VALUE,-Number.MAX_VALUE,Number.MIN_VALUE,-Number.MIN_VALUE]) {
    for (let field = 0; field < 8; field++) {
      const args = [0,0,10,10,0,10,10,0]; args[field] = value;
      assert.equal(scorer.cross(...args),oldScorer.cross(...args),`${value} at ${field}`);
    }
  }
});

test("4096 rectangle intersections keep endpoint, touching and degenerate behavior", () => {
  for (let i = 0; i < 4096; i++) {
    const x = random(201) - 100, y = random(201) - 100;
    const rect = {x1:x,y1:y,x2:x+random(40),y2:y+random(40)};
    const seg = {x1:random(201)-100,y1:random(201)-100,x2:random(201)-100,y2:random(201)-100};
    if (i % 4 === 0) {seg.x1 = rect.x1; seg.y1 = rect.y2;}
    assert.equal(scorer.intersect(rect,seg),oldScorer.intersect(rect,seg));
  }
});

test("complete layouts, topics, chips and all label coordinates match the immediate predecessor", () => {
  const cases = [...layoutCases(),duplicateBoard(),reservedBoard(),prototypeBoard(),phantomBoard(),
    ...Array.from({length:32}, (_, i) => boardFixture({count:[12,24,75,96][i%4],edges:i*7+1,
      seed:i+1,hierarchy:["backend","explicit","legacy"][i%3],labelled:[0,0.25,1][i%3]}))];
  const options = [undefined,{}, {fallbackBoardTitle:"知識 | 🙂",externalNodeLabel:"外部"}, {topicIds:[]},
    {topicIds:["n0"]},{topicIds:["n1","n2"]},{topicIds:["n2","n0","n0"]},{topicIds:["missing"]}];
  for (const [i,board] of cases.entries()) {
    const original = structuredClone(board);
    assert.deepEqual(after.topics(board),before.topics(board),`topics ${i}`);
    for (const option of options) {
      const result = after.compute(board,option), old = before.compute(board,option);
      assert.deepEqual(result,old,`layout ${i}`);
      assert.equal(JSON.stringify(result),JSON.stringify(old),`complete bytes ${i}`);
    }
    assert.deepEqual(board,original,`input ${i}`);
  }
});

test("same-side rejection saves actual orientations while retaining all candidates and penalties", () => {
  const separated = [0,0,10,0,0,1,10,1];
  oldScorer.reset(); scorer.reset();
  assert.equal(scorer.cross(...separated),oldScorer.cross(...separated));
  assert.equal(oldScorer.stats.orientations,4); assert.equal(scorer.stats.orientations,2);
  for (const config of [{count:18,edges:36},{count:75,edges:160},{count:96,edges:192}]) {
    const board = boardFixture(config); oldScorer.reset(); scorer.reset();
    assert.deepEqual(scorer.layout.compute(board),oldScorer.layout.compute(board));
    for (const key of ["overlaps","intersections","candidateRects"]) assert.equal(scorer.stats[key],oldScorer.stats[key]);
    assert.ok(scorer.stats.orientations < oldScorer.stats.orientations);
  }
});
