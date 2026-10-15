// Actual LIVE caption components and follow binding, without app IPC or audio.
// Open the printed localhost URL. Ctrl-C stops this isolated test server.
import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { createServer } from "node:http";

const live = await readFile("src/lib/views/Live.svelte", "utf8");
const follow = live.match(/  function bindLiveTranscriptFollow\(\) {[\s\S]*?\n  }\n/)?.[0];
if (!follow) throw new Error("Actual LIVE transcript follow binding not found");
const component = `<script lang="ts">
  import {onMount,onDestroy,tick} from "svelte";
  import Stage from ${JSON.stringify(resolve("src/lib/views/live/LiveTranscriptStage.svelte"))};
  import {ResourceScope} from ${JSON.stringify(resolve("src/lib/resourceScope.ts"))};
  import {LiveTranscriptFollow} from ${JSON.stringify(resolve("src/lib/views/live/liveTranscriptFollow.ts"))};
  const resources=new ResourceScope();
  let harnessVisible=$state(true);
  let scrollEl=$state<HTMLElement|null>(null);
  let visibleLines=$state.raw(Array.from({length:30},(_,i)=>({at:'10:00',text:'Original line '+i})));
  let snapshot=$state.raw({active:true,session_id:'a',transcript_line_count:30});
  let refinedCaptions=$state.raw(Array.from({length:12},(_,i)=>({segment_id:i+1,corrected:'Previous source '+i,translation:'Complete translation '+i,ready:true})));
  let captionTranslation=$state(true),refinedFollowCount=$state(12),partialText=$state('');
  let liveSurfaceVisible=$derived(harnessVisible);
  const whiteboardExpanded=false,summaryDetailOpen=false,sttListening=true,autoFollow=true;
  ${follow}
  bindLiveTranscriptFollow();
  onDestroy(()=>resources.dispose());
  const paint=async()=>{await tick();await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));};
  const query=(selector:string)=>document.querySelector<HTMLElement>(selector)!;
  const intersects=(item:HTMLElement,pane:HTMLElement)=>{const r=item.getBoundingClientRect(),v=pane.getBoundingClientRect();return r.bottom>v.top&&r.top<v.bottom;};
  const latestOriginal=()=>Array.from(document.querySelectorAll<HTMLElement>('.lyric-line:not(.partial)')).at(-1)!;
  onMount(()=>{void run();});
  async function run(){
    const checks:string[]=[];
    const check=(ok:unknown,label:string)=>{if(!ok)throw new Error(label);checks.push(label);};
    try{
      await paint();
      let history=query('.caption-history');
      check(intersects(latestOriginal(),scrollEl!), 'latest original is visible with twelve translated batches');
      check(history.clientHeight<=220&&history.scrollHeight>history.clientHeight,'translation history has a bounded independent viewport');
      check(intersects(query('.lyric-refine:last-child'),history),'newest translated batch is visible');
      visibleLines=[...visibleLines,{at:'10:01',text:'LATEST COMPLETED ORIGINAL'}];
      snapshot={...snapshot,transcript_line_count:31};
      refinedCaptions=[...refinedCaptions.slice(1),{segment_id:13,corrected:'Newest source',translation:'NEWEST COMPLETE TRANSLATION',ready:true}];
      refinedFollowCount++;
      await paint();
      check(latestOriginal().textContent?.includes('LATEST COMPLETED ORIGINAL')&&intersects(latestOriginal(),scrollEl!),'new final stays visible after translation updates');
      check(intersects(query('.lyric-refine:last-child'),history),'translation history follows an arriving batch');
      partialText='CURRENT SPEECH';await paint();
      check(intersects(latestOriginal(),scrollEl!)&&intersects(query('.partial'),scrollEl!),'latest original and ongoing speech remain visible together');
      history.scrollTop=0;history.dispatchEvent(new Event('scroll'));await tick();
      refinedCaptions=[...refinedCaptions.slice(1),{segment_id:14,corrected:'Next source',translation:'Next complete translation',ready:true}];
      refinedFollowCount++;await paint();
      check(history.scrollTop===0,'reading older translations is not interrupted by a new batch');
      captionTranslation=false;await paint();
      check(!document.querySelector('.caption-history')&&intersects(latestOriginal(),scrollEl!),'disabling translations keeps current speech visible');
      captionTranslation=true;await paint();history=query('.caption-history');
      check(intersects(query('.lyric-refine:last-child'),history),'reopening translations follows the newest batch');
      check(document.querySelectorAll('.lyric-line:not(.partial)').length===31,'each original line appears exactly once');
      captionTranslation=false;await paint();
      harnessVisible=false;captionTranslation=true;
      refinedCaptions=Array.from({length:12},(_,i)=>({segment_id:20+i,corrected:'Source arriving in the background '+i,translation:'Background translation '+i,ready:true}));
      refinedFollowCount+=12;await paint();
      harnessVisible=true;await paint();history=query('.caption-history');
      check(intersects(query('.lyric-refine:last-child'),history),'history first mounted in the background follows its latest translation on return');
      history.scrollTop=0;history.dispatchEvent(new Event('scroll'));await paint();
      harnessVisible=false;await paint();
      refinedCaptions=[...refinedCaptions.slice(1),{segment_id:40,corrected:'Next background source',translation:'Next background translation',ready:true}];
      refinedFollowCount++;await paint();harnessVisible=true;await paint();
      check(history.scrollTop===0,'returning to LIVE preserves manually selected translation history');
      history.scrollTop=history.scrollHeight;history.dispatchEvent(new Event('scroll'));await paint();
      harnessVisible=false;await paint();
      refinedCaptions=[...refinedCaptions.slice(1),{segment_id:41,corrected:'Newest background source',translation:'NEWEST BACKGROUND TRANSLATION',ready:true}];
      refinedFollowCount++;await paint();harnessVisible=true;await paint();
      check(intersects(query('.lyric-refine:last-child'),history),'resumed following catches up after background updates');
      await fetch('/result',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({checks})});
      query('#result').textContent=checks.length+' caption checks passed';
    }catch(error){
      const message=String(error);query('#result').textContent=message;
      await fetch('/result',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({checks,error:message})});
    }
  }
</script>
<main style:display={harnessVisible ? 'block' : 'none'} bind:this={scrollEl} style="width:min(700px,90vw);height:400px;overflow-y:auto;border:1px solid #aaa">
  <Stage pageLoading={false} hasContent={true} {snapshot} {partialText} lastSaved={null} showSaveNotif={false}
    {visibleLines} hiddenLineCount={0} renderMd={text=>text} {refinedCaptions} {captionTranslation} visible={harnessVisible} onToggleCaptionTranslation={()=>{captionTranslation=!captionTranslation;}}/>
</main>`;
const bundle = await build({
  stdin: { contents: `import {mount} from "svelte";import Harness from "caption-harness";mount(Harness,{target:document.querySelector('#probe')});`, resolveDir: process.cwd(), loader: "js" },
  bundle: true, write: false, platform: "browser", format: "esm", conditions: ["browser"],
  plugins: [{ name: "caption-client", setup(builder) {
    builder.onResolve({ filter: /^caption-harness$/ }, () => ({ path: "caption-harness", namespace: "caption-harness" }));
    builder.onLoad({ filter: /.*/, namespace: "caption-harness" }, () => ({
      contents: compile(component, { filename: "caption-harness.svelte", generate: "client" }).js.code,
      loader: "js", resolveDir: process.cwd(),
    }));
    builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
      contents: compile(await readFile(path, "utf8"), { filename: path, generate: "client", css: "injected" }).js.code,
      loader: "js", resolveDir: dirname(path),
    }));
  } }],
});
const server = createServer(async (request, response) => {
  if (request.url === "/probe.js") {
    response.setHeader("Content-Type", "text/javascript");response.end(bundle.outputFiles[0].text);
  } else if (request.url === "/result" && request.method === "POST") {
    const chunks=[];for await(const chunk of request) chunks.push(chunk);
    console.log(Buffer.concat(chunks).toString());response.end("ok");
  } else {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><meta charset="utf-8"><title>LIVE caption verification</title><style>body{font:14px system-ui;margin:20px;color:#263440;--text-primary:#263440;--text-tertiary:#647586;--accent:#416f76}*{animation-duration:0s!important;transition:none!important}</style><h1 id="result">Running caption checks</h1><div id="probe"></div><script type="module" src="/probe.js"></script>');
  }
});
server.listen(0,"127.0.0.1",()=>console.log(`http://127.0.0.1:${server.address().port}/`));
