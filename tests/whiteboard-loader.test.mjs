import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { ensureWhiteboardLayout, prepareWhiteboardLayout, whiteboardLayoutReady } = await loadTypeScript("src/lib/whiteboardLayout.ts");
const engine = () => ({ compute: () => null, topics: () => [] });

function environment() {
  const previous = Object.fromEntries(["window", "document", "HTMLScriptElement"].map(key => [key, globalThis[key]]));
  const scripts = [], connected = new Set();
  let failure = null;
  class Script extends EventTarget {
    src = "";
    async = false;
    dataset = {};
    listeners = new Map();
    captured = new Map();
    addEventListener(type, callback, options) {
      this.listeners.set(type, callback); this.captured.set(type, callback);
      super.addEventListener(type, callback, options);
    }
    removeEventListener(type, callback) {
      if (this.listeners.get(type) === callback) this.listeners.delete(type);
      super.removeEventListener(type, callback);
    }
    remove() { connected.delete(this); }
  }
  globalThis.HTMLScriptElement = Script;
  globalThis.window = {};
  globalThis.document = {
    querySelector: () => { if (failure === "query") throw new Error("query failure"); return [...connected][0] ?? null; },
    createElement: () => { if (failure === "create") throw new Error("create failure"); const script = new Script(); scripts.push(script); return script; },
    head: { appendChild: script => { if (failure === "append") throw new Error("append failure"); connected.add(script); } },
  };
  whiteboardLayoutReady.set(false);
  return {
    scripts, connected, Script, failAt: value => { failure = value; },
    ready: () => { let value; whiteboardLayoutReady.subscribe(next => { value = next; })(); return value; },
    restore() {
      for (const [key, value] of Object.entries(previous)) { if (value === undefined) delete globalThis[key]; else globalThis[key] = value; }
      whiteboardLayoutReady.set(false);
    },
  };
}

test("a failed load removes its script and the next request can complete with a fresh script", async () => {
  const dom = environment();
  try {
    const first = ensureWhiteboardLayout();
    const rejected = assert.rejects(first, /failed to load/);
    dom.scripts[0].dispatchEvent(new Event("error")); await rejected;
    const next = ensureWhiteboardLayout();
    assert.equal(dom.scripts.length, 2, "retry must create a script instead of waiting on a completed error event");
    assert.equal(dom.connected.size, 1);
    assert.equal(dom.scripts[0].listeners.size, 0);
    window.WhiteboardLayout = engine();
    dom.scripts[1].dispatchEvent(new Event("load")); await next;
    assert.equal(dom.ready(), true);
    assert.equal(dom.scripts[1].listeners.size, 0);
  } finally { dom.restore(); }
});

test("concurrent callers share one pending script and publish readiness only on a valid load", async () => {
  const dom = environment();
  try {
    const first = ensureWhiteboardLayout();
    for (let i = 0; i < 100; i++) assert.equal(ensureWhiteboardLayout(), first);
    assert.equal(dom.scripts.length, 1);
    assert.equal(dom.ready(), false);
    assert.equal(dom.scripts[0].src, "/whiteboard-layout.js");
    assert.equal(dom.scripts[0].async, true);
    assert.equal(dom.scripts[0].dataset.whiteboardLayout, "1");
    window.WhiteboardLayout = engine();
    dom.scripts[0].dispatchEvent(new Event("load")); await first;
    assert.equal(dom.ready(), true);
    assert.equal(dom.scripts[0].listeners.size, 0);
    await ensureWhiteboardLayout();
    assert.equal(dom.scripts.length, 1);
  } finally { dom.restore(); }
});

test("missing or incomplete exports reject once and remain retryable", async () => {
  const dom = environment();
  try {
    for (const exports of [undefined, {}, { compute: () => null }, { topics: () => [] }]) {
      window.WhiteboardLayout = exports;
      const pending = ensureWhiteboardLayout();
      const rejected = assert.rejects(pending, /without.*WhiteboardLayout/);
      dom.scripts.at(-1).dispatchEvent(new Event("load")); await rejected;
      assert.equal(dom.connected.size, 0);
      assert.equal(dom.scripts.at(-1).listeners.size, 0);
      assert.equal(dom.ready(), false);
    }
    delete window.WhiteboardLayout;
    const next = ensureWhiteboardLayout();
    window.WhiteboardLayout = engine(); dom.scripts.at(-1).dispatchEvent(new Event("load")); await next;
    assert.equal(dom.ready(), true);
  } finally { dom.restore(); }
});

test("late callbacks from a failed attempt cannot settle or clear a new attempt", async () => {
  const dom = environment();
  try {
    const first = ensureWhiteboardLayout(), old = dom.scripts[0];
    const rejected = assert.rejects(first);
    old.dispatchEvent(new Event("error")); await rejected;
    const next = ensureWhiteboardLayout();
    const current = dom.scripts[1];
    old.captured.get("load")(new Event("load"));
    old.captured.get("error")(new Event("error"));
    assert.equal(ensureWhiteboardLayout(), next);
    assert.equal(dom.connected.size, 1);
    assert.equal(dom.ready(), false);
    window.WhiteboardLayout = engine(); current.dispatchEvent(new Event("load")); await next;
    assert.equal(dom.ready(), true);
    old.captured.get("error")(new Event("error"));
    assert.equal(dom.ready(), true);
  } finally { dom.restore(); }
});

test("synchronous DOM failures reject through the promise and do not poison later attempts", async () => {
  const dom = environment();
  try {
    for (const point of ["query", "create", "append"]) {
      dom.failAt(point);
      let pending;
      assert.doesNotThrow(() => { pending = ensureWhiteboardLayout(); });
      await assert.rejects(pending, new RegExp(`${point} failure`));
      assert.equal(dom.connected.size, 0);
      for (const script of dom.scripts) assert.equal(script.listeners.size, 0);
    }
    dom.failAt(null);
    const next = ensureWhiteboardLayout();
    window.WhiteboardLayout = engine(); dom.scripts.at(-1).dispatchEvent(new Event("load")); await next;
    assert.equal(dom.ready(), true);
  } finally { dom.restore(); }
});

test("a stale tagged script is replaced rather than awaiting an event that may already have fired", async () => {
  const dom = environment();
  try {
    const stale = new dom.Script(); stale.dataset.whiteboardLayout = "1"; dom.connected.add(stale);
    const pending = ensureWhiteboardLayout();
    assert.equal(dom.scripts.length, 1);
    assert.equal(dom.connected.has(stale), false);
    window.WhiteboardLayout = engine(); dom.scripts[0].dispatchEvent(new Event("load")); await pending;
    assert.equal(dom.connected.size, 1);
    assert.equal(dom.ready(), true);
  } finally { dom.restore(); }
});

test("an existing valid engine and non-browser imports require no script", async () => {
  const dom = environment();
  try {
    window.WhiteboardLayout = engine(); await ensureWhiteboardLayout();
    assert.equal(dom.ready(), true);
    assert.equal(dom.scripts.length, 0);
    delete globalThis.document; await ensureWhiteboardLayout();
    delete globalThis.window; await ensureWhiteboardLayout();
    assert.equal(dom.scripts.length, 0);
  } finally { dom.restore(); }
});

test("board preparation shares its first attempt and single retry across concurrent views", async () => {
  const dom = environment();
  try {
    const preparations = Array.from({ length: 100 }, () => prepareWhiteboardLayout());
    assert.equal(dom.scripts.length, 1);
    dom.scripts[0].dispatchEvent(new Event("error"));
    await Promise.resolve();
    assert.equal(dom.scripts.length, 2);
    assert.equal(dom.connected.size, 1);
    window.WhiteboardLayout = engine(); dom.scripts[1].dispatchEvent(new Event("load"));
    await Promise.all(preparations);
    assert.equal(dom.ready(), true);
    for (const script of dom.scripts) assert.equal(script.listeners.size, 0);
  } finally { dom.restore(); }
});

test("persistent preparation failure stops after two attempts and a later board can try again", async () => {
  const dom = environment();
  try {
    const pending = prepareWhiteboardLayout(), rejected = assert.rejects(pending, /failed to load/);
    dom.scripts[0].dispatchEvent(new Event("error")); await Promise.resolve();
    dom.scripts[1].dispatchEvent(new Event("error")); await rejected;
    await Promise.resolve();
    assert.equal(dom.scripts.length, 2);
    assert.equal(dom.connected.size, 0);
    const later = prepareWhiteboardLayout();
    window.WhiteboardLayout = engine(); dom.scripts[2].dispatchEvent(new Event("load")); await later;
    assert.equal(dom.scripts.length, 3);
    assert.equal(dom.ready(), true);
  } finally { dom.restore(); }
});
