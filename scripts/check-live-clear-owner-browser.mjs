// Real Svelte preview/clear binding with synthetic deferred IPC and course data.
// No Tauri imports, native deletion, microphone or installed app interaction.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { createServer } from 'node:http';
import { liveClearOwnerCode, clearOwnerImports, clearOwnerStateCode } from '../tests/load-live-clear-owner.mjs';

const code = await liveClearOwnerCode();
const state = clearOwnerStateCode
  .replace('snapshot = emptyLiveSurfaceSnapshot(), busy = false, showSaveNotif = false',
    'snapshot = $state.raw(emptyLiveSurfaceSnapshot()), busy = $state(false), showSaveNotif = $state(false)')
  .replace('now = new Date(2026, 9, 8, 12)', 'now = $state(new Date(2026, 9, 8, 12))')
  .replace('lastSaved = null', 'lastSaved = $state.raw(null)')
  .replace('courseOptions = [', 'courseOptions = $state.raw([')
  .replace("{name:'Course B',day:4,period:2,room:'B202'}\n  ];", "{name:'Course B',day:4,period:2,room:'B202'}\n  ]);")
  .replace('selectedKey = courseKey(courseOptions[0])', 'selectedKey = $state(courseKey(courseOptions[0]))')
  .replace("overallSummary = 'existing summary', summaryDetailOpen = true, summaryViewIndex = 2, notice = null",
    "overallSummary = $state('existing summary'), summaryDetailOpen = $state(true), summaryViewIndex = $state(2), notice = $state(null)");
const component = `<script lang="ts">
  import {onDestroy,untrack} from 'svelte';
  ${clearOwnerImports}
  const resources = new ResourceScope();
  ${state}
  ${code.selected}
  ${code.identity}
  ${code.preview}
  ${code.binding}
  ${code.clear}
  window.clearProbe = {
    calls, configure:value=>{config=value;}, clear:executeClearCourseData,
    select:index=>{selectedKey=index==null?'__free_note__':courseKey(courseOptions[index]);},
    snapshot:(value,event=false)=>{snapshot=value;if(event)sessionEventVersion++;},
    saved:(value,preview=lastSaved)=>{showSaveNotif=value;lastSaved=preview;}, time:value=>{now=value;},
    replaceCourses:value=>{courseOptions=value;},
    get value(){return {snapshot,busy,notice,showSaveNotif};}
  };
  onDestroy(()=>resources.dispose());
</script>
<span id="clear-course">{snapshot.course?.course_name||''}</span>
<span id="clear-count">{snapshot.transcript_line_count}</span>
<span id="clear-notice">{notice?.text||''}</span>
<span id="clear-busy">{busy?'busy':'idle'}</span>`;
const source = `
import {mount,unmount,tick} from 'svelte';
import Probe from 'test-live-clear-component';
const result={checks:[],error:null};
const check=(condition,text)=>{if(!condition)throw new Error(text);result.checks.push(text);};
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const preview=name=>({active:false,session_id:null,course:{course_name:name},transcript_line_count:3,visible_lines:[{text:name+' transcript'}],summaries:[]});
const recording=id=>({...preview('Recording '+id),active:true,session_id:id});
const text=id=>document.getElementById(id)?.textContent;
const app=mount(Probe,{target:document.getElementById('probe')});
let closed=false;
try {
  await tick();await tick();
  const owner=window.clearProbe, peeks=[], clears=[];
  owner.configure({peek:course=>{const d={course,...deferred()};peeks.push(d);return d.promise;},
    clear:course=>{const d={course,...deferred()};clears.push(d);return d.promise;}});
  owner.select(1);await tick();owner.select(0);await tick();
  check(peeks.length===2,'course changes read their own previews');
  const oldA=peeks[1], oldB=peeks[0];
  const clearing=owner.clear();await tick();owner.select(1);await tick();
  check(clears.length===1&&clears[0].course.course_name==='Course A','deletion captures its admitted course');
  check(peeks.length===2,'selection while clearing defers preview IO');
  clears[0].resolve();await clearing;await tick();
  check(peeks.length===3&&peeks[2].course.course_name==='Course B','clear completion resumes the latest skipped selection');
  check(text('clear-busy')==='idle'&&text('clear-notice')==='','retired clear leaves no wrong-course success notice');
  peeks[2].resolve(preview('Course B'));await tick();await tick();
  check(text('clear-course')==='Course B'&&text('clear-count')==='3','latest selected course reaches the actual DOM');
  oldA.resolve(preview('Deleted Course A'));oldB.resolve(preview('Old Course B'));await tick();await tick();
  check(text('clear-course')==='Course B','pre-delete and pre-selection responses cannot revive stale previews');
  const failed=owner.clear();await tick();owner.select(0);await tick();
  clears[1].reject(new Error('retired Course B error'));await failed;await tick();
  check(text('clear-notice')==='','retired clear failure does not replace current course notice');
  check(peeks.length===4&&peeks[3].course.course_name==='Course A','failed clear also resumes skipped preview');
  peeks[3].resolve(preview('Course A'));await tick();await tick();
  const same=owner.clear();await tick();
  check(text('clear-course')==='Course A','pending current deletion preserves displayed content');
  clears[2].resolve();await same;await tick();
  check(text('clear-count')==='0'&&text('clear-notice').includes('Course A'),'current deletion clears DOM and presents its acknowledgment');
  check(peeks.length===5,'idle return reads the post-mutation cache once');
  peeks[4].resolve({active:false,session_id:null,course:null,transcript_line_count:0,summaries:[]});await tick();await tick();
  check(text('clear-count')==='0','empty post-delete cache keeps removed content absent');
  const freeNote=owner.clear();await tick();owner.select(null);await tick();
  clears[3].resolve();await freeNote;await tick();
  check(peeks.length===5&&text('clear-notice')==='','free-note after deletion resumes without course IO or stale notice');
  owner.select(0);await tick();const beforeRecording=peeks[5];
  const displaced=owner.clear();await tick();owner.snapshot(recording('new'),true);await tick();
  clears[4].resolve();await displaced;await tick();
  check(text('clear-course')==='Recording new'&&text('clear-count')==='3','late clear preserves new recording DOM');
  check(peeks.length===6,'active recording does not trigger preview IO on busy release');
  beforeRecording.resolve(preview('Old Course A'));await tick();await tick();
  check(text('clear-course')==='Recording new','recording activation rejects an earlier preview');
  owner.saved(true);owner.snapshot({...recording('new'),active:false},true);await tick();
  owner.select(1);await tick();
  check(peeks.length===6&&text('clear-course')==='Recording new','saved badge holds completed recording while selection changes');
  owner.saved(false);await tick();
  check(peeks.length===7&&peeks[6].course.course_name==='Course B','badge expiry resumes current course preview');
  peeks[6].resolve(preview('Course B'));await tick();await tick();
  const fixed=peeks.length;
  for(let i=0;i<100;i++){
    owner.replaceCourses([{name:'Course A',day:4,period:1,room:'A'+i},{name:'Course B',day:4,period:2,room:'B'+i}]);
    owner.snapshot({...preview('Course B'),update_revision:i});owner.time(new Date(2026,9,8,13, i%60));await tick();
  }
  check(peeks.length===fixed,'one hundred unchanged course/day and inactive snapshot replacements do not reread');
  const sameDay=owner.clear();await tick();owner.select(0);await tick();owner.select(1);await tick();
  clears[5].resolve();await sameDay;await tick();
  check(text('clear-count')==='0','return to the same course after busy selection changes clears its owned display');
  check(peeks.length===fixed+1,'busy A to B to A navigation resumes one current preview');
  peeks.at(-1).resolve(preview('Course B'));await tick();await tick();
  owner.time(new Date(2026,9,9,0));await tick();
  check(peeks.length===fixed+2,'new day requests a new course cache');
  peeks.at(-1).resolve(preview('Course B'));await tick();await tick();
  owner.saved(true,{summary_markdown:'existing saved preview'});await tick();
  const withBadge=owner.clear();await tick();clears[6].resolve();await withBadge;await tick();
  check(text('clear-count')==='0'&&text('clear-notice').includes('Course B'),'existing saved badge permits the current owned clear acknowledgment');
  owner.snapshot(preview('Course B'));await tick();const supersededSave=owner.clear();await tick();
  owner.saved(true,{summary_markdown:'new saved preview'});clears[7].resolve();await supersededSave;await tick();
  check(text('clear-count')==='3'&&text('clear-notice')==='','a newer save preview retires an old clear without replacing its DOM');
  owner.saved(false);await tick();
  const lateRead=peeks.at(-1), closingClear=owner.clear();await tick();await unmount(app);closed=true;
  clears[8].resolve();lateRead.resolve(preview('late closed preview'));await closingClear;await tick();
  check(document.getElementById('probe').textContent==='','unmount removes probe DOM and queued work cannot restore it');
  const afterClose=owner.calls.length;await owner.clear();
  check(owner.calls.length===afterClose,'closed page issues no new deletion or preview');
}catch(error){result.error=error.stack||String(error);}
if(!closed) await unmount(app);
const output=document.createElement('pre');output.textContent=JSON.stringify(result,null,2);document.body.append(output);
await fetch('/result',{method:'POST',body:JSON.stringify(result)});
`;
const bundle = await build({stdin:{contents:source,loader:'js',resolveDir:process.cwd()},bundle:true,write:false,
  platform:'browser',format:'esm',conditions:['browser'],define:{controlsBusy:'controls.busy'},
  plugins:[{name:'live-clear-owner-probe',setup(plugin){
    plugin.onResolve({filter:/^test-live-clear-component$/},()=>({path:'component',namespace:'live-clear'}));
    plugin.onLoad({filter:/.*/,namespace:'live-clear'},()=>({contents:compile(component,{filename:'live-clear-probe.svelte',generate:'client'}).js.code,loader:'js',resolveDir:process.cwd()}));
  }}],
});
let completed=null;
const server=createServer(async(request,response)=>{
  if(request.url==='/probe.js'){response.setHeader('Content-Type','text/javascript');response.end(bundle.outputFiles[0].text);}
  else if(request.url==='/result'&&request.method==='POST'){
    const chunks=[];for await(const chunk of request)chunks.push(chunk);
    completed=JSON.parse(Buffer.concat(chunks).toString());console.log(JSON.stringify({passed:completed.checks.length,error:completed.error}));response.end('ok');
  }else if(request.url==='/result'){response.setHeader('Content-Type','application/json');response.end(JSON.stringify(completed));}
  else if(request.url==='/'){response.setHeader('Content-Type','text/html');response.end('<!doctype html><meta charset="utf-8"><title>LIVE clear owner verification</title><div id="probe"></div><script type="module" src="/probe.js"></script>');}
  else{response.statusCode=404;response.end();}
});
server.listen(0,'127.0.0.1',()=>console.log('http://127.0.0.1:'+server.address().port+'/'));
