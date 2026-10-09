import test from "node:test";
import assert from "node:assert/strict";
import { loadAgentChat } from "./load-agent-chat.mjs";
import { loadAgentPanel } from "./load-agent-panel.mjs";

const settle = () => new Promise(resolve => setImmediate(resolve));
const part = name => ({ mime: "image/png", data_base64: name });
const file = (name = "test.png", type = "image/png") => ({ name, type, size: 4 });
function readerBoundary() {
  const original = globalThis.FileReader;
  const reads = [];
  globalThis.FileReader = class {
    readAsDataURL(file) { this.file = file; reads.push(this); }
    finish(data = "QUJDRA==") { this.result = `data:image/png;base64,${data}`; this.onload?.(); }
    fail() { this.onerror?.(); }
  };
  return { reads, restore() { globalThis.FileReader = original; } };
}
const surfaces = [
  ["main chat", loadAgentChat],
  ["sidebar", () => loadAgentPanel({ hash: "#owner=agent-popup&target=agent-popup&kind=agent" })],
];
for (const [name, load] of surfaces) {
  test(`${name}: a mixed selection retains valid files and names the failed attachment`, async () => {
    const boundary = readerBoundary(), h = await load();
    try {
      const adding = h.panel.addFiles([file("broken.exe", "application/octet-stream"), file("good.png")]);
      await settle();
      assert.equal(boundary.reads.length, 1);
      assert.match(h.panel.attachmentState.error, /broken\.exe:.*対応形式/);
      boundary.reads[0].finish();
      await adding;
      assert.deepEqual(h.panel.attachmentState.attachments, [part("QUJDRA==")]);
      assert.match(h.panel.attachmentState.error, /broken\.exe/);
    } finally { h.panel.dispose(); boundary.restore(); }
  });

  test(`${name}: extraction failure is visible and a later image can still be sent`, async () => {
    const boundary = readerBoundary(), h = await load();
    let finishSend;
    const reply = new Promise(resolve => { finishSend = resolve; });
    h.configure(command => {
      if (command === "agent_read_document_attachment") throw new Error("PDFに読み取れる文字がありません");
      if (command === "agent_active_conversation") return "A";
      if (command === "agent_load_display_messages") return [];
      if (command === "agent_list_conversations") return [{ id: "A", title: "A" }];
      if (command === "agent_set_active_conversation" || command === "agent_cancel") return;
      if (command === "agent_send") return reply;
      throw new Error(`Unexpected IPC ${command}`);
    });
    let sending;
    try {
      if (h.panel.select) await h.panel.select("A"); else await h.panel.load();
      const adding = h.panel.addFiles([file("scan.pdf", "application/pdf")]);
      boundary.reads[0].finish(); await adding;
      assert.match(h.panel.attachmentState.error, /読み取れる文字/);
      assert.equal(h.panel.attachmentState.attachments.length, 0);
      const retry = h.panel.addFiles([file()]);
      boundary.reads[1].finish(); await retry;
      sending = h.panel.send(); await settle();
      assert.deepEqual(h.calls.find(c => c.command === "agent_send").args.images, [part("QUJDRA==")]);
    } finally { h.panel.dispose(); finishSend(); await sending; boundary.restore(); }
  });

  test(`${name}: document-only sends wait for extraction and carry the complete typed document`, async () => {
    const boundary = readerBoundary(), h = await load();
    let finishExtraction, finishSend;
    const extraction = new Promise(resolve => { finishExtraction = resolve; });
    const reply = new Promise(resolve => { finishSend = resolve; });
    const document = { name: "資料.pdf", mime: "application/pdf", size: 4, text: "日本語の資料\n完全な内容 🌕", truncated: false };
    h.configure(command => {
      if (command === "agent_read_document_attachment") return extraction;
      if (command === "agent_active_conversation") return "A";
      if (command === "agent_load_display_messages") return [];
      if (command === "agent_list_conversations") return [{ id: "A", title: "A" }];
      if (command === "agent_set_active_conversation" || command === "agent_cancel") return;
      if (command === "agent_send") return reply;
      throw new Error(`Unexpected IPC ${command}`);
    });
    let sending, adding;
    try {
      if (h.panel.select) await h.panel.select("A"); else await h.panel.load();
      adding = h.panel.addFiles([file("資料.pdf", "application/pdf")]);
      boundary.reads[0].finish();
      await settle();
      assert.deepEqual(h.calls.find(c => c.command === "agent_read_document_attachment").args, { name: "資料.pdf", dataBase64: "QUJDRA==" });
      await h.panel.send();
      assert.equal(h.calls.filter(c => c.command === "agent_send").length, 0);
      finishExtraction(document);
      await adding;
      assert.deepEqual(h.panel.attachmentState.attachments, [document]);
      sending = h.panel.send();
      await settle();
      const args = h.calls.find(c => c.command === "agent_send").args;
      assert.equal(args.content, "");
      assert.deepEqual(args.images, []);
      assert.deepEqual(args.documents, [document]);
      assert.deepEqual(h.panel.state.messages.at(-1).documents, [document]);
    } finally {
      h.panel.dispose(); finishExtraction(document); finishSend();
      await adding; await sending; boundary.restore();
    }
  });

  test(`${name}: a PNG without a MIME type becomes an attachment`, async () => {
    const boundary = readerBoundary(), h = await load();
    try {
      const adding = h.panel.addFiles([file("日本語.PNG", "")]);
      assert.equal(boundary.reads.length, 1);
      boundary.reads[0].finish();
      await adding;
      assert.deepEqual(h.panel.attachmentState.attachments, [part("QUJDRA==")]);
    } finally { h.panel.dispose(); boundary.restore(); }
  });

  test(`${name}: file read failure and unsupported files have visible errors`, async () => {
    const boundary = readerBoundary(), h = await load();
    try {
      const adding = h.panel.addFiles([file()]);
      boundary.reads[0].fail();
      await adding;
      assert.match(h.panel.attachmentState.error, /読み込/);
      assert.equal(h.panel.attachmentState.attachments.length, 0);
      await h.panel.addFiles([file("document.exe", "application/octet-stream")]);
      assert.match(h.panel.attachmentState.error, /画像/);
      assert.equal(boundary.reads.length, 1);
    } finally { h.panel.dispose(); boundary.restore(); }
  });

  test(`${name}: concurrent reads respect the four-attachment limit`, async () => {
    const boundary = readerBoundary(), h = await load();
    try {
      for (const name of ["one", "two", "three"]) h.panel.addAttachment(part(name));
      const first = h.panel.addFiles([file("first.png")]);
      const second = h.panel.addFiles([file("second.png")]);
      assert.equal(boundary.reads.length, 2);
      boundary.reads[0].finish("first");
      await first;
      boundary.reads[1].finish("second");
      await second;
      assert.deepEqual(h.panel.attachmentState.attachments.map(p => p.data_base64), ["one", "two", "three", "first"]);
      assert.match(h.panel.attachmentState.error, /最大4/);
    } finally { h.panel.dispose(); boundary.restore(); }
  });

  test(`${name}: disposing the view cannot append a late file read`, async () => {
    const boundary = readerBoundary(), h = await load();
    try {
      const adding = h.panel.addFiles([file()]);
      h.panel.dispose();
      boundary.reads[0].finish();
      await adding;
      assert.equal(h.panel.attachmentState.attachments.length, 0);
    } finally { h.panel.dispose(); boundary.restore(); }
  });

  test(`${name}: abort and malformed data release the file-read gate for a retry`, async () => {
    const boundary = readerBoundary(), h = await load();
    try {
      const input = { files: [file()], value: "selected-file" };
      const picking = h.panel.pick({ currentTarget: input });
      assert.equal(input.value, "");
      assert.equal(typeof boundary.reads[0].onabort, "function");
      boundary.reads[0].onabort();
      await picking;
      assert.match(h.panel.attachmentState.error, /読み込/);

      const invalid = h.panel.addFiles([file()]);
      boundary.reads[1].result = "not a data URL";
      boundary.reads[1].onload();
      await invalid;
      assert.equal(h.panel.attachmentState.attachments.length, 0);
      assert.match(h.panel.attachmentState.error, /読み込/);

      const retry = h.panel.addFiles([file()]);
      boundary.reads[2].finish();
      await retry;
      assert.deepEqual(h.panel.attachmentState.attachments, [part("QUJDRA==")]);
      assert.equal(h.panel.attachmentState.error, "");
    } finally { h.panel.dispose(); boundary.restore(); }
  });

  test(`${name}: a pending file read prevents sending an incomplete attachment list`, async () => {
    const boundary = readerBoundary(), h = await load();
    let finishSend;
    const reply = new Promise(resolve => { finishSend = resolve; });
    h.configure(command => {
      if (command === "agent_active_conversation") return "A";
      if (command === "agent_load_display_messages") return [];
      if (command === "agent_list_conversations") return [{ id: "A", title: "A" }];
      if (command === "agent_set_active_conversation" || command === "agent_cancel") return;
      if (command === "agent_send") return reply;
      throw new Error(`Unexpected IPC ${command}`);
    });
    let sending;
    try {
      if (h.panel.select) await h.panel.select("A"); else await h.panel.load();
      h.panel.setDraft("写真を確認");
      const adding = h.panel.addFiles([file()]);
      sending = h.panel.send();
      await settle();
      assert.equal(h.calls.filter(c => c.command === "agent_send").length, 0);
      boundary.reads[0].finish();
      await adding;
      await sending;
      sending = h.panel.send();
      await settle();
      assert.deepEqual(h.calls.find(c => c.command === "agent_send").args.images.at(-1), part("QUJDRA=="));
    } finally {
      h.panel.dispose(); boundary.reads.forEach(r => r.finish()); finishSend();
      await sending; boundary.restore();
    }
  });
}

test("main chat preserves attachments added while conversation history is preparing", async () => {
  const h = await loadAgentChat();
  let finishHistory, finishSend;
  const history = new Promise(resolve => { finishHistory = resolve; });
  const reply = new Promise(resolve => { finishSend = resolve; });
  h.configure(command => {
    if (command === "agent_load_display_messages") return history;
    if (command === "agent_list_conversations") return [{ id: "A", title: "A" }];
    if (command === "agent_set_active_conversation" || command === "agent_cancel") return;
    if (command === "agent_send") return reply;
    throw new Error(`Unexpected IPC ${command}`);
  });
  const selected = h.panel.select("A");
  h.panel.setDraft("first message");
  const sent = part("first");
  h.panel.addAttachment(sent);
  const sending = h.panel.send();
  const next = part("next");
  h.panel.addAttachment(next);
  h.panel.setDraft("next message");
  try {
    finishHistory([]);
    await selected;
    await settle();
    assert.deepEqual(h.calls.find(c => c.command === "agent_send").args.images, [sent]);
    assert.deepEqual(h.panel.state.attachments, [next]);
    assert.equal(h.panel.state.draft, "next message");
  } finally { h.panel.dispose(); finishHistory([]); finishSend(); await sending; }
});
