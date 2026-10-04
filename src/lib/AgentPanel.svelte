<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import DOMPurify from "dompurify";
  import { marked } from "marked";
  import { onDestroy, onMount, tick } from "svelte";
  import selahLogoUrl from "../assets/logo.png";
  import AgentThinkingStatus from "./AgentThinkingStatus.svelte";
  import AgentIslandIconButton from "./AgentIslandIconButton.svelte";
  import { applyAuxiliaryTheme, syncAuxiliaryTheme } from "./auxiliarySurfaceTheme";
  import Icon, { type IconName } from "./Icon.svelte";
  import type { AgentImagePart, AgentMessage, AgentStreamEvent } from "./api";
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
  let attachments = $state<AgentImagePart[]>([]);
  let fileInput = $state<HTMLInputElement | null>(null);
  let sending = $state(false);
  let error = $state("");
  let streamText = $state("");
  let toolChips = $state<ToolChip[]>([]);
  let sttListening = $state(false);
  let sttBaseText = $state("");
  let sttCommittedText = $state("");
  let sttPartialText = $state("");
  let sttStopRequested = $state(false);
  let preemptedCaller = $state<string | null>(null);
  let messagesEl = $state<HTMLElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let resizeDrag = $state<{ pointerId: number; startScreenX: number; startWidth: number } | null>(null);
  let resizeQueuedWidth: number | null = null;
  let resizeInFlight = false;
  let composing = false;
  let suppressEnterUntil = 0;
  let contextSequence = 0;
  let chipCounter = 0;
  let contextTimer: number | null = null;
  let unlistenStream: UnlistenFn | null = null;
  let unlistenActiveConv: UnlistenFn | null = null;
  let unlistenConversationsChanged: UnlistenFn | null = null;
  let unlistenTabs: UnlistenFn | null = null;
  let unlistenTheme: UnlistenFn | null = null;
  let unlistenAppTheme: UnlistenFn | null = null;
  let unlistenSttPartial: UnlistenFn | null = null;
  let unlistenSttFinal: UnlistenFn | null = null;
  let unlistenSttState: UnlistenFn | null = null;
  let unlistenSttError: UnlistenFn | null = null;

  const actionMode = $derived<ActionMode>(
    sending ? "stop" : sttListening || (!draft.trim() && attachments.length === 0) ? "mic" : "send"
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
  marked.setOptions({ breaks: true, gfm: true });

  const renderCache = new Map<string, string>();
  const RENDER_CACHE_MAX = 256;

  function renderMessage(content: string): string {
    const cached = renderCache.get(content);
    if (cached !== undefined) return cached;
    const out = DOMPurify.sanitize(marked.parse(content || "") as string);
    if (renderCache.size >= RENDER_CACHE_MAX) {
      const firstKey = renderCache.keys().next().value;
      if (firstKey !== undefined) renderCache.delete(firstKey);
    }
    renderCache.set(content, out);
    return out;
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
    void tick().then(() => requestAnimationFrame(() => {
      if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
    }));
  }

  function resizeComposer(): void {
    if (!composerEl) return;
    composerEl.style.height = "auto";
    composerEl.style.height = `${Math.min(composerEl.scrollHeight, 132)}px`;
  }

  function mergeSttText(base: string, committed: string, partial: string): string {
    const spoken = [committed.trim(), partial.trim()].filter(Boolean).join(" ").trim();
    if (!spoken) return base;
    if (!base) return spoken;
    return /\s$/.test(base) ? `${base}${spoken}` : `${base}\n${spoken}`;
  }

  async function bindStream(id: string): Promise<void> {
    unlistenStream?.();
    unlistenStream = await listen<AgentStreamEvent>(`agent_stream:${id}`, (event) => {
      handleStream(event.payload);
    });
  }

  async function refreshConversationTitle(id = convId): Promise<void> {
    if (!id) return;
    try {
      const rows = await invoke<AgentConversationSummary[]>("agent_list_conversations");
      conversations = rows;
      const current = rows.find((row) => row.id === id);
      if (!current) return;
      convTitle = current.title.trim() || "新しい会話";
      if (!editingTitle) document.title = convTitle;
    } catch {}
  }

  async function startRename(): Promise<void> {
    if (!convId || sending) return;
    conversationMenuOpen = false;
    titleDraft = convTitle;
    editingTitle = true;
    await tick();
    titleInputEl?.focus();
    titleInputEl?.select();
  }

  async function toggleConversationMenu(): Promise<void> {
    if (sending || editingTitle) return;
    if (!conversationMenuOpen) await refreshConversationTitle();
    conversationMenuOpen = !conversationMenuOpen;
  }

  async function selectConversation(id: string): Promise<void> {
    conversationMenuOpen = false;
    if (!id || id === convId || sending) return;
    await invoke("agent_set_active_conversation", { convId: id }).catch((cause) => {
      error = `会話を切り替えられませんでした: ${String(cause)}`;
    });
    await loadActiveConversation();
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
    try {
      await invoke("agent_rename_conversation", { convId, title: next });
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
      while (resizeQueuedWidth !== null) {
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
  async function loadActiveConversation(): Promise<void> {
    const sequence = ++contextSequence;
    error = "";
    sending = false;
    streamText = "";
    toolChips = [];
    let id = (await invoke<string | null>("agent_active_conversation").catch(() => null)) || "";
    if (id) {
      try {
        await invoke<AgentMessage[]>("agent_load_messages", { convId: id });
      } catch {
        id = "";
      }
    }
    if (!id) {
      id = await invoke<string>("agent_create_conversation", { title: null });
      await invoke("agent_set_active_conversation", { convId: id }).catch(() => {});
    }
    const rows = await invoke<AgentMessage[]>("agent_load_messages", { convId: id });
    if (sequence !== contextSequence) return;
    convId = id;
    messages = rows.filter((row) => row.role === "user" || row.role === "assistant");
    await refreshConversationTitle(id);
    await bindStream(id);
    scrollBottom();
  }

  async function newChat(): Promise<void> {
    if (sending) return;
    conversationMenuOpen = false;
    try {
      const id = await invoke<string>("agent_create_conversation", { title: null });
      await invoke("agent_set_active_conversation", { convId: id }).catch(() => {});
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
    const normalizedTarget = target.trim();
    if (!normalizedTarget) return;
    pageTarget = normalizedTarget;
    pageTitle = title.trim() || pageTitle || "エージェント";
    pageKind = kind.trim() || pageKind || "detail";
  }

  async function refreshActiveContext(): Promise<void> {
    if (owner !== "document-tabs") return;
    try {
      const tabs = await invoke<DocumentTab[]>("document_tabs_list", { owner });
      const active = tabs.find((tab) => tab.active);
      if (active) applyContext(active.target, active.title, active.type);
      else applyContext(owner, "エージェント", "agent");
    } catch {}
  }

  function handleStream(event: AgentStreamEvent): void {
    if (!sending) return;
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
      streamText += event.text;
    } else if (event.type === "error") {
      if (isContextLimitMessage(event.message) && keepStreamedAnswer()) {
        error = "";
      } else {
        error = event.message;
      }
      void finishTurn(false);
    } else if (event.type === "done") {
      void finishTurn(true);
    }
    scrollBottom();
  }

  async function finishTurn(reload: boolean): Promise<void> {
    sending = false;
    toolChips = [];
    streamText = "";
    if (reload && convId) {
      try {
        messages = (await invoke<AgentMessage[]>("agent_load_messages", { convId }))
          .filter((row) => row.role === "user" || row.role === "assistant");
      } catch {}
    }
    scrollBottom();
  }

  const MAX_ATTACHMENTS = 4;
  const MAX_IMAGE_BYTES = 10 * 1024 * 1024;

  function imageSrc(part: AgentImagePart): string {
    return `data:${part.mime};base64,${part.data_base64}`;
  }

  function fileToImagePart(file: File): Promise<AgentImagePart | null> {
    return new Promise((resolve) => {
      if (!file.type.startsWith("image/")) return resolve(null);
      if (file.size > MAX_IMAGE_BYTES) {
        error = "画像が大きすぎます（10MBまで）";
        return resolve(null);
      }
      const reader = new FileReader();
      reader.onload = () => {
        const result = typeof reader.result === "string" ? reader.result : "";
        const comma = result.indexOf(",");
        if (comma < 0) return resolve(null);
        resolve({ mime: file.type || "image/png", data_base64: result.slice(comma + 1) });
      };
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(file);
    });
  }

  async function addFiles(files: Iterable<File>): Promise<void> {
    for (const file of files) {
      if (attachments.length >= MAX_ATTACHMENTS) {
        error = `画像は最大${MAX_ATTACHMENTS}枚までです`;
        break;
      }
      const part = await fileToImagePart(file);
      if (part) attachments = [...attachments, part];
    }
    composerEl?.focus();
  }

  function openFilePicker(): void {
    fileInput?.click();
  }

  async function onPickFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    if (input.files) await addFiles(Array.from(input.files));
    input.value = "";
  }

  function removeAttachment(index: number): void {
    attachments = attachments.filter((_, i) => i !== index);
  }

  async function handlePaste(event: ClipboardEvent): Promise<void> {
    const items = event.clipboardData?.items;
    if (!items) return;
    const images: File[] = [];
    for (const item of items) {
      if (item.kind === "file" && item.type.startsWith("image/")) {
        const file = item.getAsFile();
        if (file) images.push(file);
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
    const images = Array.from(files).filter((f) => f.type.startsWith("image/"));
    if (images.length) {
      event.preventDefault();
      await addFiles(images);
    }
  }

  function isContextLimitMessage(message: string): boolean {
    return message.includes("コンテキスト上限");
  }

  let keptContextAnswer = false;

  function keepStreamedAnswer(): boolean {
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
    const content = draft.trim();
    const images = attachments;
    if ((!content && images.length === 0) || sending || !pageTarget) return;
    if (!convId) await loadActiveConversation();
    const currentConv = convId;
    error = "";
    keptContextAnswer = false;
    draft = "";
    attachments = [];
    resizeComposer();
    messages = [...messages, {
      id: -Date.now(),
      conv_id: currentConv,
      role: "user",
      content,
      images: images.length ? images : null,
      created_at: Math.floor(Date.now() / 1000),
    }];
    sending = true;
    streamText = "";
    toolChips = [];
    scrollBottom();
    try {
      if (hasPageContext) {
        await invoke("agent_send_with_context", {
          convId: currentConv,
          content,
          images,
          browserTarget: pageTarget,
          pageTitle,
          pageKind,
        });
      } else {
        await invoke("agent_send", {
          convId: currentConv,
          content,
          images,
        });
      }
      if (sending) await finishTurn(true);
    } catch (cause) {
      const message = String(cause);
      if (isContextLimitMessage(message) && keepStreamedAnswer()) {
        error = "";
      } else {
        error = `送信に失敗しました: ${message}`;
      }
      await finishTurn(false);
    }
  }

  async function stop(): Promise<void> {
    if (!sending || !convId) return;
    sending = false;
    await invoke("agent_cancel", { convId }).catch(() => {});
  }

  async function toggleStt(): Promise<void> {
    if (sttListening) {
      sttStopRequested = true;
      await invoke("stt_stop_stream").catch((cause) => error = String(cause));
      return;
    }
    try {
      sttBaseText = draft;
      sttCommittedText = "";
      sttPartialText = "";
      sttStopRequested = false;
      preemptedCaller = await invoke<string | null>("stt_start_stream", { caller: "agent", preempt: true });
    } catch (cause) {
      error = `音声入力を開始できませんでした: ${String(cause)}`;
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

  async function refreshSttState(): Promise<void> {
    try {
      const [running, caller] = await Promise.all([
        invoke<boolean>("stt_is_running"),
        invoke<string | null>("stt_get_active_caller"),
      ]);
      sttListening = running && caller === "agent";
    } catch {
      sttListening = false;
    }
  }

  onMount(async () => {
    await syncAuxiliaryTheme();
    unlistenTheme = await listen<string>("theme-changed", (event) => applyAuxiliaryTheme(event.payload)).catch(() => null);
    unlistenAppTheme = await listen("app-theme-changed", () => void syncAuxiliaryTheme()).catch(() => null);
    unlistenTabs = await listen<DocumentTabsChanged>("document-tabs-changed", (event) => {
      if (!event.payload || event.payload.owner !== owner) return;
      const active = event.payload.tabs.find((tab) => tab.active);
      if (active) applyContext(active.target, active.title, active.type);
      else applyContext(owner, "エージェント", "agent");
    }).catch(() => null);
    // Follow the shared active conversation: when it changes (main agent picks a
    // conversation, or 新規 elsewhere), reload so the sidebar stays in sync.
    unlistenActiveConv = await listen<string>("agent-active-conversation-changed", (event) => {
      const id = event.payload;
      if (id && id !== convId) void loadActiveConversation();
    }).catch(() => null);
    unlistenConversationsChanged = await listen<string>("agent-conversations-changed", (event) => {
      if (!event.payload || event.payload === convId) void refreshConversationTitle();
    }).catch(() => null);
    unlistenSttPartial = await listen<{ text: string; caller: string }>("stt-partial", (event) => {
      if (event.payload.caller !== "agent") return;
      sttPartialText = event.payload.text || "";
      draft = mergeSttText(sttBaseText, sttCommittedText, sttPartialText);
      resizeComposer();
    }).catch(() => null);
    unlistenSttFinal = await listen<{ text: string; caller: string }>("stt-final", (event) => {
      if (event.payload.caller !== "agent") return;
      sttCommittedText = event.payload.text || sttCommittedText;
      sttPartialText = "";
      draft = mergeSttText(sttBaseText, sttCommittedText, "");
      resizeComposer();
    }).catch(() => null);
    unlistenSttState = await listen<{ state: string; caller: string }>("stt-state", (event) => {
      if (event.payload.caller !== "agent") return;
      const wasListening = sttListening;
      sttListening = event.payload.state === "initializing" || event.payload.state === "listening";
      if (!sttListening) {
        draft = mergeSttText(sttBaseText, sttCommittedText, "");
        const shouldSend = wasListening && sttStopRequested && !!sttCommittedText.trim();
        sttStopRequested = false;
        if (preemptedCaller) {
          const caller = preemptedCaller;
          preemptedCaller = null;
          void invoke("stt_start_stream", { caller }).catch(() => {});
        }
        if (shouldSend) void send();
      }
    }).catch(() => null);
    unlistenSttError = await listen<{ message: string; caller: string }>("stt-error", (event) => {
      if (event.payload.caller !== "agent") return;
      sttListening = false;
      error = event.payload.message || "音声入力エラー";
    }).catch(() => null);
    await refreshSttState();
    applyContext(initialTarget || owner, initialTitle, initialKind);
    await loadActiveConversation();
    await refreshActiveContext();
    contextTimer = window.setInterval(() => void refreshActiveContext(), 900);
    composerEl?.focus();
  });

  onDestroy(() => {
    endPanelResize();
    contextSequence++;
    unlistenStream?.();
    unlistenActiveConv?.();
    unlistenConversationsChanged?.();
    unlistenTabs?.();
    unlistenTheme?.();
    unlistenAppTheme?.();
    unlistenSttPartial?.();
    unlistenSttFinal?.();
    unlistenSttState?.();
    unlistenSttError?.();
    if (contextTimer !== null) window.clearInterval(contextTimer);
    if (sttListening) void invoke("stt_stop_stream").catch(() => {});
    if (sending && convId) void invoke("agent_cancel", { convId }).catch(() => {});
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
            disabled={sending}
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
      <AgentIslandIconButton icon="pencil" size={14} title="タイトルを変更" disabled={sending || editingTitle} onclick={startRename} />
      <AgentIslandIconButton icon="plus" size={15} title="新しい会話" disabled={sending} onclick={newChat} />
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
            {#if message.content}<span>{message.content}</span>{/if}
          </div>
        {/if}
      </article>
    {/each}

    {#if sending}
      <article class="agent-row assistant">
        <div class="agent-bubble assistant-copy streaming">
          {#if streamText}
            {@html renderMessage(streamText)}
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
      accept="image/*"
      multiple
      class="agent-file-input"
      onchange={onPickFiles}
    />
    {#if attachments.length}
      <div class="agent-attachments">
        {#each attachments as att, i}
          <div class="agent-attachment">
            <img src={imageSrc(att)} alt="添付画像" />
            <button type="button" class="agent-attachment-remove" title="削除" aria-label="画像を削除" onclick={() => removeAttachment(i)}>
              <Icon name="xmark" size={11} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
    <div class="agent-send-row">
      <div class="agent-composer-island">
        <button
          type="button"
          class="agent-attach-button"
          title="画像を添付"
          aria-label="画像を添付"
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
