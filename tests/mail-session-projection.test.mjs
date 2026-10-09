import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
let instance = 0;
async function fixture() {
  const source = await readFile('src/lib/api.ts', 'utf8');
  const projection = source.slice(source.indexOf('let mailRequestRevision ='), source.indexOf('if (!isAuxiliarySurface()'));
  const built = await build({stdin:{loader:'ts',contents:`
    export const calls = [], state = {};
    const invalidateCache = key => calls.push(['invalidate',key]);
    const replaceCacheEntry = (key,data) => calls.push(['replace',key,data]);
    const requestedMailMessageId = {set: value => calls.push(['requested',value])};
    const mailAuthState = {set:value=>state.mail=value};
    ${projection}
    export {applyMailStatus};
  `},bundle:true,write:false,platform:'node',format:'esm'});
  return import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].text).toString('base64')}#mail-${++instance}`);
}
const status = (generation, connection) => ({generation, connection_id:connection,authenticated:!!connection,email:connection ?? '',display_name:''});
test('new mailbox clears old cached content and stale status cannot restore it', async () => {
  const h = await fixture();
  h.applyMailStatus(status(1,'alice'));
  h.calls.length = 0;
  h.applyMailStatus(status(3,'bob'));
  assert.deepEqual(h.calls.slice(0,2), [['invalidate','mail_inbox'],['replace','mail_inbox',[]]]);
  h.calls.length = 0;
  h.applyMailStatus(status(1,'alice'));
  assert.equal(h.state.mail.connectionId,'bob');
  assert.deepEqual(h.calls,[]);
  h.applyMailStatus(status(4,null));
  assert.equal(h.state.mail.authenticated,false);
  h.applyMailStatus(status(3,'bob'));
  assert.equal(h.state.mail.authenticated,false);
});
test('refresh of the same connection keeps its cache', async () => {
  const h = await fixture();
  h.applyMailStatus(status(1,'alice'));
  h.calls.length=0;
  h.applyMailStatus(status(1,'alice'));
  assert.deepEqual(h.calls,[]);
});
