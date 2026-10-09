import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';

let instance = 0;
export async function loadReadState() {
  const source = await readFile(process.env.SELAH_READ_STATE_BEFORE || 'src/lib/stores.ts', 'utf8');
  const section = source.split('// ============ Read State (DB is source of truth) ============')[1]?.split('export const theme')[0];
  if (!section) throw new Error('Production notification read-state section missing');
  const result = await build({
    stdin: { loader: 'ts', resolveDir: process.cwd(), contents: `
      import { writable, get } from 'svelte/store';
      import { invoke, localStorage, requests, configure } from 'test-read-state-boundaries';
      ${section}
      export { requests, configure };
      export const snapshot = () => get(readIdsStore);
    ` },
    bundle: true, write: false, platform: 'node', format: 'esm',
    plugins: [{ name: 'read-state-boundaries', setup(plugin) {
      plugin.onResolve({ filter: /^test-read-state-boundaries$/ }, () => ({ path: 'boundaries', namespace: 'read-state' }));
      plugin.onLoad({ filter: /.*/, namespace: 'read-state' }, () => ({ loader: 'js', contents: `
        export const requests = [];
        let demo = false;
        const storage = { getItem: key => key === 'selah-demo-mode' && demo ? '1' : null };
        export let localStorage = storage;
        export function configure(options) {
          if ('demo' in options) demo = options.demo;
          if ('storage' in options) localStorage = options.storage ? storage : undefined;
        }
        export function invoke(command, args) {
          return new Promise((resolve, reject) => {
            // Match invoke's argument snapshot; caller mutation cannot change
            // a request which has already crossed the transport boundary.
            requests.push({ command, args: args ? structuredClone(args) : undefined, resolve, reject });
          });
        }
      ` }));
    } }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#${instance++}`);
}
export const tick = () => new Promise(resolve => setImmediate(resolve));
export const emptyIds = () => ({ kgc: [], luna: [], kwic: [] });
