import test from "node:test";
import assert from "node:assert/strict";
import { runInNewContext } from "node:vm";
import { build } from "esbuild";
import { loadFrontendHealthProbe } from './load-frontend-health-probe.mjs';

// Exercise the exact script submitted by Rust, rather than a JS copy of it.
const probe = await loadFrontendHealthProbe();

const NodeFilter = { SHOW_TEXT: 4, SHOW_CDATA_SECTION: 8 };
function walkerDocument(root) {
  return {
    readyState: 'complete', visibilityState: 'visible', getElementById: () => root, querySelector: () => null,
    createTreeWalker(target, filter) {
      assert.equal(target,root);assert.equal(filter,12);
      let index=-1;
      return {nextNode() {return ++index < root.nodes.length;},get currentNode() {return root.nodes[index];}};
    },
  };
}

test("native probe reports DOM state without sending page or transcript text", () => {
  let sent;
  runInNewContext(probe, {
    window: { innerWidth: 1200, innerHeight: 800,
      __SELAH_PREBOOT_LOGS__: [{ type: "error" }, { type: "info" }],
      __TAURI_INTERNALS__: { invoke: (command, args) => { sent = { command, ...args }; return Promise.resolve(); } },
    },
    NodeFilter,
    document: walkerDocument({childElementCount:1,nodes:[{data:'private speech'}],
      get textContent() {throw new Error('health diagnostics must not join private page text');}}),
  });
  assert.equal(sent.command, "frontend_health_report");
  assert.equal(sent.report.sequence, 7);
  assert.equal(sent.report.rootTextLength, 14);
  assert.equal(sent.report.errors, 1);
  assert.equal(sent.report.recovery, false);
  assert.equal(JSON.stringify(sent).includes("private speech"), false);
});

test("native probe can report an empty root and the render recovery screen", () => {
  let sent;
  runInNewContext(probe, {
    window: { innerWidth: 0, innerHeight: 0,
      __TAURI_INTERNALS__: { invoke: (_command, args) => { sent = args.report; return Promise.resolve(); } },
    },
    document: { readyState: "complete", visibilityState: "visible",
      getElementById: () => null, querySelector: () => ({}),
    },
  });
  assert.equal(sent.rootChildren, 0);
  assert.equal(sent.rootTextLength, 0);
  assert.equal(sent.recovery, true);
});

test('native probe counts UTF-16 text-node units exactly without requesting aggregate page text', () => {
  const strings=['课程🌙','e\u0301','line\nwith\ttabs','', '\ud800', 'CDATA <source>'];
  let sent, reads=0;
  const root={childElementCount:4,nodes:strings.map(data=>({get data() {reads++;return data;}})),
    get textContent() {throw new Error('aggregate text allocation');}};
  runInNewContext(probe,{NodeFilter,document:walkerDocument(root),window:{innerWidth:1200,innerHeight:800,
    __TAURI_INTERNALS__:{invoke:(_command,args)=>{sent=args.report;return Promise.resolve();}}}});
  assert.equal(sent.rootTextLength,strings.join('').length);
  assert.equal(reads,strings.length);assert.equal(sent.errors,0);
  assert.ok(strings.every(value=>!value||!JSON.stringify(sent).includes(value)));
});

test('a large diagnostic reads each existing node once without reading the combined text', () => {
  const fragment='private 🌓 caption '.repeat(128), nodes=Array.from({length:10000},()=>({data:fragment}));
  let sent, aggregateReads=0;
  const root={childElementCount:100,nodes,get textContent() {aggregateReads++;return nodes.map(n=>n.data).join('');}};
  runInNewContext(probe,{NodeFilter,document:walkerDocument(root),window:{innerWidth:1,innerHeight:1,
    __TAURI_INTERNALS__:{invoke:(_command,args)=>{sent=args.report;return Promise.resolve();}}}});
  assert.equal(sent.rootTextLength,fragment.length*nodes.length);assert.equal(aggregateReads,0);
  assert.equal(sent.rootChildren,100);assert.equal(JSON.stringify(sent).includes('caption'),false);
});

test('a missing native bridge performs no DOM traversal', () => {
  runInNewContext(probe,{window:{},document:{getElementById(){throw new Error('unneeded DOM traversal');}}});
});

const productionBootstrap = await build({
  entryPoints: ["src/bootstrap.ts"], bundle: true, write: false,
  format: "iife", drop: ["console", "debugger"],
  plugins: [{ name: "isolate-bootstrap", setup(builder) {
    builder.onResolve({ filter: /^\.\// }, args =>
      args.importer.endsWith("bootstrap.ts") ? { path: args.path, namespace: "stub" } : undefined);
    builder.onLoad({ filter: /.*/, namespace: "stub" }, args => ({
      contents: args.path.endsWith("surfaceKind")
        ? "export const isAuxiliarySurface = () => true" : "export {}", loader: "js",
    }));
  } }],
});

test("production render errors reach native logs after console calls are stripped", async () => {
  const sent = [];
  const stored = new Map();
  const window = { location: { href: "http://test/" }, addEventListener() {},
    __TAURI_INTERNALS__: { invoke: (command, args) => { sent.push({ command, ...args }); return Promise.resolve(); } },
  };
  runInNewContext(productionBootstrap.outputFiles[0].text, {
    window, navigator: { userAgent: "test" },
    document: { getElementById: () => ({}), body: { classList: { add() {} } } },
    localStorage: { getItem: key => stored.get(key), setItem: (key, value) => stored.set(key, value) },
    console: { error() {}, warn() {} }, URLSearchParams,
  });
  window.__SELAH_REPORT_ERROR__("render failed");
  await Promise.resolve();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].command, "frontend_report_error");
  assert.equal(sent[0].message, "render failed");
  assert.equal(JSON.parse(stored.get("selah-frontend-errors"))[0].message, "render failed");
});
