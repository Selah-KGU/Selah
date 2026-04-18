<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import Icon from "../Icon.svelte";
  import selahLogoUrl from "../../assets/logo.png";
  import {
    agentListConversations,
    agentCreateConversation,
    agentLoadMessages,
    agentSend,
    agentCancel,
    agentDeleteConversation,
    agentRenameConversation,
    isAiReady,
    type AgentConversationSummary,
    type AgentMessage,
    type AgentStreamEvent,
  } from "../api";
  import { agentConversations, agentActiveConvId } from "../stores";
  import { invoke } from "@tauri-apps/api/core";
  import type { AiConfig } from "../stores";

  type UIMessage = AgentMessage & { _streaming?: boolean };

  let conversations = $state<AgentConversationSummary[]>([]);
  let activeConvId = $state<string | null>(null);
  let messages = $state<UIMessage[]>([]);
  let inputText = $state("");

  let sending = $state(false);
  let toolChips = $state<{ id: number; name: string; state: "running" | "ok" | "err"; preview?: string }[]>([]);
  let chipCounter = 0;
  let unlisten: UnlistenFn | null = null;
  let msgListEl: HTMLElement | null = null;
  let thinkTraceEl: HTMLElement | null = null;
  let autoFollow = $state(true);
  let aiCfg = $state<AiConfig | null>(null);
  let historyOpen = $state(false);
  let headerMenuEl: HTMLElement | null = null;
  let currentPhase = $state<"idle" | "planning" | "answering">("idle");
  let thinkBuffer = $state("");
  const activeConv = $derived(conversations.find((c) => c.id === activeConvId) ?? null);
  const assistantIsStreaming = $derived.by(() => {
    const last = messages[messages.length - 1];
    return !!last && last.role === "assistant" && last._streaming === true && !!last.content;
  });
  const showStatus = $derived(sending && !assistantIsStreaming);

  marked.setOptions({ breaks: true, gfm: true });

  function render(md: string): string {
    const raw = marked.parse(md) as string;
    return DOMPurify.sanitize(raw);
  }

  async function refreshConfig() {
    try {
      aiCfg = await invoke<AiConfig>("get_ai_config");
    } catch {
      aiCfg = null;
    }
  }

  async function refreshConversations() {
    try {
      conversations = await agentListConversations();
      agentConversations.set(conversations);
    } catch (e) {
      console.warn("agent list failed", e);
    }
  }

  async function selectConversation(id: string) {
    historyOpen = false;
    if (activeConvId === id) return;
    activeConvId = id;
    agentActiveConvId.set(id);
    toolChips = [];
    thinkBuffer = "";
    currentPhase = "idle";
    try {
      const rows = await agentLoadMessages(id);
      messages = rows;
    } catch (e) {
      console.warn("load messages", e);
      messages = [];
    }
    await tick();
    scrollToBottom(true);
    await rebindListener();
  }

  async function newConversation() {
    historyOpen = false;
    try {
      const id = await agentCreateConversation();
      await refreshConversations();
      await selectConversation(id);
    } catch (e) {
      console.warn("create conv", e);
    }
  }

  async function deleteConv(id: string, ev: MouseEvent) {
    ev.stopPropagation();
    if (!confirm("この会話を削除しますか？")) return;
    try {
      await agentDeleteConversation(id);
      if (activeConvId === id) {
        activeConvId = null;
        messages = [];
      }
      await refreshConversations();
    } catch (e) {
      console.warn("delete conv", e);
    }
  }

  let editingTitle = $state(false);
  let titleDraft = $state("");
  let titleInputEl: HTMLInputElement | null = null;

  async function startRename() {
    if (!activeConv) return;
    historyOpen = false;
    titleDraft = activeConv.title || "";
    editingTitle = true;
    await tick();
    titleInputEl?.focus();
    titleInputEl?.select();
  }

  async function commitRename() {
    if (!editingTitle) return;
    const conv = activeConv;
    editingTitle = false;
    if (!conv) return;
    const trimmed = titleDraft.trim();
    if (!trimmed || trimmed === conv.title) return;
    try {
      await agentRenameConversation(conv.id, trimmed);
      await refreshConversations();
    } catch (e) {
      console.warn("rename", e);
    }
  }

  function cancelRename() {
    editingTitle = false;
    titleDraft = "";
  }

  function onTitleKey(e: KeyboardEvent) {
    if (e.key === "Enter") { e.preventDefault(); commitRename(); }
    else if (e.key === "Escape") { e.preventDefault(); cancelRename(); }
  }

  async function rebindListener() {
    if (unlisten) { unlisten(); unlisten = null; }
    if (!activeConvId) return;
    const id = activeConvId;
    unlisten = await listen<AgentStreamEvent>(`agent_stream:${id}`, (ev) => {
      if (activeConvId !== id) return;
      handleStream(ev.payload);
    });
  }

  function handleStream(ev: AgentStreamEvent) {
    switch (ev.type) {
      case "phase":
        currentPhase = ev.stage;
        scheduleScroll();
        break;
      case "tool_call":
        chipCounter++;
        toolChips = [...toolChips, { id: chipCounter, name: ev.name, state: "running" }];
        scheduleScroll();
        break;
      case "tool_result": {
        const last = [...toolChips].reverse().find((c) => c.name === ev.name && c.state === "running");
        if (last) {
          toolChips = toolChips.map((c) =>
            c.id === last.id ? { ...c, state: ev.ok ? "ok" : "err", preview: ev.preview } : c,
          );
        }
        break;
      }
      case "think":
        thinkBuffer += ev.text;
        scheduleScroll();
        tick().then(() => {
          if (thinkTraceEl) thinkTraceEl.scrollTop = thinkTraceEl.scrollHeight;
        });
        break;
      case "token": {
        const last = messages[messages.length - 1];
        if (last && last.role === "assistant" && last._streaming) {
          messages[messages.length - 1] = { ...last, content: last.content + ev.text };
        } else {
          messages = [
            ...messages,
            {
              id: -Date.now(),
              conv_id: activeConvId ?? "",
              role: "assistant",
              content: ev.text,
              created_at: Math.floor(Date.now() / 1000),
              _streaming: true,
            },
          ];
        }
        scheduleScroll();
        break;
      }
      case "done":
        finalizeTurn();
        break;
      case "error":
        finalizeTurn();
        messages = [
          ...messages,
          {
            id: -Date.now(),
            conv_id: activeConvId ?? "",
            role: "assistant",
            content: `……エラーが出たみたい。\n\n> ${ev.message}`,
            created_at: Math.floor(Date.now() / 1000),
          },
        ];
        scheduleScroll();
        break;
    }
  }

  function finalizeTurn() {
    sending = false;
    currentPhase = "idle";
    toolChips = [];
    thinkBuffer = "";
    messages = messages.map((m) => (m._streaming ? { ...m, _streaming: false } : m));
    refreshConversations();
  }

  async function send() {
    let text = inputText.trim();
    if (!text) return;
    if (sending) return;
    if (!activeConvId) {
      await newConversation();
      if (!activeConvId) return;
    }
    if (!await isAiReady()) {
      alert("Agent は現在利用できません。AI設定（ローカルモデルまたはAPIキー）を確認してください。");
      return;
    }
    if (quotedMessage) {
      const qText = quotedMessage.role === "assistant" ? stripHtml(render(quotedMessage.content)) : quotedMessage.content;
      const lines = qText.trim().split("\n").filter(Boolean);
      const short = lines.length > 3 ? lines.slice(0, 3).join("\n") + "..." : lines.join("\n");
      text = `「${short}」について：\n${text}`;
      quotedMessage = null;
    }

    const now = Math.floor(Date.now() / 1000);
    messages = [
      ...messages,
      {
        id: -now,
        conv_id: activeConvId,
        role: "user",
        content: text,
        created_at: now,
      },
    ];
    inputText = "";
    sending = true;
    toolChips = [];
    thinkBuffer = "";
    currentPhase = "planning";
    autoFollow = true;
    scheduleScroll();

    try {
      await agentSend(activeConvId, text);
    } catch (e) {
      console.warn("agent send", e);
      sending = false;
      messages = [
        ...messages,
        {
          id: -Date.now(),
          conv_id: activeConvId,
          role: "assistant",
          content: `……送信に失敗したみたい。\n\n> ${e}`,
          created_at: Math.floor(Date.now() / 1000),
        },
      ];
    }
  }

  async function cancel() {
    if (!activeConvId || !sending) return;
    try {
      await agentCancel(activeConvId);
    } catch (e) {
      console.warn("cancel", e);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  }



  // ── Auto-scroll ──

  let scrollRafScheduled = false;
  function scheduleScroll() {
    if (!autoFollow) return;
    if (scrollRafScheduled) return;
    scrollRafScheduled = true;
    tick().then(() => {
      requestAnimationFrame(() => {
        scrollRafScheduled = false;
        scrollToBottom(false);
      });
    });
  }

  function scrollToBottom(force: boolean) {
    if (!msgListEl) return;
    if (!force && !autoFollow) return;
    msgListEl.scrollTop = msgListEl.scrollHeight;
  }

  function onScroll() {
    if (!msgListEl) return;
    const near = msgListEl.scrollHeight - msgListEl.scrollTop - msgListEl.clientHeight < 80;
    autoFollow = near;
  }

  // ── History dropdown ──

  function onDocClick(e: MouseEvent) {
    if (!historyOpen) return;
    if (headerMenuEl && e.target instanceof Node && !headerMenuEl.contains(e.target)) {
      historyOpen = false;
    }
  }

  // ── Lifecycle ──

  onMount(async () => {
    document.addEventListener("mousedown", onDocClick);
    await refreshConfig();
    await refreshConversations();
    if (!activeConvId && conversations.length > 0) {
      await selectConversation(conversations[0].id);
    }
  });

  onDestroy(() => {
    document.removeEventListener("mousedown", onDocClick);
    if (unlisten) unlisten();
    if (activeConvId && sending) agentCancel(activeConvId).catch(() => {});
  });

  function fmtDate(ts: number): string {
    const d = new Date(ts * 1000);
    const today = new Date();
    if (d.toDateString() === today.toDateString()) {
      return d.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
    }
    return d.toLocaleDateString("ja-JP", { month: "numeric", day: "numeric" });
  }

  function toolLabel(n: string): string {
    const map: Record<string, string> = {
      list_today_classes: "今日の授業を確認中…",
      list_week_classes: "週の時間割を確認中…",
      search_courses: "科目候補を探しています…",
      get_course_context: "科目の文脈を整理しています…",
      list_luna_todos: "提出物を確認中…",
      list_recent_notifications: "お知らせを確認中…",
      search_notifications: "お知らせを検索中…",
      get_course_detail: "科目の詳細を確認中…",
      list_recent_mail: "メールを確認中…",
      read_mail: "メールを読んでいます…",
      get_student_profile: "学生情報を確認中…",
      get_mail_profile: "メールアカウントを確認中…",
      list_syllabus_favorites: "お気に入りシラバスを確認中…",
      get_grades: "成績を確認中…",
      get_cancellations: "休講情報を確認中…",
      get_makeup_classes: "補講情報を確認中…",
      get_room_changes: "教室変更を確認中…",
      get_registration: "履修情報を確認中…",
      get_exam_timetable: "試験時間割を確認中…",
      get_weather: "天気を確認中…",
      get_weekly_summary: "週間サマリーを確認中…",
      get_upcoming_deadlines: "締め切りを確認中…",
      get_todo_guide: "タスクガイドを作成中…",
      get_luna_activity_detail: "課題の詳細を確認中…",
      refresh_data: "データを更新中…",
    };
    return map[n] ?? n;
  }

  let copiedId = $state<number | null>(null);
  let quotedMessage = $state<UIMessage | null>(null);

  function stripHtml(html: string): string {
    const tmp = document.createElement("div");
    tmp.innerHTML = html;
    return tmp.textContent ?? tmp.innerText ?? "";
  }

  async function copyMessage(m: UIMessage) {
    const text = m.role === "assistant" ? stripHtml(render(m.content)) : m.content;
    await navigator.clipboard.writeText(text);
    copiedId = m.id;
    setTimeout(() => { if (copiedId === m.id) copiedId = null; }, 1500);
  }

  function quoteReply(m: UIMessage) {
    quotedMessage = m;
    tick().then(() => {
      const ta = document.querySelector<HTMLTextAreaElement>(".composer-row textarea");
      if (ta) ta.focus();
    });
  }

  function dismissQuote() {
    quotedMessage = null;
  }
</script>

<div class="agent-root">
  <!-- Floating top island -->
  <header class="top-island" bind:this={headerMenuEl}>
    <div class="island-inner">
      {#if editingTitle}
        <input
          class="conv-pill-input"
          bind:value={titleDraft}
          bind:this={titleInputEl}
          onkeydown={onTitleKey}
          onblur={commitRename}
          placeholder="タイトル"
          maxlength="80"
        />
      {:else}
        <button
          class="conv-pill"
          onclick={() => (historyOpen = !historyOpen)}
          class:open={historyOpen}
          title="履歴を開く"
        >
          <span class="pill-title">{activeConv?.title || "新しい会話"}</span>
          <span class="pill-caret" class:flip={historyOpen}>
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
          </span>
        </button>
      {/if}

      <div class="island-actions">
        {#if activeConv && !editingTitle}
          <button class="island-icon-btn" onclick={startRename} title="タイトルを変更">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 20h9"/>
              <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/>
            </svg>
          </button>
        {/if}
        <button class="island-icon-btn" onclick={newConversation} title="新しい会話">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <line x1="12" y1="5" x2="12" y2="19"/>
            <line x1="5" y1="12" x2="19" y2="12"/>
          </svg>
        </button>
      </div>
    </div>

    {#if historyOpen}
      <div class="history-dropdown" role="menu">
        {#if conversations.length === 0}
          <div class="hd-empty">……まだ何も。</div>
        {:else}
          {#each conversations as c (c.id)}
            <div
              class="hd-item"
              class:active={activeConvId === c.id}
              role="menuitem"
              tabindex="0"
              onclick={() => selectConversation(c.id)}
              onkeydown={(e) => { if (e.key === "Enter") selectConversation(c.id); }}
            >
              <div class="hd-title">{c.title || "無題"}</div>
              <div class="hd-meta">
                <span class="hd-date">{fmtDate(c.updated_at)}</span>
                <button
                  class="hd-del"
                  onclick={(e) => deleteConv(c.id, e)}
                  aria-label="削除"
                  title="削除"
                >
                  <Icon name="trash" size={12} />
                </button>
              </div>
            </div>
          {/each}
        {/if}
      </div>
    {/if}
  </header>

  <!-- Full-height message area -->
  <section class="chat-panel">
    <div
      class="msg-list"
      bind:this={msgListEl}
      onscroll={onScroll}
      role="log"
      aria-live="polite"
    >
      <div class="top-spacer"></div>

      {#if !activeConvId}
        <div class="empty-hero">
          <img src={selahLogoUrl} alt="Selah" class="hero-logo" />
          <p class="hero-text">……話しかけてくれたら、そこにいる。</p>
          <button class="primary-btn" onclick={newConversation}>新しい会話を始める</button>
        </div>
      {:else if messages.length === 0}
        <div class="empty-hero subtle">
          <img src={selahLogoUrl} alt="Selah" class="hero-logo dim" />
          <p class="hero-text">……なにか書いてみて。</p>
        </div>
      {:else}
        {#each messages as m (m.id)}
          {#if m.role === "user"}
            <div class="row user">
              <div class="bubble user-bubble">
                {#if m.content}
                  <div class="text">{m.content}</div>
                {/if}
                <div class="msg-actions">
                  <button class="msg-act-btn" title="コピー" onclick={() => copyMessage(m)}>
                    {#if copiedId === m.id}
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                      <span>コピー済</span>
                    {:else}
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><rect x="4" y="4" width="11" height="11" rx="2"/></svg>
                      <span>コピー</span>
                    {/if}
                  </button>
                </div>
              </div>
            </div>
          {:else if m.role === "assistant"}
            <div class="row assistant">
              <img src={selahLogoUrl} alt="" class="avatar" />
              <div class="bubble assistant-bubble">
                <div class="md">{@html render(m.content)}</div>
                <div class="msg-actions">
                  <button class="msg-act-btn" title="コピー" onclick={() => copyMessage(m)}>
                    {#if copiedId === m.id}
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                      <span>コピー済</span>
                    {:else}
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><rect x="4" y="4" width="11" height="11" rx="2"/></svg>
                      <span>コピー</span>
                    {/if}
                  </button>
                  <button class="msg-act-btn" title="引用して返信" onclick={() => quoteReply(m)}>
                    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 8L4 12l6 4"/><path d="M4 12h10a6 6 0 0 1 6 6"/></svg>
                    <span>返信</span>
                  </button>
                </div>
              </div>
            </div>
          {/if}
        {/each}
      {/if}

      {#if showStatus}
        <div class="row assistant status-row">
          <img src={selahLogoUrl} alt="" class="avatar pulse" />
          <div class="status-area">
            {#if toolChips.length}
              <div class="tool-steps">
                {#each toolChips as chip (chip.id)}
                  <div class="tool-step" class:ok={chip.state === "ok"} class:err={chip.state === "err"}>
                    {#if chip.state === "running"}
                      <span class="spin"></span>
                    {:else if chip.state === "ok"}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                    {:else}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                    {/if}
                    <span>{toolLabel(chip.name)}</span>
                  </div>
                {/each}
              </div>
            {:else}
              <div class="wait-dots" aria-label="考え中"><span></span><span></span><span></span></div>
            {/if}
            {#if thinkBuffer}
              <p class="think-trace" bind:this={thinkTraceEl}>{thinkBuffer}</p>
            {/if}
          </div>
        </div>
      {/if}

      <div class="bottom-spacer"></div>
    </div>

    <!-- Floating bottom composer + action capsule -->
    <div class="composer-bottom">
      {#if quotedMessage}
        <div class="quote-bar">
          <span class="quote-label">返信：</span>
          <span class="quote-text">{quotedMessage.role === "assistant" ? stripHtml(render(quotedMessage.content)) : quotedMessage.content}</span>
          <button class="quote-dismiss" onclick={dismissQuote} title="キャンセル">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      {/if}
      <div class="send-row">
        <div class="composer-island">
          <div class="composer-row">
            <textarea
              bind:value={inputText}
              onkeydown={onKeydown}
              placeholder={sending ? "返事を書いている途中……" : "なにか書いてみて。"}
              rows="1"
              disabled={sending}
            ></textarea>
          </div>
        </div>
        {#if sending}
          <button class="action-capsule stop" onclick={cancel} title="停止">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>
            <span>停止</span>
          </button>
        {:else}
          <button class="action-capsule" onclick={send} disabled={!inputText.trim()}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
            <span>送る</span>
          </button>
        {/if}
      </div>
    </div>
  </section>
</div>

<style>
  /* ═══════════════════════════════════════════════
     Agent Chat — Floating Island Design
     ═══════════════════════════════════════════════ */

  .agent-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    width: 100%;
    position: relative;
  }

  /* ── Floating Top Island ── */
  .top-island {
    position: absolute;
    top: 10px;
    left: 14px;
    z-index: 30;
    max-width: min(520px, calc(100% - 32px));
    width: auto;
  }

  .island-inner {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 4px 4px 6px;
    border-radius: 18px;
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    box-shadow: var(--shadow-glass), 0 4px 20px rgba(0, 0, 0, 0.06);
  }

  .conv-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border: none;
    border-radius: 14px;
    background: transparent;
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
    transition: background 0.15s;
    max-width: 300px;
    min-width: 0;
  }
  .conv-pill:hover, .conv-pill.open {
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
  }
  .pill-title {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
    letter-spacing: -0.01em;
  }
  .pill-caret {
    display: inline-flex;
    align-items: center;
    color: var(--text-tertiary);
    transition: transform 0.2s ease;
    flex-shrink: 0;
  }
  .pill-caret.flip { transform: rotate(180deg); }

  .conv-pill-input {
    display: inline-flex;
    align-items: center;
    padding: 5px 10px;
    border: 0.5px solid color-mix(in srgb, var(--accent) 45%, var(--glass-border));
    border-radius: 14px;
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-primary));
    color: var(--text-primary);
    font-size: 13px;
    font-weight: 500;
    letter-spacing: -0.01em;
    max-width: 300px;
    min-width: 160px;
    outline: none;
    font-family: inherit;
  }
  .conv-pill-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 22%, transparent);
  }

  .island-actions {
    display: flex;
    align-items: center;
    gap: 1px;
    margin-left: 2px;
  }

  .island-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
    padding: 0;
  }
  .island-icon-btn:hover {
    background: color-mix(in srgb, var(--text-primary) 8%, transparent);
    color: var(--text-primary);
  }

  /* ── History Dropdown ── */
  .history-dropdown {
    position: absolute;
    top: calc(100% + 8px);
    left: 0;
    width: 340px;
    max-height: 400px;
    overflow-y: auto;
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    border-radius: 16px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.12), 0 0 0.5px rgba(0, 0, 0, 0.08);
    padding: 6px;
    z-index: 20;
  }
  .hd-empty {
    padding: 24px 18px;
    text-align: center;
    font-size: 12px;
    color: var(--text-tertiary);
  }
  .hd-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 9px 10px;
    border-radius: 10px;
    cursor: pointer;
    transition: background 0.12s;
    outline: none;
  }
  .hd-item:hover, .hd-item:focus {
    background: color-mix(in srgb, var(--text-primary) 5%, transparent);
  }
  .hd-item.active {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .hd-title {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 450;
  }
  .hd-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .hd-date {
    font-size: 11px;
    color: var(--text-tertiary);
  }
  .hd-del {
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--text-tertiary);
    opacity: 0;
    padding: 4px;
    border-radius: 6px;
    transition: opacity 0.15s, color 0.15s, background 0.15s;
    display: inline-flex;
    align-items: center;
  }
  .hd-item:hover .hd-del, .hd-item:focus .hd-del { opacity: 1; }
  .hd-del:hover { color: #d64545; background: color-mix(in srgb, #d64545 12%, transparent); }

  /* ── Chat Panel ── */
  .chat-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    position: relative;
  }

  .msg-list {
    flex: 1;
    overflow-y: auto;
    padding: 0 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .top-spacer { flex-shrink: 0; height: 60px; }
  .bottom-spacer { flex-shrink: 0; height: 88px; }

  .row {
    display: flex;
    max-width: 100%;
    gap: 10px;
    align-items: flex-end;
    animation: msg-enter 0.25s ease-out;
  }
  .row.user { justify-content: flex-end; }
  .row.assistant { justify-content: flex-start; align-items: flex-start; }

  @keyframes msg-enter {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .avatar {
    width: 26px;
    height: 26px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
    margin-top: 2px;
  }
  .avatar.dim { opacity: 0.6; }

  .bubble {
    position: relative;
    max-width: 76%;
    padding: 10px 14px;
    border-radius: 16px;
    font-size: 13.5px;
    line-height: 1.65;
    word-wrap: break-word;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .user-bubble {
    background: var(--accent);
    color: white;
    border-bottom-right-radius: 6px;
    box-shadow: 0 1px 4px rgba(0, 40, 85, 0.12);
  }
  .assistant-bubble {
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: blur(20px) var(--glass-saturate);
    -webkit-backdrop-filter: blur(20px) var(--glass-saturate);
    color: var(--text-primary);
    border: 0.5px solid var(--glass-border);
    border-top-left-radius: 6px;
    box-shadow: var(--shadow-sm);
  }
  .user-bubble .text { white-space: pre-wrap; }
  .imgs { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 6px; }
  .imgs img {
    max-width: 180px;
    max-height: 180px;
    border-radius: 10px;
    display: block;
  }

  /* ── Markdown ── */
  .md :global(p) { margin: 0 0 8px; }
  .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(ul), .md :global(ol) { margin: 0 0 8px; padding-left: 20px; }
  .md :global(code) {
    background: color-mix(in srgb, var(--text-primary) 7%, transparent);
    padding: 2px 5px;
    border-radius: 5px;
    font-size: 0.88em;
  }
  .md :global(pre) {
    background: color-mix(in srgb, var(--text-primary) 5%, transparent);
    padding: 10px 12px;
    border-radius: 10px;
    overflow-x: auto;
    font-size: 12.5px;
  }
  .md :global(pre code) { background: transparent; padding: 0; }
  .md :global(blockquote) {
    border-left: 2px solid color-mix(in srgb, var(--accent) 30%, var(--glass-border));
    margin: 0 0 8px;
    padding-left: 10px;
    color: var(--text-secondary);
  }
  .md :global(a) { color: var(--accent); text-decoration: none; }
  .md :global(a:hover) { text-decoration: underline; }

  .msg-actions {
    position: absolute;
    bottom: 7px;
    right: 8px;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px 4px;
    border-radius: 999px;
    background: transparent;
    backdrop-filter: blur(12px) saturate(1.6);
    -webkit-backdrop-filter: blur(12px) saturate(1.6);
    border: 0.5px solid rgba(255, 255, 255, 0.25);
    box-shadow: 0 1px 6px rgba(0,0,0,0.12);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s;
    z-index: 2;
  }
  .assistant-bubble .msg-actions {
    border-color: rgba(0, 0, 0, 0.08);
    box-shadow: 0 1px 6px rgba(0,0,0,0.08);
  }
  .bubble:hover .msg-actions,
  .bubble:focus-within .msg-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .msg-act-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 4px;
    background: none;
    border: none;
    cursor: pointer;
    border-radius: 999px;
    font-size: 11px;
    font-family: inherit;
    letter-spacing: 0.01em;
    transition: background 0.1s;
  }
  .user-bubble .msg-act-btn { color: rgba(255,255,255,0.85); }
  .assistant-bubble .msg-act-btn { color: rgba(0,0,0,0.45); }
  .user-bubble .msg-act-btn:hover { background: rgba(255,255,255,0.2); color: #fff; }
  .assistant-bubble .msg-act-btn:hover { background: rgba(0,0,0,0.07); color: rgba(0,0,0,0.75); }

  /* ── Spinner ── */
  .spin {
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--glass-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: agent-spin 0.8s linear infinite;
  }
  @keyframes agent-spin { to { transform: rotate(360deg); } }

  /* ── Status Area ── */
  .status-row .status-area {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    max-width: 76%;
    padding: 12px 16px;
    border-radius: 16px;
    border-top-left-radius: 6px;
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: blur(20px) var(--glass-saturate);
    -webkit-backdrop-filter: blur(20px) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    box-shadow: var(--shadow-sm);
  }

  .tool-steps {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .tool-step {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text-secondary);
    line-height: 1.4;
  }
  .tool-step.ok { color: var(--green); }
  .tool-step.err { color: var(--red); }

  .wait-dots {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 0;
  }
  .wait-dots span {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent);
    animation: dot-wave 1.4s ease-in-out infinite;
  }
  .wait-dots span:nth-child(1) { animation-delay: 0s; }
  .wait-dots span:nth-child(2) { animation-delay: 0.2s; }
  .wait-dots span:nth-child(3) { animation-delay: 0.4s; }
  @keyframes dot-wave {
    0%, 60%, 100% { transform: translateY(0); opacity: 0.35; }
    30% { transform: translateY(-4px); opacity: 1; }
  }

  .think-trace {
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-tertiary);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    max-height: 100px;
    overflow-y: auto;
  }

  .avatar.pulse {
    animation: agent-avatar-pulse 2s ease-in-out infinite;
  }
  @keyframes agent-avatar-pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 30%, transparent); }
    50% { box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 0%, transparent); }
  }

  /* ── Empty State ── */
  .empty-hero {
    margin: auto;
    text-align: center;
    color: var(--text-secondary);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    padding: 20px;
  }
  .empty-hero.subtle { opacity: 0.7; }
  .hero-logo {
    width: 72px;
    height: 72px;
    border-radius: 50%;
    object-fit: cover;
    opacity: 0.85;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
  }
  .hero-logo.dim { width: 52px; height: 52px; opacity: 0.45; }
  .hero-text {
    font-size: 14px;
    color: var(--text-tertiary);
    margin: 0;
    letter-spacing: -0.01em;
  }
  .primary-btn {
    padding: 9px 20px;
    border-radius: 12px;
    background: var(--accent);
    color: white;
    border: none;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: opacity 0.15s, transform 0.1s;
    box-shadow: 0 2px 8px rgba(0, 40, 85, 0.15);
  }
  .primary-btn:hover { opacity: 0.9; }
  .primary-btn:active { transform: scale(0.97); }

  /* ═══ Floating Composer Area ═══ */
  .composer-bottom {
    position: absolute;
    bottom: 12px;
    left: 50%;
    transform: translateX(-50%);
    width: min(640px, calc(100% - 28px));
    z-index: 30;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }

  .quote-bar {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 12px;
    background: var(--glass-bg, rgba(255, 255, 255, 0.82));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.07);
    animation: msg-enter 0.18s ease-out;
  }
  .quote-label {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--accent);
    flex-shrink: 0;
  }
  .quote-text {
    font-size: 12px;
    color: var(--text-secondary);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    word-break: break-word;
  }
  .quote-dismiss {
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
  }
  .quote-dismiss:hover { color: var(--text-secondary); }

  .composer-row-actions {
    display: flex;
    align-items: stretch;
    gap: 8px;
  }

  .send-row {
    display: flex;
    align-items: stretch;
    gap: 8px;
  }

  .composer-island {
    flex: 1;
    min-width: 0;
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    border-radius: 18px;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.08), 0 0 0.5px rgba(0, 0, 0, 0.06), var(--glass-highlight);
    padding: 6px 12px;
    display: flex;
    align-items: center;
    transition: box-shadow 0.2s;
  }
  .composer-island:focus-within {
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.08), 0 0 0 2px color-mix(in srgb, var(--accent) 25%, transparent);
  }

  .composer-row {
    display: flex;
    flex: 1;
    align-items: center;
  }

  textarea {
    flex: 1;
    min-height: 22px;
    max-height: 180px;
    resize: none;
    border: none;
    background: transparent;
    color: var(--text-primary);
    padding: 4px 4px;
    font-size: 13.5px;
    font-family: inherit;
    line-height: 1.5;
    outline: none;
  }
  textarea::placeholder { color: var(--text-tertiary); }

  /* ── Action Capsule (send / stop) ── */
  .action-capsule {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 0 16px;
    border-radius: 18px;
    background: var(--glass-bg, rgba(255, 255, 255, 0.5));
    backdrop-filter: var(--glass-blur) var(--glass-saturate);
    -webkit-backdrop-filter: var(--glass-blur) var(--glass-saturate);
    border: 0.5px solid var(--glass-border);
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.08), 0 0 0.5px rgba(0, 0, 0, 0.06), var(--glass-highlight);
    color: var(--accent);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    flex-shrink: 0;
    white-space: nowrap;
    transition: background 0.15s, transform 0.1s, color 0.15s;
  }
  .action-capsule:hover {
    background: color-mix(in srgb, var(--accent) 10%, var(--glass-bg, rgba(255, 255, 255, 0.5)));
  }
  .action-capsule:active { transform: scale(0.95); }
  .action-capsule.stop { color: var(--red); }
  .action-capsule.stop:hover {
    background: color-mix(in srgb, var(--red) 10%, var(--glass-bg, rgba(255, 255, 255, 0.5)));
  }
  .action-capsule:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .action-capsule:disabled:active { transform: none; }
</style>
