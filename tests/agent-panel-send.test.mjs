import test from "node:test";
import assert from "node:assert/strict";
import { loadAgentPanel } from "./load-agent-panel.mjs";

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const settle = () => new Promise(resolve => setImmediate(resolve));
const history = (id, text = "stored history") => [{ id: 1, conv_id: id, role: "user", content: text }];
const createPanel = () => loadAgentPanel({ hash: "#owner=agent-popup&target=agent-popup&kind=agent" });
function backend(id, send, read = () => history(id)) {
  return (command, args) => {
    if (command === "agent_active_conversation") return id;
    if (command === "agent_load_display_messages") return read(args);
    if (command === "agent_list_conversations") return [{ id, title: id }];
    if (command === "agent_cancel") return;
    if (command === "agent_send") return send(args);
    throw new Error(`Unexpected IPC ${command}`);
  };
}
const sendCalls = h => h.calls.filter(call => call.command === "agent_send");

test("panel sends once and waits for shared startup history and subscription without duplicate IPC", async () => {
  const h = await createPanel();
  const registration = deferred(), reply = deferred();
  let release;
  h.configure(backend("A", () => reply.promise), (_, cleanup) => { release = cleanup; return registration.promise; });
  const startup = h.panel.load();
  await settle();
  h.panel.setDraft("送信する全文");
  const first = h.panel.send();
  await h.panel.send();
  assert.equal(h.panel.state.preparing, true);
  assert.equal(h.panel.state.draft, "送信する全文");
  assert.equal(sendCalls(h).length, 0);
  assert.equal(h.calls.filter(call => call.command === "agent_load_display_messages").length, 1);
  registration.resolve(release);
  await startup;
  await settle();
  assert.equal(sendCalls(h).length, 1);
  assert.equal(sendCalls(h)[0].args.convId, "A");
  assert.equal(sendCalls(h)[0].args.content, "送信する全文");
  assert.equal(sendCalls(h)[0].args.images[0].data_base64, "AA==");
  assert.equal(h.panel.state.preparing, false);
  assert.equal(h.panel.state.sending, true);
  assert.equal(h.calls.some(call => /create|set_active/.test(call.command)), false);
  reply.resolve();
  await first;
  h.panel.dispose();
});

test("failed preparation preserves text and images and permits an actual retry", async () => {
  const h = await createPanel();
  h.configure(backend("A", () => {}), () => { throw new Error("stream unavailable"); });
  h.panel.setDraft("認識済みの全文");
  await h.panel.send();
  assert.equal(sendCalls(h).length, 0);
  assert.equal(h.panel.state.preparing, false);
  assert.equal(h.panel.state.sending, false);
  assert.match(h.panel.state.error, /stream unavailable/);
  assert.equal(h.panel.state.draft, "認識済みの全文");
  assert.equal(h.panel.state.attachments.length, 1);
  h.configure(backend("A", () => {}));
  await h.panel.send();
  assert.equal(sendCalls(h).length, 1);
  assert.equal(h.panel.state.draft, "");
  assert.equal(h.panel.state.attachments.length, 0);
  h.panel.dispose();
});

test("cancelled preparation cannot send later or release a newer preparation lock", async () => {
  const h = await createPanel();
  const selected = deferred(), reply = deferred();
  h.configure((command, args) => command === "agent_active_conversation" ? selected.promise : backend("A", () => reply.promise)(command, args));
  h.panel.setDraft("old input");
  const old = h.panel.send();
  await h.panel.stop();
  h.panel.setDraft("new input");
  const newer = h.panel.send();
  assert.equal(h.panel.state.preparing, true);
  assert.equal(h.calls.filter(call => call.command === "agent_active_conversation").length, 1);
  selected.resolve("A");
  await old;
  await settle();
  assert.equal(sendCalls(h).length, 1);
  assert.equal(sendCalls(h)[0].args.content, "new input");
  assert.equal(h.panel.state.sending, true);
  assert.equal(h.calls.some(call => call.command === "agent_cancel"), false);
  reply.resolve();
  await newer;
  h.panel.dispose();
});

test("editing the draft and adding files while preparation waits retains the newer input", async () => {
  const h = await createPanel();
  const selected = deferred(), reply = deferred();
  h.configure((command, args) => command === "agent_active_conversation" ? selected.promise : backend("A", () => reply.promise)(command, args));
  h.panel.setDraft("original input");
  const send = h.panel.send();
  h.panel.setDraft("next input");
  const nextImages = h.panel.state.attachments;
  selected.resolve("A");
  await settle();
  assert.equal(sendCalls(h).length, 1);
  assert.equal(sendCalls(h)[0].args.content, "original input");
  assert.equal(h.panel.state.draft, "next input");
  assert.deepEqual(h.panel.state.attachments, nextImages);
  assert.equal(h.panel.state.attachments[0], nextImages[0]);
  reply.resolve();
  await send;
  h.panel.dispose();
});

test("an older RPC failure cannot finish or report an error on a new turn in the same conversation", async () => {
  const h = await createPanel();
  const firstReply = deferred(), secondReply = deferred();
  let count = 0;
  h.configure(backend("A", () => ++count === 1 ? firstReply.promise : secondReply.promise));
  await h.panel.load();
  h.panel.setDraft("first turn");
  const first = h.panel.send();
  await settle();
  await h.panel.stop();
  h.panel.setDraft("second turn");
  const second = h.panel.send();
  await settle();
  firstReply.reject(new Error("late first failure"));
  await first;
  assert.equal(sendCalls(h).length, 2);
  assert.equal(h.panel.state.sending, true);
  assert.equal(h.panel.state.error, "");
  assert.equal(h.panel.state.messages.at(-1).content, "second turn");
  secondReply.resolve();
  await second;
  h.panel.dispose();
});

test("an old completion history read cannot erase a new optimistic message or its active turn", async () => {
  const h = await createPanel();
  const firstReply = deferred(), secondReply = deferred(), oldHistory = deferred();
  let count = 0, reads = 0;
  h.configure(backend("A", () => ++count === 1 ? firstReply.promise : secondReply.promise,
    () => ++reads === 2 ? oldHistory.promise : history("A")));
  await h.panel.load();
  h.panel.setDraft("first turn");
  const first = h.panel.send();
  await settle();
  h.listeners[0].receive({ payload: { type: "done", turn_id: sendCalls(h)[0].args.turnId } });
  assert.equal(h.panel.state.sending, false);
  h.panel.setDraft("second turn");
  const second = h.panel.send();
  await settle();
  oldHistory.resolve(history("A", "snapshot before second turn"));
  await settle();
  assert.equal(h.panel.state.messages.at(-1).content, "second turn");
  assert.equal(h.panel.state.sending, true);
  firstReply.resolve();
  await first;
  assert.equal(h.panel.state.sending, true);
  secondReply.resolve();
  await second;
  h.panel.dispose();
});

test("A to B to A cannot let the original A RPC clear the newer A turn", async () => {
  const h = await createPanel();
  const oldReply = deferred(), newReply = deferred();
  h.configure(backend("A", () => oldReply.promise));
  await h.panel.load();
  h.panel.setDraft("old A");
  const old = h.panel.send();
  await settle();
  h.configure(backend("B", () => {}));
  await h.panel.load(false);
  h.configure(backend("A", () => newReply.promise));
  await h.panel.load(false);
  h.panel.setDraft("new A");
  const newer = h.panel.send();
  await settle();
  oldReply.reject(new Error("old A failed"));
  await old;
  assert.equal(h.panel.state.convId, "A");
  assert.equal(h.panel.state.sending, true);
  assert.equal(h.panel.state.error, "");
  assert.equal(h.panel.state.messages.at(-1).content, "new A");
  newReply.resolve();
  await newer;
  h.panel.dispose();
});

test("switching during preparation does not submit the old draft to the new conversation", async () => {
  const h = await createPanel();
  const selected = deferred();
  h.configure(backend("A", () => {}));
  await h.panel.load();
  h.configure((command, args) => command === "agent_active_conversation" ? selected.promise : backend("B", () => {})(command, args));
  const switching = h.panel.load(false);
  h.panel.setDraft("intended for A");
  const sending = h.panel.send();
  selected.resolve("B");
  await switching;
  await sending;
  assert.equal(sendCalls(h).length, 0);
  assert.equal(h.panel.state.convId, "B");
  assert.equal(h.panel.state.draft, "intended for A");
  assert.equal(h.panel.state.attachments.length, 1);
  assert.equal(h.panel.state.preparing, false);
  h.panel.dispose();
});

test("closing during preparation never consumes a draft or starts inference", async () => {
  const h = await createPanel();
  const registration = deferred();
  let release;
  h.configure(backend("A", () => {}), (_, cleanup) => { release = cleanup; return registration.promise; });
  h.panel.setDraft("retain on close");
  const sending = h.panel.send();
  await settle();
  h.panel.dispose();
  registration.resolve(release);
  await sending;
  assert.equal(sendCalls(h).length, 0);
  assert.equal(h.panel.state.draft, "retain on close");
  assert.equal(h.listeners[0].releases, 1);
});

test("a duplicate same-selection read during preparation still sends once after current readiness", async () => {
  const h = await createPanel();
  const firstRead = deferred();
  h.configure(backend("A", () => {}));
  await h.panel.load();
  let reads = 0;
  h.configure((command, args) => command === "agent_active_conversation" && ++reads === 1
    ? firstRead.promise : backend("A", () => {})(command, args));
  const previous = h.panel.load(false);
  h.panel.setDraft("same conversation");
  const sending = h.panel.send();
  await h.panel.load(false);
  firstRead.resolve("A");
  await previous;
  await sending;
  assert.equal(sendCalls(h).length, 1);
  assert.equal(sendCalls(h)[0].args.content, "same conversation");
  h.panel.dispose();
});

test("only sent attachments are consumed when a file is added during preparation", async () => {
  const h = await createPanel();
  const selected = deferred(), reply = deferred();
  h.configure((command, args) => command === "agent_active_conversation" ? selected.promise : backend("A", () => reply.promise)(command, args));
  h.panel.setDraft("send with first image");
  const sending = h.panel.send();
  const added = { mime: "image/png", data_base64: "BB==" };
  h.panel.addAttachment(added);
  selected.resolve("A");
  await settle();
  assert.deepEqual(sendCalls(h)[0].args.images.map(image => image.data_base64), ["AA=="]);
  assert.deepEqual(h.panel.state.attachments, [added]);
  reply.resolve();
  await sending;
  h.panel.dispose();
});

test("context lookup remains cancellable preparation and never consumes an unsent draft", async () => {
  const h = await loadAgentPanel({ hash: "#owner=document-tabs&target=page-A&kind=browser&title=Page" });
  const tabs = deferred();
  h.configure((command, args) => command === "document_tabs_list" ? tabs.promise : backend("A", () => {})(command, args));
  await h.panel.load();
  const original = h.panel.state.messages;
  h.panel.setDraft("question about this page");
  const sending = h.panel.send();
  await settle();
  assert.equal(h.panel.state.preparing, true);
  assert.equal(h.panel.state.sending, false);
  assert.equal(h.panel.state.draft, "question about this page");
  assert.deepEqual(h.panel.state.messages, original);
  await h.panel.stop();
  tabs.resolve([{ id: "tab", target: "page-A", title: "Page", type: "browser", active: true }]);
  await sending;
  assert.equal(h.calls.some(call => call.command.startsWith("agent_send")), false);
  assert.equal(h.calls.some(call => call.command === "agent_cancel"), false);
  assert.equal(h.panel.state.draft, "question about this page");
  assert.equal(h.panel.state.attachments.length, 1);
  assert.deepEqual(h.panel.state.messages, original);
  h.panel.dispose();
});

test("slow title metadata does not block an otherwise ready conversation or send", async () => {
  const h = await createPanel();
  const title = deferred();
  h.configure((command, args) => command === "agent_list_conversations" ? title.promise : backend("A", () => {})(command, args));
  await h.panel.load();
  h.panel.setDraft("ready before title");
  await h.panel.send();
  assert.equal(sendCalls(h).length, 1);
  assert.equal(sendCalls(h)[0].args.content, "ready before title");
  title.resolve([{ id: "A", title: "slow metadata" }]);
  await settle();
  h.panel.dispose();
});
