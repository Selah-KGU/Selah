import test from "node:test";
import assert from "node:assert/strict";
import { loadAgentPanel } from "./load-agent-panel.mjs";
import { loadAgentChat } from "./load-agent-chat.mjs";

const settle = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}
for (const [name, load] of [
  ["main chat", loadAgentChat],
  ["sidebar", () => loadAgentPanel({ hash: "#owner=agent-popup&target=agent-popup&kind=agent" })],
]) {
  test(`${name} ignores all old or foreign request events during a new same-conversation turn`, async () => {
    const h = await load();
    const replies = [deferred(), deferred()];
    let sends = 0;
    h.configure((command) => {
      if (command === "agent_active_conversation") return "A";
      if (command === "agent_load_display_messages") return [];
      if (command === "agent_list_conversations") return [{ id: "A", title: "A" }];
      if (command === "agent_send") return replies[sends++].promise;
      if (command === "agent_cancel" || command === "agent_set_active_conversation") return;
      throw new Error(`Unexpected IPC ${command}`);
    });
    try {
      if (h.panel.select) await h.panel.select("A"); else await h.panel.load();
      h.panel.setDraft("old request");
      const first = h.panel.send();
      await settle();
      const firstId = h.calls.find(call => call.command === "agent_send").args.turnId;
      assert.match(firstId, /^[0-9a-f-]{36}$/);
      await h.panel.stop();
      assert.equal(h.calls.find(call => call.command === "agent_cancel").args.turnId, firstId);
      h.panel.setDraft("new request");
      const second = h.panel.send();
      await settle();
      const secondId = h.calls.filter(call => call.command === "agent_send")[1].args.turnId;
      assert.notEqual(secondId, firstId);
      const receive = event => h.listeners.find(listener => listener.name === "agent_stream:A").receive({ payload: event });
      const before = JSON.stringify(h.panel.state);
      const callsBefore = h.calls.length;
      for (const turn_id of [firstId, "another-window-request", undefined]) {
        for (const event of [
          { type: "phase", stage: "planning" },
          { type: "plan", steps: [{ name: "old-tool" }] },
          { type: "tool_call", name: "old-tool" },
          { type: "tool_result", name: "old-tool", preview: "obsolete", ok: false },
          { type: "think", text: "obsolete reasoning" },
          { type: "token", text: "obsolete answer" },
          { type: "error", message: "obsolete error" },
          { type: "done" },
        ]) receive({ ...event, turn_id });
      }
      await settle();
      assert.equal(JSON.stringify(h.panel.state), before);
      assert.equal(h.calls.length, callsBefore);
      receive({ type: "token", text: "current answer", turn_id: secondId });
      receive({ type: "error", message: "current error", turn_id: secondId });
      assert.equal(h.panel.state.sending, false);
      if (name === "sidebar") assert.equal(h.panel.state.error, "current error");
      else assert.match(h.panel.state.messages.at(-1).content, /current error/);
      const finished = JSON.stringify(h.panel.state);
      receive({ type: "token", text: "late current token", turn_id: secondId });
      receive({ type: "error", message: "duplicate error", turn_id: secondId });
      assert.equal(JSON.stringify(h.panel.state), finished);
      replies[0].resolve();
      await first;
      replies[1].resolve();
      await second;
    } finally {
      h.panel.dispose();
      replies.forEach(reply => reply.resolve());
    }
  });
}

test("sidebar with page context sends and cancels the same request identity", async () => {
  const h = await loadAgentPanel({ hash: "#target=course-view&title=授業&kind=luna" });
  const reply = deferred();
  h.configure(command => {
    if (command === "agent_active_conversation") return "A";
    if (command === "agent_load_display_messages") return [];
    if (command === "agent_list_conversations") return [{ id: "A", title: "A" }];
    if (command === "document_tabs_list") return [{ id: "1", target: "course-view", title: "授業", type: "luna", active: true }];
    if (command === "agent_send_with_context") return reply.promise;
    if (command === "agent_cancel") return;
    throw new Error(`Unexpected IPC ${command}`);
  });
  try {
    await h.panel.load();
    h.panel.setDraft("授業を確認");
    const send = h.panel.send();
    await settle();
    const args = h.calls.find(call => call.command === "agent_send_with_context").args;
    assert.equal(args.browserTarget, "course-view");
    assert.match(args.turnId, /^[0-9a-f-]{36}$/);
    await h.panel.stop();
    assert.equal(h.calls.find(call => call.command === "agent_cancel").args.turnId, args.turnId);
    reply.resolve();
    await send;
  } finally { h.panel.dispose(); reply.resolve(); }
});
