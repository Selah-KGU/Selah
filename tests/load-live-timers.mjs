import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

export async function liveTimerCode(path = process.env.SELAH_LIVE_TIMERS_BEFORE || 'src/lib/views/Live.svelte') {
  const source = await readFile(path, 'utf8');
  const fn = name => {
    const value = source.match(new RegExp(`  (?:async )?function ${name}\\([^]*?\\n  }\\n`))?.[0];
    if (!value) throw new Error(`Production LIVE timer function missing: ${name}`);
    return value;
  };
  const declarations = ['saveNotifTimer', 'noticeTimer', 'scheduleFocusTimer', 'liveAutoGuardTimer'].map(name => {
    const value = source.match(new RegExp(`  let ${name}[^\\n]+;`))?.[0];
    if (!value) throw new Error(`Production LIVE timer owner missing: ${name}`);
    return value;
  }).join('\n');
  const binding = source.includes('function bindLiveAutoGuard()')
    ? fn('bindLiveAutoGuard') + '\n  bindLiveAutoGuard();'
    : source.match(/  \$effect\(\(\) => \{\n    if \(snapshot.active\) \{[^]*?\n  \}\);\n/)?.[0];
  if (!binding) throw new Error('Production LIVE auto guard binding missing');
  const destroy = source.match(/  onDestroy\(\(\) => \{([^]*?)\n  \}\);\n<\/script>/)?.[1];
  if (!destroy) throw new Error('Production LIVE destruction missing');
  return { declarations, binding, destroy,
    functions: ['rememberSaved', 'clearNoticeTimer', 'clearNotice', 'setNotice', 'setMessage',
      'setReadinessNotice', 'clearReadinessNotice', 'clearSttNotice', 'refreshFocusedCoursesFromClock',
      'clearLiveAutoLifecycle', 'markLiveListeningStarted', 'markLivePaused', 'stopLiveAutoGuardTimer',
      'checkLiveAutoLifecycle', 'stopScheduleFocusTimer', 'applyLiveSurfacePolicy'].map(fn).join('\n') };
}

export const timerBoundaryCode = `
  export const timers = [], calls = [], effects = [];
  let time = 100000;
  const NativeDate = globalThis.Date;
  export class OwnerDate extends NativeDate {
    constructor(...args) { if (args.length) super(...args); else super(time); }
    static now() { return time; }
  }
  export const setTime = value => { time = value; };
  function create(kind, callback, delay) {
    const timer = { id: timers.length + 1, kind, callback, delay, active: true, clears: 0 };
    timers.push(timer); return timer.id;
  }
  export const timerSetTimeout = (callback, delay) => create('timeout', callback, delay);
  export const timerSetInterval = (callback, delay) => create('interval', callback, delay);
  function clear(id) { const timer = timers.find(t => t.id === id); if (timer) { timer.clears++; timer.active = false; } }
  export const timerClearTimeout = clear, timerClearInterval = clear;
  export function fire(id) {
    const timer = timers.find(t => t.id === id);
    if (!timer) throw new Error('Unknown timer '+id);
    if (timer.kind === 'timeout') timer.active = false;
    timer.callback(); // Also deliver callbacks already queued before clearing.
  }
  export const trackEffect = callback => { effects.push(callback); callback(); };
  export const flushEffects = () => { for (const effect of effects) effect(); };
  export const liveTimers = () => timers.filter(t => t.active);
`;

export const timerStateCode = `
  let snapshot = emptyLiveSurfaceSnapshot(), sessionUpdateRevision = 0, lastSaved = null;
  let showSaveNotif = false, notice = null, now = new Date(0), scheduleData = null;
  let sttListening = false, sttPhase = 'idle', busy = false;
  let liveSurfaceWasVisible = false, liveMounted = true;
  let lastEffectiveSpeechAtMs = null, pausedSinceMs = null, autoLifecycleBusy = false;
  const NO_EFFECTIVE_SPEECH_AUTO_PAUSE_MS = 10 * 60 * 1000;
  const PAUSED_AUTO_FINISH_MS = 20 * 60 * 1000;
  const LIVE_AUTO_GUARD_INTERVAL_MS = 60 * 1000;
  let config = {};
  const controls = { get busy() { return isLiveBusy(snapshot,busy); },
    get booting() { return ['checking','starting','initializing'].includes(sttPhase); } };
  const applyScheduleSnapshot = (_data,date) => calls.push(['schedule',date.getTime()]);
  const bindLiveSttListeners = async () => { calls.push(['bind']); };
  const unbindLiveSttListeners = () => { calls.push(['unbind']); };
  const refreshLiveSttState = async () => { calls.push(['sttRead']); };
  const openSubtitleOverlay = async () => { calls.push(['overlay']); };
  async function pauseLiveInternal(auto) { if (!resources.active) return; calls.push(['pause',auto]); await config.pause?.(); }
  async function stopLiveInternal(auto) { if (!resources.active) return; calls.push(['stop',auto]); await config.stop?.(); }
`;

let instance = 0;
export async function loadLiveTimers() {
  const code = await liveTimerCode();
  // Owner tests execute the effect body explicitly; actual derived/effect
  // scheduling is separately checked by the isolated Svelte browser probe.
  const binding = code.binding.replace('const running = $derived(snapshot.active);', '').replace(/\brunning\b/g, 'snapshot.active');
  const result = await build({ stdin: { loader: 'ts', resolveDir: process.cwd(), contents: `
    import { ResourceScope } from ${JSON.stringify(resolve('src/lib/resourceScope.ts'))};
    import { emptyLiveSurfaceSnapshot, liveSavedPreview } from ${JSON.stringify(resolve('src/lib/views/live/liveTranscript.ts'))};
    import { isLiveBusy } from ${JSON.stringify(resolve('src/lib/views/live/liveFinish.ts'))};
    import { timers, calls, fire, setTime, liveTimers, flushEffects } from 'test-live-timer-boundaries';
    const resources = new ResourceScope();
    ${timerStateCode}
    ${code.declarations}
    ${code.functions}
    ${binding}
    export { timers, calls, fire, setTime, liveTimers, rememberSaved, setNotice, setMessage, clearNotice,
      setReadinessNotice, clearReadinessNotice, applyLiveSurfacePolicy, markLiveListeningStarted,
      markLivePaused, stopLiveAutoGuardTimer, checkLiveAutoLifecycle, refreshFocusedCoursesFromClock };
    export function destroy() { ${code.destroy} }
    export function configure(value) { config = value; }
    export function setState(value) {
      if ('snapshot' in value) snapshot = value.snapshot;
      if ('listening' in value) sttListening = value.listening;
      if ('phase' in value) sttPhase = value.phase;
      if ('busy' in value) busy = value.busy;
      if ('schedule' in value) scheduleData = value.schedule;
      flushEffects();
    }
    export const state = { get value() { return { snapshot, notice, showSaveNotif, lastSaved, now: now.getTime(),
      lastEffectiveSpeechAtMs, pausedSinceMs, autoLifecycleBusy, saveOwner: !!saveNotifTimer,
      noticeOwner: !!noticeTimer, autoOwner: !!liveAutoGuardTimer, scheduleOwner: !!scheduleFocusTimer }; } };
  ` }, bundle: true, write: false, platform: 'node', format: 'esm',
    define: { setTimeout: 'timerSetTimeout', clearTimeout: 'timerClearTimeout',
      setInterval: 'timerSetInterval', clearInterval: 'timerClearInterval', Date: 'OwnerDate',
      $effect: 'trackEffect', controlsBusy: 'controls.busy', sttBooting: 'controls.booting' },
    inject: ['test-live-timer-boundaries'],
    plugins: [{ name: 'live-timer-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^test-live-timer-boundaries$/ }, () => ({ path: 'timers', namespace: 'live-timer' }));
      plugin.onLoad({ filter: /.*/, namespace: 'live-timer' }, () => ({ contents: timerBoundaryCode, loader: 'js' }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#live-timers-${instance++}`);
}
