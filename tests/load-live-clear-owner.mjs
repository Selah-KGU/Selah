import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

export async function liveClearOwnerCode(path = process.env.SELAH_LIVE_CLEAR_OWNER_BEFORE || 'src/lib/views/Live.svelte') {
  const source = await readFile(path, 'utf8');
  const required = (value, name) => { if (!value) throw new Error(`Production LIVE clear source missing: ${name}`); return value; };
  const fn = name => required(source.match(new RegExp(`  (?:async )?function ${name}\\([^]*?\\n  }\\n`))?.[0], name);
  const derived = name => required(source.match(new RegExp(`  const ${name} = \\$derived.by\\(\\(\\) => \\{[^]*?\\n  \\}\\);`))?.[0], name);
  return {
    clear: fn('executeClearCourseData'),
    selected: derived('selectedCourse'), identity: derived('previewIdentity'),
    preview: required(source.match(/  const previewRead = new LatestViewRead\([^]*?\n  \}\);/)?.[0], 'previewRead'),
    binding: source.includes('function bindLiveCoursePreview()')
      ? fn('bindLiveCoursePreview') + '\n  bindLiveCoursePreview();'
      : required(source.match(/  \$effect\(\(\) => \{\n    void previewIdentity;[^]*?\n  \}\);/)?.[0], 'preview binding'),
  };
}

export const clearOwnerStateCode = `
  const calls = [];
  let config = {};
  let snapshot = emptyLiveSurfaceSnapshot(), busy = false, showSaveNotif = false, lastSaved = null;
  let now = new Date(2026, 9, 8, 12), sessionEventVersion = 0;
  let courseOptions = [
    {name:'Course A',day:4,period:1,room:'A101'},
    {name:'Course B',day:4,period:2,room:'B202'}
  ];
  let selectedKey = courseKey(courseOptions[0]);
  let overallSummary = 'existing summary', summaryDetailOpen = true, summaryViewIndex = 2, notice = null;
  const controls = { get busy() { return isLiveBusy(snapshot,busy); } };
  const clearNotice = () => {notice = null;};
  const setMessage = (kind,text) => { if (resources.active) {notice = {kind,text};calls.push(['message',kind,text]);} };
  async function liveClearDayCache(course) {calls.push(['clear',course]);return config.clear?.(course);}
  async function livePeekDaySurface(course) {calls.push(['peek',course]);return config.peek?.(course) ?? emptyLiveSurfaceSnapshot();}
`;

export const clearOwnerImports = `
  import {ResourceScope} from ${JSON.stringify(resolve('src/lib/resourceScope.ts'))};
  import {LatestViewRead} from ${JSON.stringify(resolve('src/lib/latestViewRead.ts'))};
  import {emptyLiveSurfaceSnapshot} from ${JSON.stringify(resolve('src/lib/views/live/liveTranscript.ts'))};
  import {isLiveBusy} from ${JSON.stringify(resolve('src/lib/views/live/liveFinish.ts'))};
  import {courseKey,toLiveCourse} from ${JSON.stringify(resolve('src/lib/views/live/liveCourseSelection.ts'))};
`;

let instance = 0;
export async function loadLiveClearOwner() {
  const code = await liveClearOwnerCode();
  const readDerived = (source, name) => source.replace(`const ${name} = $derived.by(() => {`, `function read${name}() {`).replace(/\}\);$/, '}');
  const result = await build({stdin:{loader:'ts',resolveDir:process.cwd(),contents:`
    ${clearOwnerImports}
    const resources = new ResourceScope();
    const untrack = callback => callback();
    ${clearOwnerStateCode}
    ${readDerived(code.selected,'selectedCourse')}
    ${readDerived(code.identity,'previewIdentity')}
    const selection = {get course() {return readselectedCourse();},get identity() {return readpreviewIdentity();}};
    ${code.preview}
    ${code.clear}
    export {calls,executeClearCourseData};
    export function configure(value) {config=value;}
    export function select(index) {selectedKey = index == null ? '__free_note__' : courseKey(courseOptions[index]);}
    export function replaceCourses(value) {courseOptions=value;}
    export function setSnapshot(value, event=false) {snapshot=value;if(event) sessionEventVersion++;}
    export function setTime(value) {now=value;}
    export function setSaved(value, saved=value ? {summary_markdown:'new saved preview'} : lastSaved) {showSaveNotif=value;lastSaved=saved;}
    export function setBusy(value) {busy=value;}
    export function dispose() {resources.dispose();}
    export function peek() {return previewRead.refresh(toLiveCourse(selection.course));}
    export const state = {get value() {return {snapshot,busy,showSaveNotif,overallSummary,summaryDetailOpen,summaryViewIndex,notice};}};
  `},bundle:true,write:false,platform:'node',format:'esm',
    define:{selectedCourse:'selection.course',previewIdentity:'selection.identity',controlsBusy:'controls.busy'}});
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#live-clear-${instance++}`);
}
