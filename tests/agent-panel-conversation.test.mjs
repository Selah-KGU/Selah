import test from "node:test";
import assert from "node:assert/strict";
import { loadAgentPanel } from "./load-agent-panel.mjs";

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const history = (id, content = id) => [{ id: 1, conv_id: id, role: "user", content }];
function backend(shared) {
  return (command, args) => {
    if (command === "agent_active_conversation") return shared;
    if (command === "agent_load_display_messages") return history(args.convId);
    if (command === "agent_list_conversations") return [{ id: shared, title: shared }];
    if (command === "agent_cancel") return;
    throw new Error(`Unexpected IPC ${command}`);
  };
}

test("panel deletion invalidates startup before its conversation ID arrives and rereads without creation", async () => {
  const h = await loadAgentPanel();
  const first = deferred();
  let reads = 0;
  h.configure((command, args) => command === "agent_active_conversation" && ++reads === 1
    ? first.promise : backend("B")(command, args));
  const pending = h.panel.load();
  h.panel.deleted("A");
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(h.panel.state.convId, "B");
  first.resolve("A");
  await pending;
  assert.equal(h.panel.state.convId, "B");
  assert.deepEqual(h.panel.state.messages, history("B"));
  assert.deepEqual(h.listeners.map(listener => listener.name), ["agent_stream:B"]);
  assert.equal(h.calls.some(call => /create|set_active|stt/.test(call.command)), false);
  h.panel.dispose();
});

test("panel deletion keeps newer selections and voice drafts while A to B to C to B rejects old history", async () => {
  const h = await loadAgentPanel();
  h.configure(backend("A"));
  await h.panel.load();
  h.panel.setDraft("認識済みの全文");
  h.panel.streaming("partial answer");
  const late = deferred();
  let historyReads = 0;
  h.configure((command, args) => command === "agent_load_display_messages" && ++historyReads === 1
    ? late.promise : backend("B")(command, args));
  const pending = h.panel.load(false);
  await Promise.resolve();
  h.panel.deleted("A");
  h.configure(backend("C"));
  await h.panel.load(false);
  h.configure(backend("B"));
  await h.panel.load(false);
  assert.equal(h.panel.state.convId, "B");
  late.resolve(history("B", "obsolete snapshot"));
  await pending;
  h.panel.deleted("A");
  assert.deepEqual(h.panel.state.messages, history("B"));
  assert.equal(h.panel.state.draft, "認識済みの全文");
  assert.equal(h.panel.state.sttCommittedText, "認識済みの全文");
  assert.equal(h.panel.state.attachments[0].data_base64, "AA==");
  assert.equal(h.listeners[0].releases, 1);
  assert.deepEqual(h.calls.filter(call => call.command === "agent_cancel").map(call => call.args.convId), ["A"]);
  assert.equal(h.calls.some(call => /create|set_active|stt/.test(call.command)), false);
  h.panel.dispose();
});

test("duplicate shared selection keeps the current answer, history and subscription", async () => {
  const h = await loadAgentPanel();
  h.configure(backend("A"));
  await h.panel.load();
  h.panel.streaming("answer continues");
  await h.panel.load(false);
  assert.equal(h.panel.state.sending, true);
  assert.equal(h.panel.state.streamText, "answer continues");
  assert.equal(h.calls.filter(call => call.command === "agent_load_display_messages").length, 1);
  assert.equal(h.listeners.length, 1);
  assert.equal(h.listeners[0].releases, 0);
  h.panel.dispose();
});

test("current read failures retain the existing panel and obsolete failures cannot affect a newer selection", async () => {
  const h = await loadAgentPanel();
  h.configure(backend("A"));
  await h.panel.load();
  h.panel.streaming("keep answer");
  h.configure(() => { throw new Error("database read failed"); });
  await assert.rejects(h.panel.load(false), /database read failed/);
  assert.equal(h.panel.state.convId, "A");
  assert.equal(h.panel.state.streamText, "keep answer");
  assert.equal(h.panel.state.sending, true);
  const late = deferred();
  h.configure((command, args) => command === "agent_load_display_messages" ? late.promise : backend("B")(command, args));
  const old = h.panel.load(false);
  await Promise.resolve();
  h.configure(backend("C"));
  await h.panel.load(false);
  late.reject(new Error("obsolete B read failed"));
  await old;
  assert.equal(h.panel.state.convId, "C");
  assert.deepEqual(h.panel.state.messages, history("C"));
  assert.equal(h.calls.some(call => /create|set_active/.test(call.command)), false);
  h.panel.dispose();
});

test("deletion without a replacement clears the panel and closing during IO rejects late state", async () => {
  const h = await loadAgentPanel();
  h.configure(backend("A"));
  await h.panel.load();
  h.panel.setDraft("keep draft");
  h.panel.deleted("A");
  h.configure(backend(null));
  await h.panel.load(false);
  assert.equal(h.panel.state.convId, "");
  assert.deepEqual(h.panel.state.messages, []);
  assert.equal(h.panel.state.draft, "keep draft");
  assert.equal(h.listeners[0].releases, 1);
  const late = deferred();
  h.configure((command, args) => command === "agent_active_conversation" ? late.promise : backend("B")(command, args));
  const pending = h.panel.load(false);
  h.panel.dispose();
  late.resolve("B");
  await pending;
  assert.equal(h.panel.state.convId, "");
  assert.equal(h.listeners.length, 1);
  assert.equal(h.calls.some(call => /create|set_active/.test(call.command)), false);
});

test("history failure never creates a new conversation and failed stream registration can retry the same ID", async () => {
  const h = await loadAgentPanel();
  h.configure(backend("A"));
  await h.panel.load();
  h.panel.streaming("keep previous answer");
  h.configure((command, args) => {
    if (command === "agent_load_display_messages") throw new Error("history decode failed");
    return backend("B")(command, args);
  });
  await assert.rejects(h.panel.load(), /history decode failed/);
  // Selection adopts B immediately. Failed B history never leaves A's rows
  // under B's title or lets a send bypass B's unsuccessful preparation.
  assert.equal(h.panel.state.convId, "B");
  assert.deepEqual(h.panel.state.messages, []);
  assert.equal(h.panel.state.conversationReady, false);
  assert.equal(h.listeners[0].releases, 1);
  h.configure(backend("B"), () => { throw new Error("registration failed"); });
  await assert.rejects(h.panel.load(false), /registration failed/);
  assert.equal(h.panel.state.convId, "B");
  assert.equal(h.panel.state.conversationReady, false);
  h.configure(backend("B"));
  await h.panel.load(false);
  assert.equal(h.panel.state.conversationReady, true);
  assert.equal(h.listeners.at(-1).name, "agent_stream:B");
  assert.equal(h.calls.some(call => /create|set_active/.test(call.command)), false);
  h.panel.dispose();
});

test("closing the panel while a subscription registers releases the late subscription", async () => {
  const h = await loadAgentPanel();
  const registering = deferred();
  let release;
  h.configure(backend("A"), (_, cleanup) => { release = cleanup; return registering.promise; });
  const pending = h.panel.load(false);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(h.listeners.length, 1);
  h.panel.dispose();
  registering.resolve(release);
  await pending;
  assert.equal(h.listeners[0].releases, 1);
  assert.equal(h.panel.state.conversationReady, false);
});
