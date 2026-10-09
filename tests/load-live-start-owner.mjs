import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

let instance = 0;
export async function loadLiveStartOwner() {
  const source = await readFile(process.env.SELAH_LIVE_START_OWNER_BEFORE || 'src/lib/views/Live.svelte','utf8');
  const fn = name => {
    const entry = source.match(new RegExp(`  (?:async )?function ${name}\\([^]*?\\n  }\\n`))?.[0];
    if (!entry) throw new Error(`LIVE production function missing: ${name}`);
    return entry;
  };
  const owner = source.match(/  let (?:pendingStartSessionId|cancelSessionOnStartFailure)[^\n]+;/)?.[0];
  if (!owner) throw new Error('LIVE start owner missing');
  const result = await build({stdin:{loader:'ts',resolveDir:process.cwd(),contents:`
    import {ResourceScope,ResourceSlot,acquireResourceGroup} from ${JSON.stringify(resolve('src/lib/resourceScope.ts'))};
    import {emptyLiveSurfaceSnapshot,mergeLiveSnapshot,isCurrentLiveSttEvent} from ${JSON.stringify(resolve('src/lib/views/live/liveTranscript.ts'))};
    import {isLiveBusy} from ${JSON.stringify(resolve('src/lib/views/live/liveFinish.ts'))};
    const calls = [], listeners = [], warnings = [];
    let config = {};
    const resources = new ResourceScope(error => warnings.push(String(error)));
    const sttSubscription = new ResourceSlot(resources);
    let snapshot = emptyLiveSurfaceSnapshot(), busy = false, sttListening = false, sttPhase = 'idle';
    let sessionEventVersion = 0, sessionUpdateRevision = 0, sttBindToken = 0, sttListenersBound = false;
    let overallSummary = 'saved overall', partialText = 'existing partial', lastSaved = {summary_markdown:'saved preview'}, autoFollow = false;
    let lastEffectiveSpeechAtMs = null, pausedSinceMs = null, autoLifecycleBusy = false, lastPartialSeq = 0;
    let liveReady = true, readinessMessage = '', notice = null;
    const controlState = {get busy() {return isLiveBusy(snapshot,busy);}};
    ${owner}
    const isDemoActive = () => !!config.demo;
    const clearNotice = () => {notice = null; calls.push(['clearNotice']);};
    const setSttNotice = text => calls.push(['sttNotice',text]);
    const clearSttNotice = () => calls.push(['clearSttNotice']);
    const setMessage = (kind,text) => {notice = {kind,text}; calls.push(['message',kind,text]);};
    const sttStateRead = {invalidate:() => calls.push(['invalidateStt'])};
    async function refreshReadiness() {
      calls.push(['ready']);
      const result = await config.ready?.();
      if (result) {liveReady = result.ready; readinessMessage = result.message || '';}
      return result?.applied ?? true;
    }
    async function liveStartSurface(course) {calls.push(['create',course]);return config.create(course);}
    async function invoke(name,args) {calls.push(['invoke',name,args]);return config.invoke?.(name,args);}
    async function liveCancelSession(id) {calls.push(['cancel',id]);return config.cancel?.(id);}
    async function resyncSession() {if (!resources.active) return;calls.push(['resync']);return config.resync?.();}
    async function listen(name,receive) {
      const listener = {name,receive,releases:0};listeners.push(listener);
      return () => {listener.releases++;};
    }
    ${['mergeSessionRead','ensureReadyToStart','markLiveListeningStarted','markEffectiveSpeech','markLivePaused',
      'clearLiveAutoLifecycle','startSession','applyLiveSttPhase','bindLiveSttListeners'].map(fn).join('\n')}
    export {calls,listeners,warnings,startSession,bindLiveSttListeners};
    export const configure = value => {config = value;};
    export function pushSession(value) {sessionEventVersion++;mergeSessionRead(value);}
    export function dispose() {resources.dispose();}
    export const state = {get value() {return {snapshot,busy,sttListening,sttPhase,overallSummary,partialText,lastSaved,
      autoFollow,lastEffectiveSpeechAtMs,pausedSinceMs,notice};}};
  `}, bundle:true,write:false,platform:'node',format:'esm',
    define:{controlsBusy:'controlState.busy','console.warn':'ownerWarning'},inject:['test-owner-console'],
    plugins:[{name:'owner-console',setup(plugin) {
      plugin.onResolve({filter:/^test-owner-console$/},() => ({path:'console',namespace:'owner-test'}));
      plugin.onLoad({filter:/.*/,namespace:'owner-test'},() => ({contents:'export function ownerWarning() {}',loader:'js'}));
    }}],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#live-owner-${++instance}`);
}
