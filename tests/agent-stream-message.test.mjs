import test from "node:test";
import assert from "node:assert/strict";
import { loadAgentChat } from "./load-agent-chat.mjs";

const row = (id, role, content) => ({ id, role, content, conv_id: "A", created_at: 1 });

test("stream chunks change only the active row and preserve all completed history and metadata", async () => {
  const h = await loadAgentChat();
  try {
    const history = [row(1, "user", "質問 🙂"), row(2, "assistant", "過去の **完全な答え**")];
    h.messagesProbe.set(history);
    h.messagesProbe.append("");
    assert.equal(h.messagesProbe.rows, history);
    h.messagesProbe.append("現在の答え");
    const active = h.messagesProbe.rows.at(-1);
    const metadata = { id: active.id, conv_id: active.conv_id, created_at: active.created_at, role: active.role };
    const chunks = Array.from({ length: 1000 }, (_, i) => `\n${i} 中文・日本語 👩🏽‍💻`);
    for (const chunk of chunks) {
      h.messagesProbe.append(chunk);
      assert.equal(h.messagesProbe.rows.at(-1), active, "stable keyed row throughout the stream");
    }
    assert.equal(active.content, "現在の答え" + chunks.join(""));
    assert.deepEqual({ id: active.id, conv_id: active.conv_id, created_at: active.created_at, role: active.role }, metadata);
    assert.equal(h.messagesProbe.rows.length, 3);
    assert.equal(h.messagesProbe.rows[0], history[0]);
    assert.equal(h.messagesProbe.rows[1], history[1]);
    assert.deepEqual(history, [row(1, "user", "質問 🙂"), row(2, "assistant", "過去の **完全な答え**")]);
    h.messagesProbe.finalize();
    assert.equal(h.messagesProbe.rows.at(-1)._streaming, false);
    assert.equal(h.messagesProbe.rows.at(-1).content, active.content);
    assert.deepEqual(h.calls, [], "message rendering does not invoke a native operation");
  } finally { h.panel.dispose(); }
});

test("a reply quotes the clicked prefix even after later chunks and finalization", async () => {
  const h = await loadAgentChat();
  try {
    h.messagesProbe.append("引用した時点 **🙂**");
    h.messagesProbe.quoteLast();
    const quoted = structuredClone(h.messagesProbe.quote);
    for (let i = 0; i < 100; i++) h.messagesProbe.append(`\n後の出力 ${i}`);
    assert.deepEqual(h.messagesProbe.quote, quoted);
    assert.notEqual(h.messagesProbe.rows.at(-1).content, quoted.content);
    h.messagesProbe.finalize();
    assert.deepEqual(h.messagesProbe.quote, quoted);
    assert.equal(h.messagesProbe.rows.at(-1)._streaming, false);
  } finally { h.panel.dispose(); }
});
