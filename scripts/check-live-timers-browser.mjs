// Actual Svelte-client timer ownership probe; all data, clocks and audio actions
// are synthetic. It never imports Tauri or starts native recording.
import { build } from 'esbuild';
import { compile } from 'svelte/compiler';
import { resolve } from 'node:path';
import { createServer } from 'node:http';
import { liveTimerCode, timerBoundaryCode, timerStateCode } from '../tests/load-live-timers.mjs';

const code = await liveTimerCode();
const state = timerStateCode
  .replace('snapshot = emptyLiveSurfaceSnapshot()', 'snapshot = $state.raw(emptyLiveSurfaceSnapshot())')
  .replace('lastSaved = null', 'lastSaved = $state.raw(null)')
  .replace('showSaveNotif = false', 'showSaveNotif = $state(false)')
  .replace('notice = null', 'notice = $state(null)')
  .replace('now = new Date(0)', 'now = $state(new Date(0))');
const component = `<script lang="ts">
  import { onDestroy } from 'svelte';
  import { ResourceScope } from ${JSON.stringify(resolve('src/lib/resourceScope.ts'))};
  import { emptyLiveSurfaceSnapshot, liveSavedPreview } from ${JSON.stringify(resolve('src/lib/views/live/liveTranscript.ts'))};
  import { isLiveBusy } from ${JSON.stringify(resolve('src/lib/views/live/liveFinish.ts'))};
  import { calls } from 'test-live-timer-boundaries';
  const resources = new ResourceScope();
  ${state}
  let visible = $state(false), guardEffectRuns = 0;
  ${code.declarations}
  ${code.functions}
  ${code.binding.replace('$effect(() => {', '$effect(() => { guardEffectRuns += 1;')}
  $effect(() => { applyLiveSurfacePolicy(visible, !visible, snapshot.active); });
  window.timerProbe = {
    notice: text => setMessage('success', text),
    saved: text => rememberSaved({ saved:true, summary_markdown:text, snapshot:{session_id:snapshot.session_id||'a',update_revision:1} }),
    setSnapshot: value => { snapshot = value; },
    visible: value => { visible = value; },
    listening: value => { sttListening = value; sttPhase = value ? 'listening' : 'idle'; },
    markListening: markLiveListeningStarted,
    get value() { return { notice, showSaveNotif, now:now.getTime(), guardEffectRuns, active:snapshot.active }; }
  };
  onDestroy(() => { ${code.destroy} });
</script>
<span id="probe-notice">{notice?.text||''}</span>
<span id="probe-save">{showSaveNotif?'saved':''}</span>
<span id="probe-clock">{now.getTime()}</span>`;

const source = `
import { mount, unmount, tick } from 'svelte';
import Probe from 'test-live-timer-component';
import { timers, calls, fire, liveTimers, setTime } from 'test-live-timer-boundaries';
const result = { checks:[], error:null };
const check = (condition,text) => { if(!condition) throw new Error(text); result.checks.push(text); };
const active = id => ({active:true,session_id:id,finish_phase:null});
const find = (kind,delay) => liveTimers().find(t=>t.kind===kind&&t.delay===delay);
const text = id => document.getElementById(id)?.textContent;
const app = mount(Probe,{target:document.getElementById('probe')});
try {
  await tick();
  const owner = window.timerProbe;
  check(liveTimers().length===0,'inactive hidden page has no timer');
  owner.notice('same notice'); await tick();
  const firstNotice=find('timeout',4000);
  owner.notice('same notice'); await tick();
  const latestNotice=find('timeout',4000);
  fire(firstNotice.id); await tick();
  check(text('probe-notice')==='same notice','old equal-text notice callback preserves current DOM');
  fire(latestNotice.id); await tick();
  check(text('probe-notice')==='','current notice callback removes its own DOM');
  owner.saved('first'); await tick(); const firstSave=find('timeout',6000);
  owner.saved('latest'); await tick(); const latestSave=find('timeout',6000);
  fire(firstSave.id); await tick();
  check(text('probe-save')==='saved','old saved callback preserves replacement badge DOM');
  fire(latestSave.id); await tick();
  check(text('probe-save')==='','current saved callback removes its own badge');
  owner.visible(true); await tick(); const focus=find('interval',60000);
  check(!!focus,'visible page owns course-clock interval');
  owner.visible(false); await tick(); const hiddenClock=text('probe-clock');
  setTime(200000); fire(focus.id); await tick();
  check(text('probe-clock')===hiddenClock,'hidden page rejects already queued course-clock tick');
  owner.visible(true); await tick(); const newFocus=find('interval',60000);
  check(newFocus.id!==focus.id,'visible return owns a fresh course-clock interval');
  check(text('probe-clock')==='200000','visible return immediately refreshes clock DOM');
  owner.visible(false); owner.setSnapshot(active('a')); owner.listening(true); await tick(); owner.markListening();
  const guard=find('interval',60000), guardRuns=owner.value.guardEffectRuns;
  check(liveTimers().length===1,'hidden recording retains exactly one lifecycle timer');
  for(let i=0;i<100;i++) { owner.setSnapshot({...active('a'),transcript_line_count:i}); await tick(); }
  check(find('interval',60000).id===guard.id,'one hundred active snapshots preserve timer identity');
  check(owner.value.guardEffectRuns===guardRuns,'primitive active derived skips one hundred redundant effect bodies');
  setTime(800000); fire(guard.id); await tick(); await tick();
  check(calls.filter(c=>c[0]==='pause').length===1,'hidden recording retains ten-minute pause check with fake audio action');
  owner.setSnapshot({active:false}); await tick();
  check(liveTimers().length===0,'inactive recording releases lifecycle timer');
  owner.setSnapshot(active('b')); owner.markListening(); await tick(); const replacement=find('interval',60000);
  setTime(1400000); fire(guard.id); await tick();
  check(calls.filter(c=>c[0]==='pause').length===1,'retired recording tick cannot run a replacement lifecycle check');
  fire(replacement.id); await tick(); await tick();
  check(calls.filter(c=>c[0]==='pause').length===2,'replacement recording owns its normal lifecycle check');
  owner.saved('queued at close'); owner.notice('queued at close'); owner.visible(true); await tick();
  const queued=[...liveTimers()], before=JSON.stringify(owner.value);
  await unmount(app);
  check(liveTimers().length===0,'unmount releases all four timer kinds');
  check(document.getElementById('probe').textContent==='','unmount removes probe DOM');
  for(const timer of queued) fire(timer.id); await tick();
  check(JSON.stringify(owner.value)===before,'callbacks queued before unmount cannot mutate retired state');
  check(queued.every(t=>t.clears===1),'resource disposal and explicit cleanup release timers once');
} catch(error) {
  result.error=error.stack||String(error);
  await unmount(app).catch(()=>{});
}
window.timerProbeResult=result;
const output=document.createElement('pre'); output.textContent=JSON.stringify(result,null,2); document.body.append(output);
await fetch('/result',{method:'POST',body:JSON.stringify(result)});
`;
const bundle = await build({ stdin: { contents:source,loader:'js',resolveDir:process.cwd() },
  bundle:true,write:false,platform:'browser',format:'esm',conditions:['browser'],
  define:{setTimeout:'timerSetTimeout',clearTimeout:'timerClearTimeout',setInterval:'timerSetInterval',clearInterval:'timerClearInterval',
    Date:'OwnerDate',controlsBusy:'controls.busy',sttBooting:'controls.booting'},
  inject:['test-live-timer-boundaries'],
  plugins:[{name:'actual-live-timer-probe',setup(plugin){
    plugin.onResolve({filter:/^test-live-timer-boundaries$/},()=>({path:'timers',namespace:'live-timer'}));
    plugin.onLoad({filter:/.*/,namespace:'live-timer'},()=>({contents:timerBoundaryCode,loader:'js'}));
    plugin.onResolve({filter:/^test-live-timer-component$/},()=>({path:'component',namespace:'live-owner'}));
    plugin.onLoad({filter:/.*/,namespace:'live-owner'},()=>({contents:compile(component,{filename:'live-timer-probe.svelte',generate:'client'}).js.code,loader:'js',resolveDir:process.cwd()}));
  }}],
});
let completed = null;
const server=createServer(async(request,response)=>{
  if(request.url==='/probe.js'){response.setHeader('Content-Type','text/javascript');response.end(bundle.outputFiles[0].text);}
  else if(request.url==='/result'&&request.method==='POST'){
    const chunks=[];for await(const chunk of request)chunks.push(chunk);
    completed=JSON.parse(Buffer.concat(chunks).toString());console.log(JSON.stringify({passed:completed.checks.length,error:completed.error}));response.end('ok');
  }else if(request.url==='/result'){response.setHeader('Content-Type','application/json');response.end(JSON.stringify(completed));}
  else if(request.url==='/'){response.setHeader('Content-Type','text/html');response.end('<!doctype html><meta charset="utf-8"><title>LIVE timer ownership verification</title><div id="probe"></div><script type="module" src="/probe.js"></script>');}
  else{response.statusCode=404;response.end();}
});
server.listen(0,'127.0.0.1',()=>console.log('http://127.0.0.1:'+server.address().port+'/'));
