import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';

let instance = 0;
async function fixture(options) {
  const source = await readFile('src/lib/views/settings/SettingsCalendar.svelte', 'utf8');
  const save = source.match(/export async function save\(\)[\s\S]*?\n  }/)[0];
  const result = await build({ stdin: { contents: `
    const options = ${JSON.stringify(options)};
    export const calls = [];
    let saveBusy = false, springStart = '2026-04-03', fallStart = '2026-09-21', calSyncInterval = '12';
    let gcalAutoSync = options.enabled ? 'true' : 'false';
    let gcalClientId = 'custom', gcalClientSecret = 'explicit';
    let gcalConfigLoaded = !!options.loaded, gcalConfigEdited = !!options.edited;
    let bindingAccount = { username: 'alice', generation: 7, connectionId: 'google-a', calendarId: 'calendar-a' };
    const isDemoActive = () => false;
    const localStorage = { setItem() {} };
    async function gcalSaveConfig(...args) { calls.push(['credentials', ...args]); if (options.switchGoogleDuringSave) bindingAccount = { username:'alice', generation:7, connectionId:'google-b', calendarId:'calendar-b' }; if (options.switchDuringSave) bindingAccount = { username:'bob', generation:8, connectionId:'google-b', calendarId:'calendar-b' }; }
    async function invoke(command, args) { calls.push([command,args]); }
    ${save}
  `, loader: 'ts' }, bundle: true, write: false, format: 'esm', platform: 'node' });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}#calendar-save-${++instance}`);
}

test('saving semester preferences after credential read failure cannot overwrite the secret', async () => {
  const h = await fixture({});
  await h.save();
  assert.equal(h.calls.some(([name]) => name === 'credentials'), false);
  assert.equal(h.calls[0][0], 'save_calendar_config');
});

test('automatic sync consent carries the account the settings page actually displayed', async () => {
  const h = await fixture({ loaded: true, enabled: true, switchDuringSave: true });
  await h.save();
  assert.deepEqual(h.calls.map(([name]) => name), ['credentials', 'save_calendar_config']);
  assert.equal(h.calls[1][1].accountUsername, 'alice');
  assert.equal(h.calls[1][1].accountGeneration, 7);
  assert.equal(h.calls[1][1].connectionId, 'google-a');
  assert.equal(h.calls[1][1].calendarId, 'calendar-a');
  assert.equal(h.calls[1][1].config.gcal_auto_sync, true);
});

test('an explicit credential edit can repair an unreadable configuration', async () => {
  const h = await fixture({ edited: true });
  await h.save();
  assert.deepEqual(h.calls[0], ['credentials', 'custom', 'explicit']);
});

test('same university account cannot silently confirm a Google connection that changed during save', async () => {
  const h = await fixture({ loaded: true, enabled: true, switchGoogleDuringSave: true });
  await h.save();
  assert.equal(h.calls[1][1].accountUsername, 'alice');
  assert.equal(h.calls[1][1].connectionId, 'google-a');
  assert.equal(h.calls[1][1].calendarId, 'calendar-a');
});
