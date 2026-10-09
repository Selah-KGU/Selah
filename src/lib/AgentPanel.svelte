<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { AGENT_ATTACHMENT_ACCEPT, isAgentAttachmentFile, isAgentImagePart, appendAgentAttachments } from "./agentAttachments";
  import AgentDocumentPreview from "./AgentDocumentPreview.svelte";
  import AgentAttachmentStatus from "./AgentAttachmentStatus.svelte";
  import { listen, type Event as NativeEvent } from "@tauri-apps/api/event";
  import { ResourceScope } from "./resourceScope";
  import { AgentConversationView } from "./agentConversationView";
  import { appendSttFinal, mergeSttText } from "./speechDraft";
  import { AgentSpeechInput, type AgentSpeechEvent } from "./agentSpeechInput";
  import DOMPurify from "dompurify";
  import { createMarkdownRenderer } from "./markdownRenderer";
  import { TextStreamBuffer } from "./textStreamBuffer";
  import { onDestroy, onMount, tick } from "svelte";
  import selahLogoUrl from "../assets/logo.png";
  import AgentThinkingStatus from "./AgentThinkingStatus.svelte";
  import AgentIslandIconButton from "./AgentIslandIconButton.svelte";
  import { applyAuxiliaryTheme, syncAuxiliaryTheme } from "./auxiliarySurfaceTheme";
  import Icon, { type IconName } from "./Icon.svelte";
  import type { AgentAttachment, AgentImagePart, AgentMessage, AgentStreamEvent } from "./api";
  import "./agent-panel.css";

  interface DocumentTab {
    id: string;
    target: string;
    title: string;
    type: string;
    active: boolean;
  }

  interface DocumentTabsChanged {
    owner: string;
    tabs: DocumentTab[];
  }

  interface AgentConversationSummary {
    id: string;
    title: string;
  }

  type ToolChip = { id: number; name: string; detail?: string | null; state: "pending" | "running" | "ok" | "err" };
  type ActionMode = "send" | "mic" | "stop";

  function readParam(name: string): string {
    const search = new URLSearchParams(window.location.search);
    const fromSearch = search.get(name);
    if (fromSearch) return fromSearch;
    const rawHash = window.location.hash.startsWith("#") ? window.location.hash.slice(1) : window.location.hash;
    return new URLSearchParams(rawHash).get(name) || "";
  }

  const owner = readParam("owner") || "document-tabs";
  const initialTarget = readParam("target");
  const initialTitle = readParam("title") || "エージェント";
  const initialKind = readParam("kind") || "agent";
  const standalone = owner === "agent-popup";
  const resources = new ResourceScope();

  function listenPanel<T>(name: string, handler: (event: NativeEvent<T>) => void) {
    return resources.acquire(() => listen<T>(name, resources.guard(handler)));
  }

  let pageTarget = $state(initialTarget);
  let pageTitle = $state(initialTitle);
  let pageKind = $state(initialKind);
  let convId = $state("");
  let convTitle = $state("新しい会話");
  let conversations = $state<AgentConversationSummary[]>([]);
  let conversationMenuOpen = $state(false);
  let editingTitle = $state(false);
  let titleDraft = $state("");
  let titleInputEl = $state<HTMLInputElement | null>(null);
  let messages = $state<AgentMessage[]>([]);
  let draft = $state("");
  let attachments = $state<AgentAttachment[]>([]);
  let attachmentReads = $state(0);
  let attachmentError = $state("");
  let fileInput = $state<HTMLInputElement | null>(null);
  let sending = $state(false);
  let preparing = $state(false);
  let preparationVersion = 0;
  let turnVersion = 0;
  let activeRequestId: string | null = null;
  let error = $state("");
  let streamText = $state("");
  let toolChips = $state<ToolChip[]>([]);
  let sttListening = $state(false);
  let sttBaseText = $state("");
  let sttCommittedText = $state("");
  let sttPartialText = $state("");
  let sttStopRequested = $state(false);
  let sttStarting = $state(false);
  const speechInput = new AgentSpeechInput(resources, (active) => { sttListening = active; });
  let messagesEl = $state<HTMLElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let resizeDrag = $state<{ pointerId: number; startScreenX: number; startWidth: number } | null>(null);
  let resizeQueuedWidth: number | null = null;
  let resizeInFlight = false;
  let composing = false;
  let suppressEnterUntil = 0;
  let contextSequence = 0;
  let conversationSelectionVersion = 0;
  let conversationLoading = false;
  let conversationLoad: Promise<boolean> | null = null;
  let conversationReady = false;
  let chipCounter = 0;
  let contextReadVersion = 0;
  let contextRead: Promise<void> | null = null;
  let titleSequence = 0;
  let scrollFrame: number | null = null;

  const actionMode = $derived<ActionMode>(
    sending || preparing ? "stop" : sttListening || (!draft.trim() && attachments.length === 0) ? "mic" : "send"
  );
  const hasPageContext = $derived(pageKind !== "agent" && !!pageTarget && pageTarget !== owner);
  const kindLabel = $derived(
    pageKind === "reader" ? "リーダー"
      : pageKind === "browser" ? "ブラウザ"
      : pageKind === "kwic" ? "KWIC"
      : pageKind === "kgc" ? "KGC"
      : pageKind === "agent" ? "エージェント"
      : "詳細"
  );
  const kindIcon = $derived<IconName>(
    pageKind === "reader" ? "doc"
      : pageKind === "browser" ? "globe"
      : pageKind === "kwic" ? "building.2"
      : pageKind === "kgc" ? "book"
      : pageKind === "agent" ? "copilot"
      : "doc"
  );
  const renderCache = createMarkdownRenderer((html) => DOMPurify.sanitize(html));
  const streamBuffer = new TextStreamBuffer((text) => {
    if (!resources.active || !sending) return;
    streamText += text;
    scrollBottom();
  }, (callback, delay) => resources.schedule(callback, delay));
  resources.own(() => streamBuffer.dispose());

  function renderMessage(content: string): string {
    return renderCache.render(content);
  }

  function toolLabel(name: string): string {
    const labels: Record<string, string> = {
      list_today_classes: "今日の授業",
      list_week_classes: "週間時間割",
      search_courses: "科目検索",
      get_course_context: "科目情報",
      list_luna_todos: "提出物",
      list_recent_notifications: "お知らせ",
      search_notifications: "お知らせ検索",
      get_course_detail: "科目詳細",
      list_recent_mail: "メール",
      read_mail: "メール本文",
      search_mail: "メール検索",
      list_luna_announcements: "Luna掲示",
      get_student_profile: "学生情報",
      get_mail_profile: "メール設定",
      list_syllabus_favorites: "シラバス",
      get_grades: "成績",
      get_cancellations: "休講",
      get_makeup_classes: "補講",
      get_room_changes: "教室変更",
      get_registration: "履修",
      get_exam_timetable: "試験時間割",
      get_weather: "天気",
      get_weekly_summary: "週間まとめ",
      get_upcoming_deadlines: "締切",
      get_todo_guide: "タスク案内",
      get_luna_activity_detail: "Luna詳細",
      refresh_data: "更新",
      list_downloaded_files: "ファイル検索",
      read_downloaded_file: "ファイル読込",
      inspect_file: "ファイル確認",
      write_downloaded_text_file: "ファイル保存",
      open_downloaded_file: "ファイルを開く",
      delete_downloaded_file: "ファイル削除",
      download_url: "URL保存",
      open_luna_attachment: "添付を開く",
      download_luna_attachment: "添付保存",
      download_course_material: "資料保存",
      list_browser_windows: "ブラウザ一覧",
      open_browser_url: "ページを開く",
      open_copilot_page: "Copilotで開く",
      read_browser_page: "ページ読取",
      browser_back: "戻る",
      browser_forward: "進む",
      browser_reload_page: "再読込",
      browser_click: "クリック",
      browser_fill: "入力",
      browser_select_option: "選択",
      browser_press: "キー入力",
      browser_scroll: "スクロール",
      browser_wait_for: "待機",
      browser_close: "閉じる",
      browser_mouse_click: "座標クリック",
      browser_mouse_drag: "ドラッグ",
      get_today_brief: "今日のまとめ",
      get_notification_detail: "本文確認",
      create_google_calendar_event: "予定作成",
      list_google_calendar_events: "予定一覧",
      delete_google_calendar_event: "予定削除",
      update_google_calendar_event: "予定更新",
      computer_screenshot: "画面確認",
      computer_mouse_click: "画面クリック",
      computer_mouse_drag: "画面ドラッグ",
      computer_scroll: "画面スクロール",
    };
    return labels[name] || name;
  }

  const currentPlanText = $derived.by(() => {
    const active = toolChips.find((tool) => tool.state === "running")
      ?? toolChips.find((tool) => tool.state === "pending")
      ?? toolChips.at(-1);
    return active?.detail?.trim() || (active ? toolLabel(active.name) : "");
  });

  function scrollBottom(): void {
    if (!resources.active || scrollFrame !== null) return;
    void tick().then(() => {
      if (!resources.active || scrollFrame !== null) return;
      scrollFrame = requestAnimationFrame(() => {
        scrollFrame = null;
        if (resources.active && messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
      });
    });
  }

  function resizeComposer(): void {
    if (!resources.active || !composerEl) return;
    composerEl.style.height = "auto";
    composerEl.style.height = `${Math.min(composerEl.scrollHeight, 132)}px`;
  }

  const conversationView = new AgentConversationView<AgentMessage[], AgentStreamEvent>({
    scope: resources,
    loadMessages: (id) => invoke<AgentMessage[]>("agent_load_display_messages", { convId: id }),
    listen: (id, receive) => listen<AgentStreamEvent>(`agent_stream:${id}`, (event) => receive(event.payload)),
    select: (id) => {
      if ((id ?? "") !== convId) conversationSelectionVersion += 1;
      if (sending && convId) void invoke("agent_cancel", { convId, turnId: activeRequestId }).catch(() => {});
      turnVersion += 1;
      activeRequestId = null;
      streamBuffer.clear();
      renderCache.clear();
      titleSequence += 1;
      conversationReady = false;
      convId = id ?? "";
      convTitle = "新しい会話";
      document.title = convTitle;
      messages = [];
      sending = false;
      streamText = "";
      toolChips = [];
      conversationMenuOpen = false;
      editingTitle = false;
      error = "";
    },
    applyMessages: (rows) => {
      messages = rows.filter((row) => row.role === "user" || row.role === "assistant");
      scrollBottom();
    },
    receive: handleStream,
  });

  async function refreshConversationTitle(id = convId): Promise<void> {
    if (!id) return;
    const sequence = ++titleSequence;
    try {
      const rows = await invoke<AgentConversationSummary[]>("agent_list_conversations");
      if (!resources.active || id !== convId || sequence !== titleSequence) return;
      conversations = rows;
      const current = rows.find((row) => row.id === id);
      if (!current) return;
      convTitle = current.title.trim() || "新しい会話";
      if (!editingTitle) document.title = convTitle;
    } catch {}
  }

  async function startRename(): Promise<void> {
    if (!convId || sending || preparing) return;
    conversationMenuOpen = false;
    titleDraft = convTitle;
    editingTitle = true;
    await tick();
    titleInputEl?.focus();
    titleInputEl?.select();
  }

  async function toggleConversationMenu(): Promise<void> {
    if (sending || preparing || editingTitle) return;
    if (!conversationMenuOpen) await refreshConversationTitle();
    conversationMenuOpen = !conversationMenuOpen;
  }

  async function selectConversation(id: string): Promise<void> {
    conversationMenuOpen = false;
    if (!id || id === convId || sending || preparing) return;
    try {
      await invoke("agent_set_active_conversation", { convId: id });
      await loadActiveConversation(false);
    } catch (cause) {
      if (resources.active) error = `会話を切り替えられませんでした: ${String(cause)}`;
    }
  }

  function cancelRename(): void {
    editingTitle = false;
    titleDraft = "";
  }

  async function commitRename(): Promise<void> {
    if (!editingTitle || !convId) return;
    const next = titleDraft.trim();
    editingTitle = false;
    titleDraft = "";
    if (!next || next === convTitle) return;
    const id = convId;
    try {
      await invoke("agent_rename_conversation", { convId: id, title: next });
      if (!resources.active || id !== convId) return;
      convTitle = next;
      document.title = next;
    } catch (cause) {
      error = `タイトルを変更できませんでした: ${String(cause)}`;
    }
  }

  function handleTitleKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      void commitRename();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancelRename();
    }
  }

  function queuePanelResize(width: number): void {
    resizeQueuedWidth = width;
    if (resizeInFlight) return;
    resizeInFlight = true;
    void (async () => {
      while (resources.active && resizeQueuedWidth !== null) {
        const nextWidth = resizeQueuedWidth;
        resizeQueuedWidth = null;
        await invoke("document_tabs_resize_agent_panel", { width: nextWidth }).catch(() => {
          resizeQueuedWidth = null;
        });
      }
      resizeInFlight = false;
    })();
  }

  function beginPanelResize(event: PointerEvent): void {
    if (standalone || event.button !== 0) return;
    event.preventDefault();
    resizeDrag = {
      pointerId: event.pointerId,
      startScreenX: event.screenX,
      startWidth: window.innerWidth,
    };
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {}
  }

  function updatePanelResize(event: PointerEvent): void {
    if (!resizeDrag || event.pointerId !== resizeDrag.pointerId) return;
    event.preventDefault();
    queuePanelResize(resizeDrag.startWidth - (event.screenX - resizeDrag.startScreenX));
  }

  function endPanelResize(event?: PointerEvent): void {
    if (!resizeDrag || (event && event.pointerId !== resizeDrag.pointerId)) return;
    try {
      (event?.currentTarget as HTMLElement | undefined)?.releasePointerCapture(resizeDrag.pointerId);
    } catch {}
    resizeDrag = null;
  }

  function handlePanelResizeKeydown(event: KeyboardEvent): void {
    if (standalone || (event.key !== "ArrowLeft" && event.key !== "ArrowRight")) return;
    event.preventDefault();
    const step = event.shiftKey ? 32 : 12;
    queuePanelResize(window.innerWidth + (event.key === "ArrowLeft" ? step : -step));
  }

  // One globally-shared continuous conversation drives BOTH the sidebar agent
  // (across every tab/page) and the main-window agent, so the chat never resets
  // when you switch pages. The current page is still injected per turn (browser
  // context), so "this page" keeps working. Use 新規 to start a fresh chat.
  function loadActiveConversation(createIfMissing = true): Promise<boolean> {
    if (!resources.active) return Promise.resolve(false);
    const sequence = ++contextSequence;
    const current = () => resources.active && sequence === contextSequence;
    conversationLoading = true;
    const pending = (async () => {
      try {
        let id = (await invoke<string | null>("agent_active_conversation")) || "";
        if (!current()) return false;
        if (id && id === convId && conversationReady) return true;
        if (!id) {
          if (!createIfMissing) {
            clearDeletedConversation();
            return false;
          }
          id = await invoke<string>("agent_create_conversation", { title: null });
          if (!current()) return false;
          await invoke("agent_set_active_conversation", { convId: id });
          if (!current()) return false;
        }
        // The common controller owns history, stream registration and A → B → A.
        // Readiness cannot be inferred from the conversation ID alone.
        const selected = await conversationView.select(id, false);
        if (!current() || !selected) return false;
        conversationReady = true;
        // Title metadata is display-only; it must not delay a ready send.
        void refreshConversationTitle(id);
        return true;
      } catch (cause) {
        if (current()) throw cause;
        return false;
      } finally {
        if (sequence === contextSequence) conversationLoading = false;
      }
    })();
    conversationLoad = pending;
    const finished = () => { if (conversationLoad === pending) conversationLoad = null; };
    void pending.then(finished, finished);
    return pending;
  }

  function clearDeletedConversation(): void {
    conversationView.clear(false);
  }

  function conversationDeleted(id: string): void {
    if (!id || !resources.active || (id !== convId && !conversationLoading)) return;
    const reload = conversationLoading;
    contextSequence += 1;
    conversationLoading = false;
    conversationView.deleted(id);
    // Invalidate an in-flight startup/selection read even when its ID has not
    // arrived yet. Re-read durable selection without creating an empty chat.
    if (reload) void loadActiveConversation(false).catch((cause) => {
      if (resources.active) error = `会話を読み込めませんでした: ${String(cause)}`;
    });
  }

  async function newChat(): Promise<void> {
    if (sending || preparing) return;
    conversationMenuOpen = false;
    try {
      const id = await invoke<string>("agent_create_conversation", { title: null });
      if (!resources.active) return;
      await invoke("agent_set_active_conversation", { convId: id });
      await loadActiveConversation();
      composerEl?.focus();
    } catch (cause) {
      error = `新しい会話を作成できませんでした: ${String(cause)}`;
    }
  }

  // Only updates the per-turn page context (which page the next message is about).
  // The conversation itself is the shared active one and does NOT change with the
  // page — that's what keeps cross-page chats continuous.
  function applyContext(target: string, title: string, kind: string): void {
    if (!resources.active) return;
    const normalizedTarget = target.trim();
    if (!normalizedTarget) return;
    contextReadVersion += 1;
    pageTarget = normalizedTarget;
    pageTitle = title.trim() || pageTitle || "エージェント";
    pageKind = kind.trim() || pageKind || "detail";
  }

  async function refreshActiveContext(force = false): Promise<void> {
    if (!resources.active || owner !== "document-tabs" || (!force && document.hidden)) return;
    if (contextRead) {
      await contextRead;
      if (force && resources.active) await refreshActiveContext(true);
      return;
    }
    const version = contextReadVersion;
    contextRead = (async () => {
      try {
        const tabs = await invoke<DocumentTab[]>("document_tabs_list", { owner });
        if (!resources.active || version !== contextReadVersion) return;
        const active = tabs.find((tab) => tab.active);
        if (active) applyContext(active.target, active.title, active.type);
        else applyContext(owner, "エージェント", "agent");
      } catch {}
      finally { contextRead = null; }
    })();
    await contextRead;
  }

  function handleStream(event: AgentStreamEvent): void {
    if (!resources.active || !sending || !activeRequestId || event.turn_id !== activeRequestId) return;
    if (event.type === "plan") {
      toolChips = [
        ...toolChips,
        ...event.steps.map((step) => ({ id: ++chipCounter, ...step, state: "pending" as const })),
      ];
    } else if (event.type === "tool_call") {
      const pending = toolChips.find((chip) => chip.name === event.name && chip.state === "pending");
      if (pending) {
        toolChips = toolChips.map((chip) => chip.id === pending.id ? { ...chip, state: "running" } : chip);
      } else {
        toolChips = [...toolChips, { id: ++chipCounter, name: event.name, state: "running" }];
      }
    } else if (event.type === "tool_result") {
      const match = toolChips.find((chip) => chip.name === event.name && chip.state === "running")
        ?? toolChips.find((chip) => chip.name === event.name && chip.state === "pending");
      if (match) toolChips = toolChips.map((chip) => chip.id === match.id ? { ...chip, state: event.ok ? "ok" : "err" } : chip);
    } else if (event.type === "token") {
      streamBuffer.append(event.text);
      return;
    } else if (event.type === "error") {
      streamBuffer.flush();
      if (isContextLimitMessage(event.message) && keepStreamedAnswer()) {
        error = "";
      } else {
        error = event.message;
      }
      void finishTurn(false);
    } else if (event.type === "done") {
      streamBuffer.flush();
      void finishTurn(true);
    }
    scrollBottom();
  }

  async function finishTurn(reload: boolean, version = turnVersion): Promise<void> {
    if (!resources.active || version !== turnVersion) return;
    streamBuffer.clear();
    sending = false;
    toolChips = [];
    streamText = "";
    if (reload && convId) {
      try {
        // New sends invalidate this read, including in the same conversation.
        await conversationView.reload(() => version === turnVersion);
      } catch (cause) {
        if (resources.active && version === turnVersion) error = `会話を読み込めませんでした: ${String(cause)}`;
      }
    }
    scrollBottom();
  }

  function imageSrc(part: AgentImagePart): string {
    return `data:${part.mime};base64,${part.data_base64}`;
  }

  async function addFiles(files: Iterable<File>): Promise<void> {
    if (!resources.active) return;
    attachmentReads++;
    attachmentError = "";
    try {
      await appendAgentAttachments(files, {
        active: () => resources.active,
        count: () => attachments.length,
        append: part => { attachments = [...attachments, part]; },
        error: message => { attachmentError = message; },
      });
    } finally {
      if (resources.active) attachmentReads--;
    }
    if (resources.active) composerEl?.focus();
  }

  function openFilePicker(): void {
    fileInput?.click();
  }

  async function onPickFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = "";
    await addFiles(files);
  }

  function removeAttachment(index: number): void {
    attachments = attachments.filter((_, i) => i !== index);
    attachmentError = "";
  }

  async function handlePaste(event: ClipboardEvent): Promise<void> {
    const items = event.clipboardData?.items;
    if (!items) return;
    const images: File[] = [];
    for (const item of items) {
      if (item.kind === "file") {
        const file = item.getAsFile();
        if (file && isAgentAttachmentFile(file)) images.push(file);
      }
    }
    if (images.length) {
      event.preventDefault();
      await addFiles(images);
    }
  }

  async function handleDrop(event: DragEvent): Promise<void> {
    const files = event.dataTransfer?.files;
    if (!files?.length) return;
    event.preventDefault();
    await addFiles(Array.from(files));
  }

  function isContextLimitMessage(message: string): boolean {
    return message.includes("コンテキスト上限");
  }

  let keptContextAnswer = false;

  function keepStreamedAnswer(): boolean {
    streamBuffer.flush();
    if (keptContextAnswer) return true;
    const partial = streamText.trim();
    if (!partial) return false;
    keptContextAnswer = true;
    messages = [...messages, {
      id: -Date.now(),
      conv_id: convId,
      role: "assistant",
      content: partial,
      images: null,
      created_at: Math.floor(Date.now() / 1000),
    }];
    streamText = "";
    return true;
  }

  async function send(): Promise<void> {
    const inputDraft = draft;
    const content = inputDraft.trim();
    const selectedAttachments = attachments;
    const images = selectedAttachments.filter(isAgentImagePart);
    const documents = selectedAttachments.filter(part => !isAgentImagePart(part));
    if (!resources.active || (!content && selectedAttachments.length === 0) || sending || preparing || attachmentReads > 0 || !pageTarget) return;
    preparing = true;
    const preparation = ++preparationVersion;
    const previousId = convId;
    const previousSelection = conversationSelectionVersion;
    // Re-registering a failed subscription for the same ID is preparation, not
    // a user switch. A → B → A still changes the selection version twice.
    const preparationCurrent = () => resources.active && preparation === preparationVersion && (!previousId || previousSelection === conversationSelectionVersion);
    let ownsTurn: (() => boolean) | null = null;
    let version = 0;
    try {
      // Share startup work rather than creating or reading a second conversation.
      let ready = conversationLoad
        ? await conversationLoad
        : conversationReady || await loadActiveConversation();
      // A duplicate notification can supersede the outer shared-pointer read
      // while retaining this exact selection and its owned subscription.
      while (!ready && previousId && preparationCurrent()) {
        const pending = conversationLoad;
        if (!pending) { ready = conversationReady; break; }
        ready = await pending;
      }
      if (!preparationCurrent() || !ready || !convId || !conversationReady || sending) return;
      // Context lookup is still preparation. Cancellation or a view switch
      // during that IO must leave the draft intact and create no optimistic row.
      await refreshActiveContext(true);
      if (!preparationCurrent() || !convId || !conversationReady || sending) return;
      const currentConv = convId;
      const selected = conversationView.capture();
      version = ++turnVersion;
      ownsTurn = () => selected() && version === turnVersion;
      conversationView.invalidateMessages();
      error = "";
      keptContextAnswer = false;
      // Text and files added while preparation was pending belong to the next
      // input; only consume the exact draft and attachment list being sent.
      if (draft === inputDraft) draft = "";
      attachments = attachments.filter((image) => !selectedAttachments.includes(image));
      resizeComposer();
      messages = [...messages, {
        id: -Date.now(),
        conv_id: currentConv,
        role: "user",
        content,
        images: images.length ? images : null,
        documents,
        created_at: Math.floor(Date.now() / 1000),
      }];
      const requestId = crypto.randomUUID();
      activeRequestId = requestId;
      sending = true;
      preparing = false;
      streamText = "";
      toolChips = [];
      streamBuffer.clear();
      scrollBottom();
      if (hasPageContext) {
        await invoke("agent_send_with_context", {
          convId: currentConv,
          turnId: requestId,
          content,
          images,
          ...(documents.length ? { documents } : {}),
          browserTarget: pageTarget,
          pageTitle,
          pageKind,
        });
      } else {
        await invoke("agent_send", { convId: currentConv, content, images, turnId: requestId, ...(documents.length ? { documents } : {}) });
      }
      if (ownsTurn() && sending) await finishTurn(true, version);
    } catch (cause) {
      if (ownsTurn) {
        if (!ownsTurn() || !sending) return;
        const message = String(cause);
        if (isContextLimitMessage(message) && keepStreamedAnswer()) error = "";
        else error = `送信に失敗しました: ${message}`;
        await finishTurn(false, version);
      } else if (preparationCurrent()) {
        error = `送信の準備に失敗しました: ${String(cause)}`;
      }
    } finally {
      if (resources.active && preparation === preparationVersion) preparing = false;
    }
  }

  async function stop(): Promise<void> {
    preparationVersion += 1;
    preparing = false;
    if (!sending || !convId) return;
    turnVersion += 1;
    conversationView.invalidateMessages();
    sending = false;
    streamBuffer.clear();
    await invoke("agent_cancel", { convId, turnId: activeRequestId }).catch(() => {});
  }

  async function toggleStt(): Promise<void> {
    if (!resources.active || sttStarting) return;
    if (sttListening) {
      sttStopRequested = true;
      await speechInput.stop().catch((cause) => { if (resources.active) error = String(cause); });
      return;
    }
    sttStarting = true;
    try {
      sttBaseText = draft;
      sttCommittedText = "";
      sttPartialText = "";
      sttStopRequested = false;
      await speechInput.start();
    } catch (cause) {
      if (resources.active) error = `音声入力を開始できませんでした: ${String(cause)}`;
    } finally {
      if (resources.active) sttStarting = false;
    }
  }

  async function runAction(): Promise<void> {
    if (actionMode === "stop") await stop();
    else if (actionMode === "mic") await toggleStt();
    else await send();
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key !== "Enter" || event.shiftKey) return;
    if (composing || event.isComposing || performance.now() < suppressEnterUntil) return;
    event.preventDefault();
    void send();
  }

  async function initializePanel(): Promise<void> {
    applyContext(initialTarget || owner, initialTitle, initialKind);
    await syncAuxiliaryTheme(() => resources.active);
    if (!resources.active) return;
    await Promise.all([
      listenPanel<string>("theme-changed", (event) => applyAuxiliaryTheme(event.payload)).catch(() => null),
      listenPanel("app-theme-changed", () => void syncAuxiliaryTheme(() => resources.active)).catch(() => null),
      listenPanel<DocumentTabsChanged>("document-tabs-changed", (event) => {
        if (!event.payload || event.payload.owner !== owner) return;
        const active = event.payload.tabs.find((tab) => tab.active);
        if (active) applyContext(active.target, active.title, active.type);
        else applyContext(owner, "エージェント", "agent");
      }).catch(() => null),
      // Follow the shared active conversation: when it changes (main agent picks a
      // conversation, or 新規 elsewhere), reload so the sidebar stays in sync.
      listenPanel<string>("agent-active-conversation-changed", (event) => {
        // Read the durable pointer even for a late event naming this view's ID.
        // Event recovery never creates a replacement conversation.
        void loadActiveConversation(false).catch((cause) => {
          if (resources.active) error = `会話を読み込めませんでした: ${String(cause)}`;
        });
      }).catch(() => null),
      listenPanel<string>("agent-conversation-deleted", (event) => conversationDeleted(event.payload)).catch(() => null),
      listenPanel<string>("agent-conversations-changed", (event) => {
        if (!event.payload || event.payload === convId) void refreshConversationTitle();
      }).catch(() => null),
      listenPanel<AgentSpeechEvent & { text: string }>("stt-partial", (event) => {
        if (!speechInput.accepts(event.payload)) return;
        sttPartialText = event.payload.text || "";
        draft = mergeSttText(sttBaseText, sttCommittedText, sttPartialText);
        resizeComposer();
      }).catch(() => null),
      listenPanel<AgentSpeechEvent & { text: string }>("stt-final", (event) => {
        if (!speechInput.accepts(event.payload)) return;
        sttCommittedText = appendSttFinal(sttCommittedText, event.payload.text || "");
        sttPartialText = "";
        draft = mergeSttText(sttBaseText, sttCommittedText, "");
        resizeComposer();
      }).catch(() => null),
      listenPanel<AgentSpeechEvent & { state: string }>("stt-state", (event) => {
        if (!speechInput.accepts(event.payload)) return;
        const wasListening = sttListening;
        speechInput.state(event.payload);
        if (!sttListening) {
          draft = mergeSttText(sttBaseText, sttCommittedText, "");
          const shouldSend = wasListening && sttStopRequested && !!sttCommittedText.trim();
          sttStopRequested = false;
          if (shouldSend) void send();
        }
      }).catch(() => null),
      listenPanel<AgentSpeechEvent & { message: string }>("stt-error", (event) => {
        if (!speechInput.accepts(event.payload)) return;
        speechInput.error(event.payload);
        error = event.payload.message || "音声入力エラー";
      }).catch(() => null),
    ]);
    if (!resources.active) return;
    await speechInput.refresh();
    if (!resources.active) return;
    await loadActiveConversation();
    if (!resources.active) return;
    await refreshActiveContext();
    if (!resources.active) return;
    if (owner === "document-tabs") {
      // Tab/navigation events provide updates; this is only a recovery read.
      const timer = window.setInterval(() => void refreshActiveContext(), 30_000);
      resources.own(() => window.clearInterval(timer));
      const refresh = resources.guard(() => { void refreshActiveContext(); });
      window.addEventListener("focus", refresh);
      document.addEventListener("visibilitychange", refresh);
      resources.own(() => {
        window.removeEventListener("focus", refresh);
        document.removeEventListener("visibilitychange", refresh);
      });
    }
    composerEl?.focus();
  }

  onMount(() => {
    void initializePanel().catch((cause) => {
      if (resources.active) error = `エージェントを準備できませんでした: ${String(cause)}`;
    });
  });

  onDestroy(() => {
    resources.dispose();
    endPanelResize();
    contextSequence++;
    resizeQueuedWidth = null;
    if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
    if (sending && convId) void invoke("agent_cancel", { convId, turnId: activeRequestId }).catch(() => {});
  });

  $effect(() => {
    draft;
    void tick().then(resizeComposer);
  });
</script>

<aside class="agent-panel" class:standalone class:embedded={!standalone}>
  {#if !standalone}
    <button
      class="agent-resize-handle"
      class:dragging={resizeDrag !== null}
      type="button"
      title="サイドバーの幅を変更"
      aria-label="サイドバーの幅を変更"
      onpointerdown={beginPanelResize}
      onpointermove={updatePanelResize}
      onpointerup={endPanelResize}
      onpointercancel={endPanelResize}
      onkeydown={handlePanelResizeKeydown}
    >
      <span aria-hidden="true"></span>
    </button>
  {/if}
  <header class="agent-topbar" data-tauri-drag-region={standalone ? "" : undefined}>
    <div class="agent-head-text">
      <div class="agent-title-row">
        {#if editingTitle}
          <input
            class="agent-title-input"
            bind:this={titleInputEl}
            bind:value={titleDraft}
            maxlength="80"
            aria-label="会話タイトル"
            onkeydown={handleTitleKeydown}
            onblur={commitRename}
          />
        {:else}
          <button
            class="agent-title-switch"
            class:open={conversationMenuOpen}
            type="button"
            title="会話を切り替える"
            aria-label="会話を切り替える"
            aria-expanded={conversationMenuOpen}
            disabled={sending || preparing}
            onclick={toggleConversationMenu}
          >
            <span class="agent-title" title={convTitle}>{convTitle}</span>
            <span class="agent-title-caret" aria-hidden="true"><Icon name="chevron.right" size={11} /></span>
          </button>
        {/if}
      </div>
      <div class="agent-page-row" title={pageTitle}>
        <span class="agent-kind-icon" title={kindLabel} aria-label={kindLabel}>
          <Icon name={kindIcon} size={13} />
        </span>
        <div class="agent-page-title">{pageTitle}</div>
      </div>
    </div>
    <div class="agent-top-actions">
      <AgentIslandIconButton icon="pencil" size={14} title="タイトルを変更" disabled={sending || preparing || editingTitle} onclick={startRename} />
      <AgentIslandIconButton icon="plus" size={15} title="新しい会話" disabled={sending || preparing} onclick={newChat} />
    </div>
    {#if conversationMenuOpen}
      <div class="agent-conversation-menu">
        {#each conversations as conversation (conversation.id)}
          <button
            class="agent-conversation-item"
            class:active={conversation.id === convId}
            type="button"
            onclick={() => selectConversation(conversation.id)}
          >
            <span>{conversation.title || "新しい会話"}</span>
            {#if conversation.id === convId}<Icon name="checkmark.circle" size={13} />{/if}
          </button>
        {/each}
      </div>
    {/if}
  </header>

  <section class="agent-messages" bind:this={messagesEl} aria-live="polite">
    {#if messages.length === 0 && !sending}
      <div class="agent-empty">
        <img src={selahLogoUrl} alt="Selah" />
        <strong>ページを見ながら頼めます</strong>
        <span>開いているブラウザや詳細ページの内容を読んで、クリックや要約までこの場で続けます。</span>
      </div>
    {/if}

    {#each messages as message (message.id)}
      <article class:user={message.role === "user"} class:assistant={message.role === "assistant"} class="agent-row">
        {#if message.role === "assistant"}
          <div class="agent-bubble assistant-copy">{@html renderMessage(message.content)}</div>
        {:else}
          <div class="agent-bubble user-copy">
            {#if message.images?.length}
              <div class="agent-bubble-images">
                {#each message.images as img}
                  <img class="agent-bubble-image" src={imageSrc(img)} alt="添付画像" />
                {/each}
              </div>
            {/if}
            {#each message.documents ?? [] as document}<AgentDocumentPreview {document} />{/each}
            {#if message.content}<span>{message.content}</span>{/if}
          </div>
        {/if}
      </article>
    {/each}

    {#if sending}
      <article class="agent-row assistant">
        <div class="agent-bubble assistant-copy streaming">
          {#if streamText}
            {@html renderCache.renderTransient(streamText)}
          {:else}
            <AgentThinkingStatus text={currentPlanText} />
          {/if}
        </div>
      </article>
    {/if}

    {#if error}
      <article class="agent-row assistant">
        <div class="agent-bubble agent-error">……エラーが出たみたい。<br /><br />{error}</div>
      </article>
    {/if}
  </section>

  <footer class="agent-composer-wrap" ondragover={(e) => e.preventDefault()} ondrop={handleDrop}>
    <input
      bind:this={fileInput}
      type="file"
      accept={AGENT_ATTACHMENT_ACCEPT}
      multiple
      class="agent-file-input"
      onchange={onPickFiles}
    />
    {#if attachments.length}
      <div class="agent-attachments">
        {#each attachments as att, i}
          <div class="agent-attachment" class:document={!isAgentImagePart(att)}>
            {#if isAgentImagePart(att)}
              <img src={imageSrc(att)} alt="添付画像" />
              {:else}<AgentDocumentPreview document={att} />{/if}
            <button type="button" class="agent-attachment-remove" title="削除" aria-label="添付を削除" onclick={() => removeAttachment(i)}>
              <Icon name="xmark" size={11} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
    <AgentAttachmentStatus reading={attachmentReads} count={attachments.length} error={attachmentError} />
    <div class="agent-send-row">
      <div class="agent-composer-island">
        <button
          type="button"
          class="agent-attach-button"
          title="ファイルを添付"
          aria-label="ファイルを添付"
          onclick={openFilePicker}
        >
          <Icon name="plus" size={18} />
        </button>
        <textarea
          bind:this={composerEl}
          bind:value={draft}
          rows="1"
          placeholder={hasPageContext ? "このページについて聞く" : "エージェントに相談する"}
          aria-label="エージェントへのメッセージ"
          onkeydown={handleKeydown}
          onpaste={handlePaste}
          oncompositionstart={() => composing = true}
          oncompositionend={() => {
            composing = false;
            suppressEnterUntil = performance.now() + 160;
          }}
        ></textarea>
      </div>
      <div class="agent-action-slot">
        <button
          class:mic={actionMode === "mic"}
          class:recording={sttListening}
          class:stop={actionMode === "stop"}
          class="agent-action-capsule"
          type="button"
          title={actionMode === "stop" ? "停止" : actionMode === "mic" ? (sttListening ? "音声入力を停止" : "音声入力") : "送信"}
          aria-label={actionMode === "stop" ? "停止" : actionMode === "mic" ? (sttListening ? "音声入力を停止" : "音声入力") : "送信"}
          onclick={runAction}
          disabled={actionMode !== "stop" && attachmentReads > 0}
        >
          <span class="agent-action-capsule-stack" aria-hidden="true">
            <span class="agent-action-face" class:visible={actionMode === "send"}>
              <Icon name="paperplane" size={14} />
              <span>送る</span>
            </span>
            <span class="agent-action-face" class:visible={actionMode === "mic"}>
              <Icon name="microphone" size={14} />
              <span>{sttListening ? "停止" : "音声"}</span>
            </span>
            <span class="agent-action-face" class:visible={actionMode === "stop"}>
              <Icon name="stop" size={14} />
              <span>停止</span>
            </span>
          </span>
        </button>
      </div>
    </div>
    <div class="agent-composer-hint">Enter で送信、Shift+Enter で改行</div>
  </footer>
</aside>
