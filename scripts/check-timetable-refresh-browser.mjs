// Render the production timetable against controlled native IPC, including the
// legacy warning from the reported screenshot and conflicting KGC cache rows.
import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile, mkdtemp, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { tmpdir } from "node:os";
import { createServer } from "node:http";
import { spawn } from "node:child_process";

const state = `
export const calls = [], listeners = new Map();
export const legacyWarning = 'KGC に接続できません。Luna と保存済みの時間割を表示しています。';
export function schedule(name, current='2026年10月19日～2026年10月24日', next='2026年10月26日～2026年10月31日') {
  return { raw: {
    kgc_entries_current: [{ name:'Old KGC', day:1, period:1, kgc_code:'old', room:'Old room', is_cancelled:true }],
    kgc_entries_next: [], luna_courses:[{ name, day:1, period:1, luna_id:'2026000000000301', teacher:'Luna teacher' }],
    session_plans:[], luna_counts:[], luna_activities:[], kgc_course_details:[], luna_communities:[],
    current_week_label:current, next_week_label:next,
  }, ai_result:{ current_week:[
      { day:1, period:1, course_name:'Old AI course', room:'Wrong AI room', teacher:'Wrong AI teacher', is_cancelled:true },
      { day:2, period:2, course_name:'Ghost AI course', is_cancelled:true },
    ], next_week:[], current_week_label:current, next_week_label:next }, ai_stale:false, snapshot_updated_at:1,
    luna_communities:[], luna_year_options:[], luna_term_options:[], luna_year:'2026', luna_term:'03', kgc_warning:legacyWarning };
}
export let snapshot = schedule('Initial Luna', '2026/10/12～2026/10/17', '2026/10/19～2026/10/24'), failure = false;
export const failSync = value => { failure = value; };
export const setSnapshot = value => { snapshot = value; };
export function pushSchedule(value) { snapshot=value; listeners.get('schedule_data')?.(value); }
export async function readSnapshot() { calls.push('get_schedule_snapshot'); return snapshot; }
export async function sync() {
  calls.push('sync_schedule_data');
  if(failure) throw new Error('Luna unavailable');
  snapshot=schedule('Updated Luna'); return snapshot;
}
`;

const api = `
import { calls, readSnapshot, sync } from 'timetable-test-state';
export const getScheduleSnapshot = readSnapshot, syncScheduleData = sync;
export async function getDataCache(key) { calls.push('cache:' + key); return null; }
export async function saveDataCache() {}
export async function getAiConfig() { return { provider:'local' }; }
export const isDemoActive = () => false;
export async function gcalCheckSession() { return { authenticated:false, calendar_exists:false, synced_events:0 }; }
export const isAiReady = () => false;
export async function aiGenerateSchedule() {}
export async function openSettingsWindow() {}
export async function gcalSyncTimetable() {}
export async function gcalOpenLogin() {}
export async function openSyllabusDetail() {}
export function resetAiReady() {}
export async function saveImageFile() {}
export async function copyImageToClipboard() {}
export async function shareImageNative() {}
export async function fetchSyllabusFavorites() { throw new Error('Unexpected KGC request'); }
`;

const stores = `
import { writable } from 'svelte/store';
import { listeners } from 'timetable-test-state';
export const lunaAuthState = writable({ authenticated:false });
export const gcalAuthState = writable({ authenticated:false });
export const sessionExpired = writable(false), aiReady = writable(false);
export const authState = writable({}), activeTab = writable('timetable');
export function onCacheUpdate(key, callback) { listeners.set(key, callback); return () => listeners.delete(key); }
export async function cachedBackendFetch() { throw new Error('Unexpected KGC request'); }
`;

const source = `
import { mount, unmount, tick } from 'svelte';
import Timetable from ${JSON.stringify(resolve("src/lib/views/Timetable.svelte"))};
import { calls, schedule, failSync, pushSchedule, setSnapshot, legacyWarning } from 'timetable-test-state';
const RealDate = Date;
let clock = new RealDate(2026,9,16,23,59).getTime();
globalThis.Date = class extends RealDate {
  constructor(...args) { if(args.length) super(...args); else super(clock); }
  static now() { return clock; }
};
const realSetInterval = window.setInterval.bind(window), realClearInterval = window.clearInterval.bind(window);
const dayChecks = new Map();
window.setInterval = (callback, ms, ...args) => {
  const id = realSetInterval(callback, ms, ...args);
  if(ms === 60_000) dayChecks.set(id, callback);
  return id;
};
window.clearInterval = id => { dayChecks.delete(id); realClearInterval(id); };
const target = document.getElementById('probe'), checks = [];
let app;
function check(value, message) { if (!value) throw new Error(message); checks.push(message); }
async function until(predicate) {
  for(let i=0;i<200;i++) { await tick(); if(predicate()) return; await new Promise(resolve => setTimeout(resolve,5)); }
  throw new Error('DOM condition timed out');
}
const updateButton = () => [...target.querySelectorAll('button')].find(button => button.textContent.trim() === '更新');
const shownWeek = () => target.querySelector('.header-date-content')?.textContent.replace(/\\s/g,'');
async function show() {
  app = mount(Timetable, { target });
  await until(() => updateButton() && target.textContent.includes('Luna'));
}
try {
  await show();
  check(shownWeek() === '26年10月12-17日', 'Friday displays the outgoing teaching week');
  check(target.textContent.includes('Initial Luna'), 'the grid displays Luna despite conflicting KGC cache data');
  check(!target.textContent.includes('Old KGC'), 'KGC cannot replace the primary course');
  check(!target.querySelector('.entry-cancelled') && !target.textContent.includes('Wrong AI room'), 'old AI cannot attach cancellation or room to a different Luna course');
  check(!target.textContent.includes('Ghost AI course') && target.querySelectorAll('.course-cell').length === 1, 'AI cannot create a course absent from the primary timetable');
  check(target.textContent.includes('Luna teacher'), 'Luna teacher remains visible without KGC');
  target.querySelector('.course-cell').click(); await tick();
  check(calls.includes('university_open_detail_window'), 'a Luna course opens its Luna detail even before the login status projection catches up');
  check(!target.textContent.includes(legacyWarning) && !target.querySelector('.error-banner'), 'the screenshot warning is absent even in an old cached payload');
  clock = new RealDate(2026,9,17,0,1).getTime();
  updateButton().click();
  await until(() => target.textContent.includes('Updated Luna') && !updateButton().disabled);
  check(target.textContent.includes('時間割を更新しました'), 'successful Luna refresh shows a success acknowledgment');
  check(shownWeek() === '26年10月19-24日', 'Saturday refresh advances the displayed week without KGC');
  target.querySelector('.header-date').click(); await tick();
  check(shownWeek() === '26年10月26-31日', 'the next-week control follows the advanced current week');
  target.querySelector('.header-date').click(); await tick();
  check(calls.filter(command => command === 'sync_schedule_data').length === 1, 'one click invokes the primary refresh once');
  check(!calls.some(command => !command.startsWith('cache:') && /kgc|syllabus|fetch_exams|login|check_session/i.test(command)), 'opening and refreshing the page issue no KGC or login commands');
  pushSchedule(schedule('Background Luna')); await tick();
  check(target.textContent.includes('Background Luna') && !target.textContent.includes('Old KGC'), 'secondary cache pushes preserve Luna priority');
  failSync(true); updateButton().click();
  await until(() => target.querySelector('.error-banner')?.textContent.includes('Luna unavailable'));
  check(target.textContent.includes('Background Luna'), 'a real Luna failure preserves the last timetable and shows its error');
  const secondary=schedule('Background Luna'); secondary.snapshot_updated_at=999; pushSchedule(secondary); await tick();
  check(target.querySelector('.error-banner')?.textContent.includes('Luna unavailable'), 'a secondary cache push cannot clear a primary Luna failure');
  failSync(false); updateButton().click();
  await until(() => target.textContent.includes('Updated Luna') && !updateButton().disabled);
  check(!target.querySelector('.error-banner'), 'retry clears the actual error after a successful refresh');
  await unmount(app); app=null; await show();
  check(!target.textContent.includes(legacyWarning), 'remounting the page cannot restore the persisted KGC warning');
  const offline = schedule('Offline Luna', '2026年10月26日～2026年10月31日', '2026年11月02日～2026年11月07日');
  failSync(true); setSnapshot(offline);
  clock = new RealDate(2026,9,24,0,1).getTime();
  const syncCalls = calls.filter(command => command === 'sync_schedule_data').length;
  check(dayChecks.size === 1, 'remounting leaves only one calendar check timer');
  for(const callback of dayChecks.values()) callback();
  await until(() => target.textContent.includes('Offline Luna'));
  check(shownWeek() === '26年10月26-31日', 'a page left open advances on Saturday from its local snapshot while offline');
  check(calls.filter(command => command === 'sync_schedule_data').length === syncCalls, 'crossing the day reads the local snapshot without network refresh or KGC login');
  updateButton().click();
  await until(() => target.querySelector('.error-banner')?.textContent.includes('Luna unavailable'));
  check(shownWeek() === '26年10月26-31日' && target.textContent.includes('Offline Luna'), 'a failed refresh preserves the advanced offline week and courses');
  setSnapshot(schedule('Resumed Luna', offline.raw.current_week_label, offline.raw.next_week_label));
  clock = new RealDate(2026,9,25,9,0).getTime();
  window.dispatchEvent(new Event('focus'));
  await until(() => target.textContent.includes('Resumed Luna'));
  check(shownWeek() === '26年10月26-31日', 'resuming after another day reloads the local dates without advancing a second time');
  await unmount(app); app=null;
  check(dayChecks.size === 0, 'unmounting releases the calendar check timer');
  await fetch('/result', { method:'POST', body:JSON.stringify({ passed:checks.length, checks }) });
} catch(error) {
  await fetch('/result', { method:'POST', body:JSON.stringify({ passed:checks.length, checks, error:String(error) }) });
} finally { if(app) await unmount(app); }
`;

const bundle = await build({
  stdin: { contents: source, loader:"js", resolveDir:process.cwd() },
  bundle:true, write:false, platform:"browser", format:"esm", conditions:["browser"], loader:{ ".png":"dataurl" },
  plugins:[{ name:"timetable-ui-fixture", setup(builder) {
    builder.onResolve({ filter:/^timetable-test-state$/ }, () => ({ path:"state", namespace:"timetable-fixture" }));
    builder.onResolve({ filter:/^@tauri-apps\/api\/core$/ }, () => ({ path:"native", namespace:"timetable-fixture" }));
    builder.onResolve({ filter:/^\.\.\/(api|stores)$/ }, args => args.importer.endsWith("Timetable.svelte")
      ? { path:args.path.endsWith("api") ? "api" : "stores", namespace:"timetable-fixture" } : undefined);
    builder.onLoad({ filter:/.*/, namespace:"timetable-fixture" }, ({path}) => ({
      contents: path === "state" ? state : path === "api" ? api : path === "stores" ? stores
        : "import { calls } from 'timetable-test-state'; export async function invoke(command) { calls.push(command); if(command === 'university_open_detail_window') return; throw new Error('Unexpected native command: ' + command); }",
      loader:"js", resolveDir:process.cwd(),
    }));
    builder.onLoad({ filter:/\.svelte$/ }, async ({path}) => ({
      contents:compile(await readFile(path.endsWith("Timetable.svelte") && process.env.SELAH_TIMETABLE_SOURCE || path, "utf8"), {
        filename:path, generate:"client", css:"injected",
      }).js.code,
      loader:"js", resolveDir:dirname(path),
    }));
  }}],
});

let finish;
const result = new Promise(resolve => { finish=resolve; });
const server=createServer(async(request,response) => {
  if(request.url === "/probe.js") { response.setHeader("Content-Type","text/javascript"); response.end(bundle.outputFiles[0].text); }
  else if(request.url === "/result" && request.method === "POST") {
    const chunks=[]; for await(const chunk of request) chunks.push(chunk);
    finish(JSON.parse(Buffer.concat(chunks).toString())); response.end("ok");
  } else {
    response.setHeader("Content-Type","text/html");
    response.end('<!doctype html><meta charset="utf-8"><title>Timetable primary-source regression</title><div id="probe"></div><script type="module" src="/probe.js"></script>');
  }
});
await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
const profile=await mkdtemp(resolve(tmpdir(),"selah-timetable-ui-"));
const chrome=spawn(process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", [
  "--headless", "--disable-gpu", "--disable-extensions", "--disable-background-networking", "--no-first-run",
  "--user-data-dir="+profile, "http://127.0.0.1:"+server.address().port,
], { stdio:"ignore" });
let timer;
try {
  const outcome=await Promise.race([result,new Promise((_,reject) => {
    timer=setTimeout(() => reject(new Error("Browser checks timed out")),25000);
    chrome.once("error",reject);
  })]);
  console.log(JSON.stringify(outcome,null,2));
  if(outcome.error) process.exitCode=1;
} finally {
  clearTimeout(timer);
  if(chrome.exitCode === null && chrome.signalCode === null) {
    chrome.kill(); await new Promise(resolve => chrome.once("exit",resolve));
  }
  server.closeAllConnections(); server.close();
  await rm(profile,{recursive:true,force:true});
}
