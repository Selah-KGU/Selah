import test from 'node:test';
import assert from 'node:assert/strict';
import { loadMarkdownReader } from './load-markdown-reader.mjs';
function deferred() { let resolve, reject; const promise = new Promise((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; }
async function flush() { for (let i = 0; i < 100; i++) await Promise.resolve(); }
const payload = (revision, markdown = '# 授業\n\n全文 👩🏽‍💻', error = '') => ({ path: '/fixture/note.md', filename: 'note.md', markdown, error, deliveryRevision: String(revision) });
const fallback = command => command === 'get_app_theme' ? 'light' : null;

test('the same delivery and older retries are parsed once and cannot reset a saved note', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback);
  h.reader.apply(payload(1)); await flush();
  h.reader.apply(payload(1)); await flush(); assert.equal(h.parses.length, 1);
  h.reader.enterEdit(); h.reader.edit('# Saved'); assert.equal(await h.reader.save(), true);
  h.reader.apply(payload(1)); await flush(); assert.equal(h.reader.state.markdown, '# Saved');
  h.reader.apply(payload(2, '# Reopened')); await flush(); assert.equal(h.reader.state.markdown, '# Reopened');
  h.reader.dispose();
});

test('an error delivered during parsing cannot be overwritten by the older render', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback); const parsing = deferred(); h.configureParser(() => parsing.promise);
  h.reader.apply(payload(1)); await flush(); h.reader.apply(payload(2, '', 'read failed'));
  parsing.resolve('<h1>old</h1>'); await flush();
  assert.deepEqual(h.reader.state.renderedSegments, []); assert.equal(h.reader.state.error, 'read failed'); h.reader.dispose();
});

test('edits made during saving remain unsaved and do not pretend to be the written snapshot', async () => {
  const h = await loadMarkdownReader(); const pending = deferred(); h.configure(command => command === 'write_markdown_file' ? pending.promise : fallback(command));
  h.reader.apply(payload(1)); await flush(); h.reader.enterEdit(); h.reader.edit('# submitted'); const saving = h.reader.save();
  h.reader.edit('# still typing'); pending.resolve(); await saving;
  assert.equal(h.reader.state.savedMarkdown, '# submitted'); assert.equal(h.reader.state.editorValue, '# still typing'); assert.equal(h.reader.state.editing, true);
  assert.equal(h.calls.find(c => c.command === 'write_markdown_file').args.contents, '# submitted'); h.reader.dispose();
});

test('read/control subscriptions are targeted and ready before blocked theme IO or pending reads', async () => {
  const h = await loadMarkdownReader(); const theme = deferred(), initial = deferred();
  h.configure(command => {
    if (command === 'get_app_theme' || command === 'get_pending_markdown_payload') assert.equal(h.listeners.length, 4);
    return command === 'get_app_theme' ? theme.promise : command === 'get_pending_markdown_payload' ? initial.promise : null;
  });
  h.reader.mount(); await flush();
  assert.deepEqual(h.listeners.filter(l => ['markdown-content', 'document-tab-control'].includes(l.name)).map(l => l.options), [{ target: 'reader-A' }, { target: 'reader-A' }]);
  h.listeners.find(l => l.name === 'markdown-content').receive({ payload: payload(5) }); await flush();
  assert.equal(h.reader.state.loading, false);
  assert.deepEqual(h.calls.find(c => c.command === 'ack_markdown_payload').args, { label: 'reader-A', deliveryRevision: '5' });
  initial.resolve(null); theme.resolve('light'); await flush();
  assert.equal(h.reader.state.error, ''); assert.equal(h.clock.active.size, 0); h.reader.dispose();
});

test('a page disposed during subscription registration releases every late handle and starts no IO', async () => {
  const h = await loadMarkdownReader(); const pending = [];
  h.configure(fallback, (l, release) => { const d = deferred(); pending.push({ d, release }); return d.promise; });
  h.reader.mount(); await flush(); assert.equal(pending.length, 4); h.reader.dispose();
  const closedCalls = h.calls.length, closedWrites = h.writes.length;
  h.listeners.find(l => l.name === 'markdown-content').receive({ payload: payload(1) });
  pending.forEach(({ d, release }) => d.resolve(release)); await flush();
  assert.deepEqual(h.listeners.map(l => l.releases), [1, 1, 1, 1]);
  assert.equal(h.calls.length, closedCalls); assert.equal(h.writes.length, closedWrites);
  assert.equal(h.parses.length, 0); assert.equal(h.clock.active.size, 0);
});

test('a failed subscription disables its group including other handles completing later', async () => {
  const h = await loadMarkdownReader(); const late = deferred(); let releaseLate;
  h.configure(fallback, (l, release) => {
    if (l.name === 'document-tab-control') return Promise.reject(new Error('registration failed'));
    if (l.name === 'markdown-content') { releaseLate = release; return late.promise; }
    return release;
  });
  h.reader.mount(); await flush(); assert.match(h.reader.state.error, /registration failed/);
  h.listeners.find(l => l.name === 'markdown-content').receive({ payload: payload(10) });
  late.resolve(releaseLate); await flush();
  assert.equal(h.parses.length, 0); assert.equal(h.reader.state.path, '');
  assert.equal(h.calls.filter(c => c.command === 'get_pending_markdown_payload').length, 0);
  assert.equal(h.listeners.find(l => l.name === 'markdown-content').releases, 1); h.reader.dispose();
});

test('startup retry delays are cancelled on delivery or close, including an already queued callback', async () => {
  for (const deliver of [true, false]) {
    const h = await loadMarkdownReader(); h.configure(fallback); h.reader.mount(); await flush();
    const timer = [...h.clock.active.values()].find(t => t.delay === 300); assert.ok(timer);
    if (deliver) h.listeners.find(l => l.name === 'markdown-content').receive({ payload: payload(1) });
    else h.reader.dispose();
    await flush();
    const calls = h.calls.length;
    timer.callback(); await flush();
    assert.equal(h.clock.active.size, 0); assert.equal(h.calls.length, calls);
    if (deliver) { assert.equal(h.reader.state.error, ''); h.reader.dispose(); }
  }
});

test('four genuine startup misses show an error but a later correct delivery can recover', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback); h.reader.mount(); await flush();
  for (const delay of [300, 700, 1500]) {
    const timer = [...h.clock.active.values()].find(t => t.delay === delay); assert.ok(timer); timer.callback(); await flush();
  }
  assert.equal(h.calls.filter(c => c.command === 'get_pending_markdown_payload').length, 4);
  assert.match(h.reader.state.error, /not delivered/);
  h.listeners.find(l => l.name === 'markdown-content').receive({ payload: payload(1) }); await flush();
  assert.equal(h.reader.state.error, ''); assert.equal(h.reader.state.loading, false); h.reader.dispose();
});

test('revisions above JS number precision remain ordered and a new revision can reopen identical content', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback);
  const first = 9007199254740992n;
  h.reader.apply(payload(first)); await flush(); h.reader.apply(payload(first + 1n)); await flush();
  assert.equal(h.parses.length, 2); const rendered = h.reader.state.renderedSegments;
  h.reader.apply(payload(first, 'older')); h.reader.apply({ ...payload(first + 2n), deliveryRevision: 'invalid' });
  await flush(); assert.equal(h.reader.state.renderedSegments, rendered); h.reader.dispose();
});

test('a new delivery during saving owns the document and a failed old write cannot show a stale toast', async () => {
  for (const fail of [false, true]) {
    const h = await loadMarkdownReader(); const write = deferred();
    h.configure(command => command === 'write_markdown_file' ? write.promise : fallback(command));
    h.reader.apply(payload(1)); await flush(); h.reader.enterEdit(); h.reader.edit('old edit'); const saving = h.reader.save();
    h.reader.apply(payload(2, 'fresh disk snapshot')); await flush();
    if (fail) write.reject(new Error('old save failed')); else write.resolve();
    assert.equal(await saving, false); assert.equal(h.reader.state.savedMarkdown, 'fresh disk snapshot');
    assert.equal(h.reader.state.editorValue, 'old edit'); assert.equal(h.reader.state.editing, true);
    assert.equal(h.reader.state.toastText, ''); assert.equal(h.reader.state.saving, false); h.reader.dispose();
  }
});

test('sharing waits for the saved snapshot and retains later unsaved edits without sharing them', async () => {
  for (const change of ['none', 'typing', 'reopen', 'close']) {
    const h = await loadMarkdownReader(); const write = deferred();
    h.configure(command => command === 'write_markdown_file' ? write.promise : fallback(command));
    h.reader.apply(payload(1)); await flush(); h.reader.enterEdit(); h.reader.edit('submitted'); const sharing = h.reader.share();
    if (change === 'typing') h.reader.edit('later edits');
    if (change === 'reopen') h.reader.apply(payload(2, 'new version'));
    if (change === 'close') h.reader.dispose();
    const callsBeforeCompletion = h.calls.length;
    write.resolve(); await sharing; await flush();
    assert.equal(h.calls.filter(c => c.command === 'share_downloaded_file_native').length, change === 'none' ? 1 : 0);
    if (change === 'close') assert.equal(h.calls.length, callsBeforeCompletion);
    else h.reader.dispose();
  }
});

test('parse failure is localized and a later version can render successfully', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback); h.configureParser(() => Promise.reject(new Error('parse failure')));
  h.reader.apply(payload(1)); await flush(); assert.match(h.reader.state.error, /parse failure/); assert.deepEqual(h.reader.state.renderedSegments, []);
  h.configureParser(() => '<h1>recovered</h1>'); h.reader.apply(payload(2)); await flush();
  assert.equal(h.reader.state.error, ''); assert.equal(h.reader.state.renderedSegments[0].html, '<h1>recovered</h1>'); h.reader.dispose();
});

test('closed parsing cannot publish segments or controls and stops before subsequent blocks', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback); const parse = deferred(); h.configureParser(() => parse.promise);
  h.reader.apply(payload(1, '# before\n```live-whiteboard\n{"nodes":[{"id":"node","label":"板書"}]}\n```\n# after'));
  await flush(); h.reader.dispose(); const count = h.calls.length; parse.resolve('<h1>before</h1>'); await flush();
  assert.equal(h.parses.length, 1); assert.deepEqual(h.reader.state.renderedSegments, []); assert.equal(h.calls.length, count);
});

test('full markdown and structured board order survive delivery and duplicates retain the same board references', async () => {
  const h = await loadMarkdownReader(); h.configure(fallback);
  const board = { nodes: [{ id: 'node', label: '板書 👩🏽‍💻' }], edges: [], topics: [{ id: 'topic', title: '授業' }] };
  const text = '# before\n\n' + '全文 🌕\n'.repeat(20000) + '\n```live-whiteboard\n' + JSON.stringify(board) + '\n```\n\n# after';
  h.reader.apply(payload(1, text)); await flush();
  assert.equal(h.reader.state.markdown, text); assert.equal(h.reader.state.savedMarkdown, text);
  const segments = h.reader.state.renderedSegments; assert.equal(segments.length, 3); assert.deepEqual(segments[1].board, board);
  assert.match(segments[0].html, /before/); assert.match(segments[2].html, /after/);
  const parses = h.parses.length; h.reader.apply(payload(1, text)); await flush();
  assert.equal(h.parses.length, parses); assert.equal(h.reader.state.renderedSegments, segments); assert.equal(h.reader.state.renderedSegments[1].board, segments[1].board); h.reader.dispose();
});

test('a pushed theme supersedes its older read and closing suppresses later DOM writes', async () => {
  const h = await loadMarkdownReader(); const read = deferred();
  h.configure(command => command === 'get_app_theme' ? read.promise : null); h.reader.mount(); await flush();
  h.listeners.find(l => l.name === 'theme-changed').receive({ payload: 'dark' }); read.resolve('light'); await flush();
  assert.deepEqual(h.writes.filter(w => w.key === 'data-theme').map(w => w.value), ['dark', 'dark']);
  h.reader.dispose(); const writes = h.writes.length;
  h.listeners.find(l => l.name === 'theme-changed').receive({ payload: 'light' }); await flush(); assert.equal(h.writes.length, writes);
});

test('a missing reader target cannot subscribe globally or invoke document commands', async () => {
  const h = await loadMarkdownReader({ query: '' }); h.configure(fallback); h.reader.mount(); await flush();
  assert.match(h.reader.state.error, /target is missing/); assert.equal(h.listeners.length, 0); assert.equal(h.calls.length, 0); h.reader.dispose();
});
