import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { resolve } from 'node:path';
let instance = 0;
export async function loadSessionRestore() {
  const source = await readFile('src/lib/api.ts', 'utf8');
  const functions = ['restoreAllSessions','applyUniversitySnapshot','applyRecoveryReport'].map(name => {
    const match = source.match(new RegExp(`(?:export )?(?:async )?function ${name}\\([^]*?\\n}\\n`));
    if (!match) throw new Error(`Missing production function ${name}`);
    return match[0];
  }).join('\n');
  const result = await build({ stdin: { contents: `
    import { universitySessionLifetime } from ${JSON.stringify(resolve('src/lib/sessionLifetime.ts'))};
    import { projectUniversitySession } from ${JSON.stringify(resolve('src/lib/universitySession.ts'))};
    export { universitySessionLifetime };
    export const calls = [], state = { auth: null, expired: null, mail: null, luna: null, kwic: null };
    let config = {};
    export const configure = value => { config = value; };
    const _isDemo = () => !!config.demo;
    const store = key => ({ set(value) { calls.push([key,value]); state[key] = value; } });
    const universityLoginPersistencePending = store('persistencePending');
    const sessionExpired = store('expired'), lunaAuthState = store('luna'), kwicAuthState = store('kwic'), mailAuthState = store('mail');
    function setAuthFromSession(value) { calls.push(['auth',value]); state.auth = value; }
    async function invoke(command) { calls.push(['invoke',command]); return config.restore(); }
    async function mailCheckSession() { calls.push(['mailCheck']); return config.mail ? config.mail() : { authenticated: true, email: 'full@example.test', display_name: '完全な名前' }; }
    ${functions}
  `, loader:'ts', resolveDir:process.cwd() }, bundle:true, write:false, platform:'node', format:'esm',
    define:{'console.warn':'ignoredWarning'}, banner:{js:'function ignoredWarning() {}'} });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#restore-${++instance}`);
}
export const identity = { username:'full', display_name:'完全な名前', student_id:'123', faculty:'理工', department:'情報' };
export function report({ who = identity, health = 'valid', generation = 0, revision = 1, present = true, proof = true, signedOut = false } = {}) {
  return { identity: who, results: [], snapshot: { generation, revision, signed_out:signedOut,
    services:['kgc','luna','kwic'].map(service => ({service,state:health,credentials_present:present,last_verified_at:proof ? 100 : null,last_attempt_at:null})) } };
}
