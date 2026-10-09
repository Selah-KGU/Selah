import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';

let instance = 0;
// Run the exact production restoration function with only its IO/store
// dependencies replaced. Already issued native work is not simulated as canceled.
export async function loadSessionRestore() {
  const source = await readFile(process.env.SELAH_RESTORE_STARTUP_BEFORE || 'src/lib/api.ts', 'utf8');
  const restore = source.match(/export async function restoreAllSessions\([\s\S]*?\n}\n/)?.[0];
  if (!restore) throw new Error('Session restoration function missing');
  const result = await build({ stdin: { contents: `
    type SessionStatus = { valid: boolean; username: string; display_name?: string; student_id?: string; faculty?: string; department?: string };
    export const calls = [], state = { auth: null, expired: null, mail: null }, storage = new Map();
    let config = {};
    export const configure = value => { config = value; };
    const localStorage = { setItem: (key, value) => storage.set(key, value) };
    const EVER_AUTH_KEY = 'selah-ever-auth';
    const debugLog = () => {}, _isDemo = () => !!config.demo;
    const authState = { set: value => { calls.push(['auth', value]); state.auth = value; } };
    const sessionExpired = { set: value => { calls.push(['expired', value]); state.expired = value; } };
    const mailAuthState = { set: value => { calls.push(['mail', value]); state.mail = value; } };
    const serviceRegistry = Object.fromEntries(['luna','kwic'].map(key => [key, {
      onRecovered: () => calls.push(['recovered', key]), onReset: () => calls.push(['reset', key]),
    }]));
    function setAuthFromSession(value) { calls.push(['setAuthFromSession', value]); state.auth = value; }
    async function getKgcSessionSnapshot() { calls.push(['snapshot']); return config.snapshot ? config.snapshot() : { valid: true, username: 'user', display_name: 'ユーザー' }; }
    async function getStoredSessionStates() { calls.push(['stored']); return config.stored ? config.stored() : { kgc: true, luna: true, kwic: true }; }
    async function lunaCheckSession() { calls.push(['validate','luna']); return config.validate ? config.validate('luna') : true; }
    async function kwicCheckSession() { calls.push(['validate','kwic']); return config.validate ? config.validate('kwic') : true; }
    async function syncSession(key) { calls.push(['sync',key]); return config.sync ? config.sync(key) : true; }
    async function mailCheckSession() { calls.push(['mailCheck']); return config.mail ? config.mail() : { authenticated: true, email: 'full@example.test', display_name: '完全な名前' }; }
    ${restore}
  `, loader: 'ts' }, bundle: true, write: false, platform: 'node', format: 'esm',
    define: { 'console.warn': 'ignoredWarning' }, banner: { js: 'function ignoredWarning() {}' } });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#session-restore-${++instance}`);
}
