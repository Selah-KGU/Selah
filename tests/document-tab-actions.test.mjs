import test from 'node:test';
import assert from 'node:assert/strict';
import { loadDocumentTabs } from './load-document-tabs.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const row = (id, type = 'files') => ({ id, target: `surface-${id}`, url: `https://${id}.example/`, type, active: true, controls: [] });
const fallback = command => command === 'document_tabs_list' ? [] : command === 'get_app_theme' ? 'light' : false;

// Both failed against the earlier component: a delayed files.search was routed
// without its origin ID, and a failed URL read fell back to the newly active URL.
test('delayed file searches cannot move to another files tab, including A to B to A', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback);
  h.panel.select(row('a')); h.panel.searchFor('original');
  const old = h.clock.created.at(-1);
  h.panel.select(row('b')); h.panel.select(row('a'));
  old.callback(); await flush();
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_send_control').length, 0);
  h.panel.dispose();
});

test('external browser fallback uses the clicked page when selection changes during the URL read', async () => {
  const h = await loadDocumentTabs(); const pending = deferred();
  h.configure(command => command === 'browser_get_url' ? pending.promise : fallback(command));
  h.panel.select(row('a', 'browser')); const action = h.panel.external();
  h.panel.select(row('b', 'browser')); pending.reject(new Error('URL read failed')); await action;
  assert.deepEqual(h.calls.find(c => c.command === 'open_in_system_browser').args, { url: 'https://a.example/' });
  h.panel.dispose();
});

test('search bursts retain the latest query and origin, while an equivalent tab update does not cancel them', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); h.panel.select(row('a'));
  h.panel.searchFor('first'); const obsolete = h.clock.created.at(-1);
  h.panel.searchFor('日本語 query'); const latest = h.clock.created.at(-1);
  obsolete.callback(); await flush();
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_send_control').length, 0);
  h.panel.select({ ...row('a'), title: 'updated title' });
  assert.equal(h.clock.active.size, 1);
  latest.callback(); latest.callback(); await flush();
  const controls = h.calls.filter(c => c.command === 'document_tabs_send_control');
  assert.deepEqual(controls, [{ command: 'document_tabs_send_control', args: { owner: 'document-tabs', tabId: 'a', action: 'files.search', payload: '日本語 query' } }]);
  assert.equal(h.clock.active.size, 0);
  h.panel.dispose();
});

test('leaving a files tab or closing the component cancels delayed searches', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback); h.panel.select(row('a'));
  h.panel.searchFor('old'); const old = h.clock.created.at(-1);
  h.panel.select(row('reader', 'reader')); old.callback(); await flush();
  h.panel.select(row('a')); h.panel.searchFor('closing'); const closing = h.clock.created.at(-1);
  h.panel.dispose(); closing.callback(); await flush();
  assert.equal(h.calls.filter(c => c.command === 'document_tabs_send_control').length, 0);
  assert.equal(h.clock.active.size, 0);
});

test('toolbar commands carry the selected tab ID and retain menu coordinates and payloads', async () => {
  const h = await loadDocumentTabs(); const sent = deferred();
  h.configure(command => command === 'document_tabs_send_control' ? sent.promise : fallback(command));
  h.panel.select(row('a'));
  h.panel.control({ action: 'files.toggleSortMenu', label: 'sort' }, { currentTarget: { getBoundingClientRect: () => ({ left: 67 }) } });
  h.panel.select(row('b'));
  assert.deepEqual(h.calls[0], { command: 'document_tabs_send_control', args: { owner: 'document-tabs', tabId: 'a', action: 'files.toggleSortMenu', payload: { x: 67 } } });
  sent.resolve(); await flush();
  h.panel.control({ action: 'detail.refresh', label: 'refresh', payload: { page: 3 } });
  await flush();
  assert.deepEqual(h.calls.find(c => c.args?.action === 'detail.refresh').args, { owner: 'document-tabs', tabId: 'b', action: 'detail.refresh', payload: { page: 3 } });
  const count = h.calls.length;
  h.panel.control({ action: 'reader.noop' }); h.panel.control({ action: 'detail.refresh', disabled: true });
  h.panel.select(null); h.panel.control({ action: 'detail.refresh' });
  assert.equal(h.calls.length, count);
  h.panel.dispose();
});

test('external browser handoff uses a successful origin URL and is suppressed after disposal', async () => {
  for (const close of [false, true]) {
    const h = await loadDocumentTabs(); const pending = deferred();
    h.configure(command => command === 'browser_get_url' ? pending.promise : fallback(command));
    h.panel.select(row('a', 'browser')); const action = h.panel.external();
    h.panel.select(row('b', 'browser')); if (close) h.panel.dispose();
    pending.resolve('https://origin.example/current'); await action;
    assert.deepEqual(h.calls.filter(c => c.command === 'open_in_system_browser').map(c => c.args), close ? [] : [{ url: 'https://origin.example/current' }]);
    h.panel.dispose();
  }
});

test('failed external handoff is not retried using a different page or duplicated', async () => {
  const h = await loadDocumentTabs(); h.configure(command => command === 'browser_get_url' ? 'https://resolved.example/' : Promise.reject(new Error('handoff failed')));
  h.panel.select(row('a', 'browser')); await h.panel.external();
  assert.equal(h.calls.filter(c => c.command === 'open_in_system_browser').length, 1);
  h.panel.dispose();
});

test('navigation errors apply only to the current request and original page, including A to B to A', async () => {
  const h = await loadDocumentTabs(); const requests = [];
  h.configure(command => {
    if (command !== 'browser_navigate') return fallback(command);
    const pending = deferred(); requests.push(pending); return pending.promise;
  });
  h.panel.select(row('a', 'browser'));
  h.panel.setAddress('first.example'); h.panel.navigate();
  h.panel.setAddress('second.example'); h.panel.navigate();
  requests[0].reject(new Error('old navigation')); await flush(); assert.equal(h.panel.state.error, '');
  requests[1].reject(new Error('current navigation')); await flush(); assert.match(h.panel.state.error, /current navigation/);
  h.panel.setAddress('third.example'); h.panel.navigate();
  assert.equal(h.panel.state.error, '');
  h.panel.select(row('b', 'browser')); h.panel.select(row('a', 'browser'));
  requests[2].reject(new Error('earlier visit')); await flush(); assert.equal(h.panel.state.error, '');
  h.panel.setAddress('fourth.example'); h.panel.navigate(); h.panel.dispose();
  requests[3].reject(new Error('after close')); await flush(); assert.equal(h.panel.state.error, '');
  assert.ok(h.calls.filter(c => c.command === 'browser_navigate').every(c => c.args.target === 'surface-a'));
});

test('copy feedback remains on the page and URL that was copied, without touching a real clipboard', async () => {
  for (const change of ['tab', 'url', 'close', 'none']) {
    const h = await loadDocumentTabs(); const pending = deferred(), copied = [];
    h.configure(fallback); h.configureClipboard(value => { copied.push(value); return pending.promise; });
    h.panel.select(row('a', 'browser')); const action = h.panel.copy();
    if (change === 'tab') h.panel.select(row('b', 'browser'));
    if (change === 'url') h.panel.select({ ...row('a', 'browser'), url: 'https://changed.example/' });
    if (change === 'close') h.panel.dispose();
    pending.resolve(); await action;
    assert.deepEqual(copied, ['https://a.example/']);
    assert.equal(h.panel.state.copied, change === 'none');
    assert.equal(h.clock.active.size, change === 'none' ? 1 : 0);
    h.panel.dispose(); assert.equal(h.clock.active.size, 0);
  }
});

test('completed copy feedback is cleared on a page change and its old timeout cannot clear the new feedback', async () => {
  const h = await loadDocumentTabs(); h.configure(fallback);
  h.panel.select(row('a', 'browser')); await h.panel.copy(); const old = h.clock.created.at(-1);
  assert.equal(h.panel.state.copied, true);
  h.panel.select(row('b', 'browser')); assert.equal(h.panel.state.copied, false);
  assert.equal(h.clock.active.size, 0);
  await h.panel.copy(); assert.equal(h.panel.state.copied, true);
  old.callback(); assert.equal(h.panel.state.copied, true);
  const current = h.clock.created.at(-1); current.callback(); assert.equal(h.panel.state.copied, false);
  h.panel.dispose(); assert.equal(h.clock.active.size, 0);
});
