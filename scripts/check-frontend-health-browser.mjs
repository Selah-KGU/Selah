// Execute the actual native diagnostic JS against real DOM nodes with a fake
// invoke sink. No native app, user page data, recording or Tauri runtime.
import { createServer } from 'node:http';
import { loadFrontendHealthProbe } from '../tests/load-frontend-health-probe.mjs';

const probe = await loadFrontendHealthProbe();
const baseline = await loadFrontendHealthProbe(process.argv[2] || 'tests/fixtures/frontend-health-probe-before.rs');
const source = `
const probe = new Function('document','NodeFilter',${JSON.stringify(probe)});
const baseline=${baseline ? `new Function('document','NodeFilter',${JSON.stringify(baseline)})` : 'null'};
const result={checks:[],counts:[],timings:[],error:null};
const check=(condition,text)=>{if(!condition)throw new Error(text);result.checks.push(text);};
let sent;
window.__SELAH_PREBOOT_LOGS__=[{type:'error'},{type:'info'},{type:'error'}];
window.__TAURI_INTERNALS__={invoke:(command,args)=>{sent={command,...args};return Promise.resolve();}};
const sink=root=>({readyState:document.readyState,visibilityState:document.visibilityState,
  getElementById:()=>root,querySelector:()=>null,createTreeWalker:(node,mask)=>document.createTreeWalker(node,mask)});
const textGetter=Object.getOwnPropertyDescriptor(Node.prototype,'textContent').get;
function inspect(root,label){
  const expected=textGetter.call(root).length;
  let aggregateReads=0;
  Object.defineProperty(root,'textContent',{configurable:true,get(){aggregateReads++;throw new Error('aggregate text read: '+label);}});
  sent=null;probe(sink(root),NodeFilter);
  check(sent.command==='frontend_health_report'&&sent.report.sequence===7,label+': exact native command and sequence');
  check(sent.report.rootTextLength===expected,label+': complete UTF-16 length equals native textContent');
  check(sent.report.rootChildren===root.childElementCount,label+': element count preserved');
  check(aggregateReads===0,label+': no aggregate text read');
  check(sent.report.errors===2&&!JSON.stringify(sent).includes('PRIVATE'),label+': local counts contain no page text');
  return expected;
}
function compare(root,label){
  if(!baseline)return;
  delete root.textContent;
  const doc=sink(root), before=[],after=[], measure=fn=>{
    const start=performance.now();for(let i=0;i<5;i++)fn(doc,NodeFilter);return (performance.now()-start)/5;
  };
  baseline(doc,NodeFilter);const previous=JSON.stringify(sent);probe(doc,NodeFilter);
  check(JSON.stringify(sent)===previous,label+': diagnostic payload equals the frozen previous script');
  for(let i=0;i<3;i++){baseline(doc,NodeFilter);probe(doc,NodeFilter);}
  for(let i=0;i<9;i++){
    if(i%2){after.push(measure(probe));before.push(measure(baseline));}
    else{before.push(measure(baseline));after.push(measure(probe));}
  }
  const median=values=>values.sort((a,b)=>a-b)[4];
  result.timings.push({label,beforeMs:median(before),currentMs:median(after)});
}
try{
  const app=document.createElement('div');app.id='app';document.getElementById('fixtures').append(app);
  app.append(document.createTextNode('PRIVATE 课程🌙 é\\nwith tabs\\t'),document.createComment('PRIVATE comment excluded'));
  const nested=document.createElement('span');nested.append('PRIVATE nested',document.createTextNode(''),document.createTextNode('\\ud800'));
  const hidden=document.createElement('div');hidden.style.display='none';hidden.append('PRIVATE hidden page');
  const style=document.createElement('style');style.append('/* PRIVATE style content */');
  const script=document.createElement('script');script.type='application/json';script.append('PRIVATE script content');
  app.append(nested,hidden,style,script);
  const host=document.createElement('span');host.append('PRIVATE light DOM');host.attachShadow({mode:'open'}).append('PRIVATE shadow text excluded');app.append(host);
  const expected=textGetter.call(app).length;
  probe(document,NodeFilter);
  check(sent.report.rootTextLength===expected,'actual mounted app root includes hidden/script/style/light text and excludes comments/shadow');
  inspect(app,'mixed mounted DOM');
  compare(app,'mixed mounted DOM');
  const empty=document.createElement('div');inspect(empty,'empty element');
  probe(sink(null),NodeFilter);
  check(sent.report.rootChildren===0&&sent.report.rootTextLength===0,'missing root reports zero without walking');
  const xml=document.implementation.createDocument(null,'app'),root=xml.documentElement;
  root.append(xml.createCDATASection('PRIVATE CDATA <nested>🌘'),xml.createTextNode('PRIVATE plain'),xml.createComment('PRIVATE excluded'));
  inspect(root,'XML text and CDATA');
  const fragment='PRIVATE caption 🌙 '.repeat(32),large=document.createElement('div'),batch=document.createDocumentFragment();
  for(let i=0;i<10000;i++){const child=document.createElement('span');child.append(fragment);batch.append(child);}large.append(batch);
  const units=inspect(large,'ten thousand complete text nodes');
  result.counts.push({nodes:10000,textUnits:units,aggregateReads:0});
  compare(large,'ten thousand complete text nodes');
  const recovery=sink(empty);recovery.querySelector=()=>({});probe(recovery,NodeFilter);
  check(sent.report.recovery===true,'recovery diagnostic remains present');
  window.__TAURI_INTERNALS__=undefined;
  probe({getElementById(){throw new Error('bridge missing must skip DOM');}},NodeFilter);
  check(true,'missing bridge performs no DOM traversal');
  document.getElementById('fixtures').replaceChildren();
}catch(error){result.error=error.stack||String(error);}
const output=document.createElement('pre');output.textContent=JSON.stringify(result,null,2);document.body.append(output);
await fetch('/result',{method:'POST',body:JSON.stringify(result)});
`;
let completed=null;
const server=createServer(async(request,response)=>{
  if(request.url==='/probe.js'){response.setHeader('Content-Type','text/javascript');response.end(source);}
  else if(request.url==='/result'&&request.method==='POST'){
    const chunks=[];for await(const chunk of request)chunks.push(chunk);
    completed=JSON.parse(Buffer.concat(chunks).toString());console.log(JSON.stringify({passed:completed.checks.length,counts:completed.counts,timings:completed.timings,error:completed.error}));response.end('ok');
  }else if(request.url==='/result'){response.setHeader('Content-Type','application/json');response.end(JSON.stringify(completed));}
  else if(request.url==='/'){response.setHeader('Content-Type','text/html');response.end('<!doctype html><meta charset="utf-8"><title>Frontend health diagnostic verification</title><div id="fixtures" hidden></div><script type="module" src="/probe.js"></script>');}
  else{response.statusCode=404;response.end();}
});
server.listen(0,'127.0.0.1',()=>console.log('http://127.0.0.1:'+server.address().port+'/'));
