import assert from "node:assert/strict";
import { loadWhiteboardLayout } from "../tests/load-whiteboard-layout.mjs";
import { loadWhiteboardScoring } from "../tests/load-whiteboard-scoring.mjs";
import { boardFixture } from "../tests/fixtures/whiteboard-layout-cases.mjs";

const [before,after,countBefore,countAfter] = await Promise.all([
  loadWhiteboardLayout("intersection"),loadWhiteboardLayout(),loadWhiteboardScoring("intersection"),loadWhiteboardScoring(),
]);
const option = {fallbackBoardTitle:"知識整理",externalNodeLabel:"外部"};
const median = values => values.sort((a,b)=>a-b)[4];
function measure(engine, board, iterations) {
  const started = performance.now();
  for(let i=0;i<iterations;i++) engine.compute(board,option);
  return (performance.now()-started)/iterations;
}
console.log("Baseline: actual shared layout immediately before first-side orientation rejection.");
for(const config of [
  {count:18,edges:36,labelled:1,iterations:60}, {count:75,edges:160,labelled:1,iterations:12},
  {count:96,edges:192,labelled:1,iterations:8}, {count:75,edges:160,labelled:0.25,iterations:30},
  {count:75,edges:160,labelled:0,iterations:100},
]) {
  const board = boardFixture(config), times = [[],[]];
  const old = before.compute(board,option), current = after.compute(board,option);
  assert.equal(JSON.stringify(current),JSON.stringify(old));
  for(let sample=-3;sample<9;sample++) {
    for(const index of sample%2 ? [1,0] : [0,1]) {
      const ms = measure(index ? after : before,board,config.iterations);
      if(sample>=0) times[index].push(ms);
    }
  }
  countBefore.reset(); countAfter.reset();
  assert.deepEqual(countAfter.layout.compute(board,option),countBefore.layout.compute(board,option));
  console.log(JSON.stringify({...config,msPerCompleteLayout:times.map(median),
    orientations:[countBefore.stats.orientations,countAfter.stats.orientations],
    preciseSegmentTests:[countBefore.stats.intersections,countAfter.stats.intersections],
    candidateRects:[countBefore.stats.candidateRects,countAfter.stats.candidateRects],
    renderedNodes:current.nodes.length,renderedEdges:current.edges.length,completeJsonBytesMatch:true}));
}
console.log("Nine alternating medians after three warmups; uninstrumented complete layout. Counts use separate injected runs. Input generation, wrapper cache, DOM/WebKit, IPC, model/audio, application RSS/GPU and comparison excluded. All graph content, candidates and label positions retained. No universal speedup claim.");
