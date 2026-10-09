import {readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {build} from 'esbuild';

let instance = 0;
export async function loadLiveSessionRecovery() {
  const source = await readFile(process.env.SELAH_LIVE_SESSION_RECOVERY_SOURCE || 'src/lib/views/Live.svelte', 'utf8');
  const start = source.indexOf('  let sessionEventVersion = 0;');
  const end = source.indexOf('  function listenLive', start);
  const recovery = source.slice(start, end < 0 ? undefined : end);
  if (start < 0 || !recovery.includes('const sessionRecovery = createCacheSyncQueue') || !recovery.includes('function resyncSession()')) {
    throw new Error('LIVE production recovery block missing');
  }
  const result = await build({stdin:{loader:'ts',resolveDir:process.cwd(),contents:`
    import {createCacheSyncQueue} from ${JSON.stringify(resolve('src/lib/cacheSyncQueue.ts'))};
    import {ResourceScope} from ${JSON.stringify(resolve('src/lib/resourceScope.ts'))};
    import {mergeLiveSnapshot,emptyLiveSurfaceSnapshot} from ${JSON.stringify(resolve('src/lib/views/live/liveTranscript.ts'))};
    const resources = new ResourceScope();
    let snapshot = emptyLiveSurfaceSnapshot();
    let read = async () => snapshot;
    const warnings = [];
    const recordWarning = (...args) => warnings.push(args);
    const isDemoActive = () => false;
    const liveGetSurface = () => read();
    ${recovery}
    export {resyncSession,warnings};
    export const configure = value => {read = value;};
    export const push = value => {sessionEventVersion++; mergeSessionRead(value);};
    export const dispose = () => resources.dispose();
    export const state = () => snapshot;
  `},bundle:true,write:false,platform:'node',format:'esm',define:{'console.warn':'recordWarning'}});
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#live-recovery-${++instance}`);
}
