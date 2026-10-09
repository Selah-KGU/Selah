<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onDestroy, onMount } from "svelte";
  import selahLogoUrl from "../assets/logo.png";
  import Icon, { type IconName } from "./Icon.svelte";
  import { listBookmarks, toggleBookmark } from "./bookmarks";
  import { tabFaviconUrl } from "./documentTabs";
  import { normalizeEffectiveTheme } from "./themePreference";
  import { acquireResourceGroup, ResourceScope } from "./resourceScope";
  import { LatestViewRead } from "./latestViewRead";
  import { createCacheSyncQueue } from "./cacheSyncQueue";

  interface DocumentTabControl {
    id: string;
    label: string;
    action: string;
    icon?: string | null;
    value?: string | null;
    group?: string | null;
    primary?: boolean;
    active?: boolean;
    disabled?: boolean;
    tone?: string | null;
    payload?: unknown;
    // Show an on/off status dot (theme accent when on, grey when off) instead of
    // the filled active background.
    indicator?: boolean;
    indicatorOn?: boolean;
  }

  interface DocumentTab {
    id: string;
    label: string;
    target: string;
    title: string;
    url: string;
    type: string;
    active: boolean;
    loading?: boolean;
    controls: DocumentTabControl[];
    splitRatios?: number[];
    reopen?: Record<string, unknown> | null;
  }

  interface DocumentTabTitleHint {
    owner?: string;
    target?: string;
    title?: string;
  }

  function readParam(name: string): string {
    const search = new URLSearchParams(window.location.search);
    const fromSearch = search.get(name);
    if (fromSearch) return fromSearch;
    const rawHash = window.location.hash.startsWith("#") ? window.location.hash.slice(1) : window.location.hash;
    return new URLSearchParams(rawHash).get(name) || "";
  }

  const owner = readParam("owner") || "document-tabs";
  const appWindow = getCurrentWindow();
  const isWindows = navigator.userAgent.includes("Windows");

  function minimizeWindow() { void appWindow.minimize(); }
  function toggleMaximize() { void appWindow.toggleMaximize(); }
  function closeWindow() { void appWindow.close(); }

  let tabs = $state<DocumentTab[]>([]);
  let busy = $state(false);
  let error = $state("");
  let copied = $state(false);
  let agentOpen = $state(false);
  const resources = new ResourceScope();
  let releasePoll: (() => void) | null = null;
  let releaseCopyTimer: (() => void) | null = null;
  let copiedTabId: string | undefined;
  let copiedUrl = "";
  let dragStart: { x: number; y: number } | null = null;
  let draggingWindow = false;
  let suppressNextTabClick = false;
  let dragTabId = $state<string | null>(null);
  let dragPointerId: number | null = null;
  let dragStartX = 0;
  let dragActive = $state(false);
  let dragDx = $state(0);
  let dragFromIndex = 0;
  let dragToIndex = $state(0);
  let tabUnit = 0;
  let titleHints = $state<Record<string, string>>({});
  let filesSearch = $state("");
  let releaseFilesSearch: (() => void) | null = null;
  let filesSearchTabId = "";
  let address = $state("");
  let addressEl = $state<HTMLInputElement | null>(null);
  let editingAddress = $state(false);
  let addressSyncedKey = "";
  let navigationVersion = 0;

  const defaultReaderControls: DocumentTabControl[] = [
    { id: "toc", label: "目次", action: "reader.toggleToc", icon: "sidebar", group: "view" },
    { id: "font-down", label: "縮小", action: "reader.fontDown", icon: "minus", group: "font" },
    { id: "font", label: "文字", action: "reader.noop", icon: "textformat", value: "14px", disabled: true, group: "font" },
    { id: "font-up", label: "拡大", action: "reader.fontUp", icon: "plus", group: "font" },
    { id: "edit", label: "編集", action: "reader.edit", icon: "pencil", group: "edit" },
    { id: "share", label: "共有", action: "reader.share", icon: "square.and.arrow.up", group: "file" },
    { id: "reveal", label: "Finder", action: "reader.reveal", icon: "folder.open", group: "file" },
    { id: "external", label: "外部", action: "reader.external", icon: "arrow.up.right.square", group: "file" },
  ];

  const activeTab = $derived(tabs.find((tab) => tab.active) || null);
  const activeControls = $derived(activeTab?.controls || []);
  const detailControls = $derived(dedupeControls(activeControls));
  const readerControls = $derived(dedupeControls(activeControls.length ? activeControls : defaultReaderControls));
  const activeTitleHint = $derived(activeTab ? titleHints[activeTab.target] || "" : "");
  const isHomeTab = $derived(activeTab?.type === "home");
  const showFallbackDetailLabel = $derived(
    !!activeTab && !["detail", "kwic", "kgc"].includes(activeTab.type)
  );

  // ── Favicons ──
  let faviconFailed = $state<Set<string>>(new Set());

  function onFaviconError(src: string): void {
    if (!src || faviconFailed.has(src)) return;
    faviconFailed = new Set(faviconFailed).add(src);
  }

  // ── Bookmarks (page favorites) ──
  let bookmarkIds = $state<Set<string>>(new Set());

  interface BookmarkDescriptor {
    id: string;
    title: string;
    spec: Record<string, unknown>;
  }

  function bookmarkDescriptor(tab: DocumentTab | null): BookmarkDescriptor | null {
    if (!tab) return null;
    if (tab.type === "browser") {
      if (!tab.url || tab.url.includes("surface=")) return null;
      return { id: tab.url, title: tab.title || tab.url, spec: { type: "browser", url: tab.url } };
    }
    const reopen = tab.reopen;
    if (reopen && typeof reopen === "object") {
      const kind = String(reopen.type || "");
      const id =
        kind === "reader" ? `reader:${reopen.path}` :
        kind === "detail" ? `detail:${reopen.params}` :
        `${kind}:${tab.id}`;
      return { id, title: tab.title, spec: { ...reopen, title: tab.title } };
    }
    return null;
  }

  const activeBookmark = $derived(bookmarkDescriptor(activeTab));
  const isFav = $derived(!!activeBookmark && bookmarkIds.has(activeBookmark.id));

  function refreshBookmarkIds(): void {
    bookmarkIds = new Set(listBookmarks().map((b) => b.id));
  }

  function toggleFav(): void {
    const desc = activeBookmark;
    if (!desc) return;
    toggleBookmark({ id: desc.id, title: desc.title, spec: desc.spec, addedAt: Date.now() });
    refreshBookmarkIds();
  }

  function shortTitle(tab: DocumentTab): string {
    const title = (tab.title || tab.url || tab.type || "Tab").trim();
    return title.length > 42 ? `${title.slice(0, 41)}…` : title;
  }

  function kindLabel(kind: string): string {
    if (kind === "reader") return "Markdown";
    if (kind === "browser") return "Web";
    if (kind === "kwic") return "KWIC";
    if (kind === "kgc") return "KGC";
    if (kind === "files") return "ファイル";
    if (kind === "home") return "新しいタブ";
    if (kind === "detective") return "なるほど";
    if (kind === "paper-check") return "論文チェック";
    return "LUNA";
  }

  function iconName(value: string | null | undefined): IconName {
    const allowed = new Set<IconName>([
      "chevron.left", "chevron.right", "arrow.clockwise", "arrow.up.right.square", "sidebar",
      "minus", "plus", "textformat", "pencil", "square.and.arrow.up", "folder.open", "doc",
      "save", "globe", "xmark", "book", "moon", "sparkles", "building.2", "video", "broadcast",
      "square.grid.2x2", "calendar", "arrow.triangle.swap", "arrow.down.circle",
      "exclamationmark.triangle", "trash", "folder", "pulse",
    ]);
    return allowed.has(value as IconName) ? value as IconName : "doc";
  }

  function dedupeControls(controls: DocumentTabControl[]): DocumentTabControl[] {
    const seen = new Set<string>();
    const out: DocumentTabControl[] = [];
    for (const control of controls || []) {
      const key = `${control.action}|${control.label}|${control.value || ""}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(control);
    }
    return out;
  }

  const tabsRead = new LatestViewRead(resources,
    () => invoke<DocumentTab[]>("document_tabs_list", { owner }),
    (nextTabs) => {
      // A drag can start while the native read is still pending.
      if (dragTabId !== null) return;
      tabs = nextTabs;
      pruneTitleHints(nextTabs);
      error = "";
    }, (e) => {
      error = String(e || "タブ一覧を取得できませんでした");
    });
  const queueTabsRead = createCacheSyncQueue(async (_keys, recoveryOnly) => {
    if (!resources.active || dragTabId !== null || (recoveryOnly && document.hidden)) return;
    await tabsRead.refresh().catch(() => {});
  });

  function refreshTabs(recoveryOnly = false): Promise<void> {
    if (!resources.active || dragTabId !== null || (recoveryOnly && document.hidden)) return Promise.resolve();
    return queueTabsRead(["tabs"], recoveryOnly);
  }

  function pruneTitleHints(nextTabs: DocumentTab[]): void {
    const liveTargets = new Set(nextTabs.map((tab) => tab.target));
    const next: Record<string, string> = {};
    for (const [target, title] of Object.entries(titleHints)) {
      if (liveTargets.has(target)) next[target] = title;
    }
    if (Object.keys(next).length !== Object.keys(titleHints).length) titleHints = next;
  }

  async function run(action: () => Promise<unknown>): Promise<void> {
    if (!resources.active || busy) return;
    busy = true;
    error = "";
    tabsRead.invalidate();
    try {
      await action();
      if (!resources.active) return;
      tabsRead.invalidate();
      await refreshTabs();
    } catch (e) {
      if (resources.active) error = String(e || "操作に失敗しました");
    } finally {
      if (resources.active) busy = false;
    }
  }

  function activate(id: string): void {
    void run(() => invoke("document_tabs_activate", { owner, id }));
  }

  function openNewTab(): void {
    void run(() => invoke("document_tabs_new_tab"));
  }

  function goMain(): void {
    void invoke("show_main_window").catch(() => {});
  }

  function activateFromClick(event: MouseEvent, id: string): void {
    if (suppressNextTabClick) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    activate(id);
  }

  function handleTabKeydown(event: KeyboardEvent, id: string): void {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    activate(id);
  }

  function closeTab(event: MouseEvent, id: string): void {
    event.stopPropagation();
    void run(() => invoke("document_tabs_close", { owner, id }));
  }

  // ── Tab reordering (pointer based, Chrome-style) ──
  // HTML5 drag-and-drop is unreliable in the WKWebView, so we drag with pointer
  // events and pure transforms: during the drag the array is untouched — the held
  // tab follows the cursor and the others slide aside to open a gap. The list is
  // committed once on release, so nothing jumps mid-drag.
  function tabShift(index: number): number {
    if (!dragActive || index === dragFromIndex) return 0;
    if (dragToIndex > dragFromIndex && index > dragFromIndex && index <= dragToIndex) return -tabUnit;
    if (dragToIndex < dragFromIndex && index >= dragToIndex && index < dragFromIndex) return tabUnit;
    return 0;
  }

  function onTabPointerDown(event: PointerEvent, id: string, index: number): void {
    if (event.button !== 0) return;
    tabsRead.invalidate();
    dragTabId = id;
    dragPointerId = event.pointerId;
    dragStartX = event.clientX;
    dragFromIndex = index;
    dragToIndex = index;
    dragDx = 0;
    dragActive = false;
  }

  function onTabPointerMove(event: PointerEvent): void {
    if (dragTabId === null || event.pointerId !== dragPointerId) return;
    if (!dragActive) {
      if (Math.abs(event.clientX - dragStartX) < 6) return;
      const el = event.currentTarget as HTMLElement;
      tabUnit = el.offsetWidth + 2; // tab width + the 2px flex gap
      dragActive = true;
      suppressNextTabClick = true;
      try {
        el.setPointerCapture(event.pointerId);
      } catch {}
    }
    dragDx = event.clientX - dragStartX;
    const moved = tabUnit > 0 ? Math.round(dragDx / tabUnit) : 0;
    dragToIndex = Math.max(0, Math.min(tabs.length - 1, dragFromIndex + moved));
  }

  function onTabPointerUp(event: PointerEvent): void {
    if (dragTabId === null || event.pointerId !== dragPointerId) return;
    try {
      (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    } catch {}
    const wasActive = dragActive;
    const from = dragFromIndex;
    const to = dragToIndex;
    dragTabId = null;
    dragPointerId = null;
    dragActive = false;
    dragDx = 0;
    if (wasActive && to !== from) {
      const next = tabs.slice();
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      tabsRead.invalidate();
      tabs = next;
      void invoke("document_tabs_reorder", { owner, ids: next.map((tab) => tab.id) }).catch(() => {});
    }
    if (wasActive) resources.schedule(() => (suppressNextTabClick = false), 120);
  }

  function onTabPointerCancel(event: PointerEvent): void {
    if (event.pointerId !== dragPointerId) return;
    dragTabId = null;
    dragPointerId = null;
    dragActive = false;
    dragDx = 0;
  }

  function sendControl(control: DocumentTabControl, event?: MouseEvent): void {
    if (!activeTab || control.disabled || control.action === "reader.noop") return;
    const tabId = activeTab.id;
    let payload: unknown = control.payload ?? null;
    // Menu-toggle controls live in this (88px) strip webview but their popup is
    // drawn in the full-size content webview below. Both webviews share the
    // window's left origin and width, so the button's clientX maps 1:1 — send it
    // so the surface can anchor the popup under the button.
    if (event && (control.action === "files.togglePeriodMenu" || control.action === "files.toggleSortMenu")) {
      const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
      payload = { x: rect.left };
    }
    void run(() => invoke("document_tabs_send_control", {
      owner,
      tabId,
      action: control.action,
      payload,
    }));
  }

  function sendFilesSearch(tabId: string, query: string): void {
    void run(() => invoke("document_tabs_send_control", {
      owner,
      tabId,
      action: "files.search",
      payload: query,
    }));
  }

  function cancelFilesSearch(): void {
    releaseFilesSearch?.();
    releaseFilesSearch = null;
    filesSearchTabId = "";
  }

  function onFilesSearchInput(): void {
    cancelFilesSearch();
    if (activeTab?.type !== "files") return;
    const tabId = activeTab.id;
    const query = filesSearch;
    filesSearchTabId = tabId;
    releaseFilesSearch = resources.schedule(() => {
      releaseFilesSearch = null;
      filesSearchTabId = "";
      if (activeTab?.id === tabId && activeTab.type === "files") sendFilesSearch(tabId, query);
    }, 140);
  }

  $effect(() => {
    // Reset the inline search field when leaving a files tab so a stale query
    // never lingers on an unrelated tab's toolbar.
    if (activeTab?.type !== "files") filesSearch = "";
    if (filesSearchTabId && filesSearchTabId !== activeTab?.id) cancelFilesSearch();
  });

  function isNoDragTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return !!target.closest("button, a, input, textarea, select, [data-no-window-drag], .tab");
  }

  function beginWindowDrag(event: PointerEvent): void {
    if (event.button !== 0 || isNoDragTarget(event.target)) {
      dragStart = null;
      return;
    }
    dragStart = { x: event.clientX, y: event.clientY };
    draggingWindow = false;
  }

  function updateWindowDrag(event: PointerEvent): void {
    if (!dragStart || draggingWindow) return;
    const dx = event.clientX - dragStart.x;
    const dy = event.clientY - dragStart.y;
    if ((dx * dx) + (dy * dy) < 16) return;
    draggingWindow = true;
    suppressNextTabClick = true;
    void appWindow.startDragging().catch(() => {});
  }

  function endWindowDrag(): void {
    dragStart = null;
    draggingWindow = false;
    if (suppressNextTabClick) {
      resources.schedule(() => {
        suppressNextTabClick = false;
      }, 120);
    }
  }

  function browserBack(): void {
    if (!activeTab) return;
    void invoke("browser_go_back", { target: activeTab.target }).catch(() => {});
  }

  function browserForward(): void {
    if (!activeTab) return;
    void invoke("browser_go_forward", { target: activeTab.target }).catch(() => {});
  }

  function browserReload(): void {
    if (!activeTab) return;
    void invoke("browser_reload", { target: activeTab.target }).catch(() => {});
  }

  async function browserExternal(): Promise<void> {
    const tab = activeTab;
    if (!resources.active || !tab || isHomeTab) return;
    let url = tab.url;
    try {
      url = await invoke<string>("browser_get_url", { target: tab.target });
    } catch {}
    if (resources.active && url && url !== "about:blank") {
      await invoke("open_in_system_browser", { url }).catch(() => {});
    }
  }

  function normalizeAddress(raw: string): string {
    const q = raw.trim();
    if (!q) return "";
    if (/^https?:\/\//i.test(q)) return q;
    // A bare domain/path with no spaces is treated as a URL; anything else
    // becomes a web search.
    if (!/\s/.test(q) && /^[^\s.]+\.[^\s.]{2,}.*$/.test(q)) return `https://${q}`;
    return `https://www.google.com/search?q=${encodeURIComponent(q)}`;
  }

  // Mirror the active browser tab's URL into the address field, but never while
  // the user is mid-edit. Switching tabs always resnaps to that tab's URL.
  $effect(() => {
    const tab = activeTab;
    const url = tab && tab.type !== "home" ? tab.url || "" : "";
    const key = tab ? tab.id : "";
    if (key !== addressSyncedKey) {
      navigationVersion += 1;
      addressSyncedKey = key;
      editingAddress = false;
      address = url;
      return;
    }
    if (!editingAddress) address = url;
  });

  function submitAddress(): void {
    const url = normalizeAddress(address);
    if (!resources.active || !url || !activeTab) return;
    const version = ++navigationVersion;
    const tabId = activeTab.id;
    error = "";
    editingAddress = false;
    if (isHomeTab) {
      const id = activeTab.id;
      void run(async () => {
        await invoke("open_external_url", { url });
        await invoke("document_tabs_close", { owner, id }).catch(() => {});
      });
      return;
    }
    addressEl?.blur();
    void invoke("browser_navigate", { target: activeTab.target, url }).catch((e) => {
      if (resources.active && version === navigationVersion && activeTab?.id === tabId) {
        error = String(e || "ページを開けませんでした");
      }
    });
  }

  async function copyActiveUrl(): Promise<void> {
    const tabId = activeTab?.id;
    const url = activeTab?.url || "";
    if (!resources.active || !url || isHomeTab) return;
    try {
      await navigator.clipboard.writeText(url);
      if (!resources.active || activeTab?.id !== tabId || activeTab.url !== url) return;
      copiedTabId = tabId;
      copiedUrl = url;
      copied = true;
      releaseCopyTimer?.();
      releaseCopyTimer = resources.schedule(() => {
        releaseCopyTimer = null;
        copied = false;
      }, 1100);
    } catch {}
  }

  $effect(() => {
    if (copied && (activeTab?.id !== copiedTabId || activeTab?.url !== copiedUrl)) {
      releaseCopyTimer?.();
      releaseCopyTimer = null;
      copied = false;
    }
  });

  function readStoredTheme(): string {
    try {
      return window.localStorage.getItem("selah-theme") || "";
    } catch {
      return "";
    }
  }

  function applyTheme(value: unknown): void {
    const theme = normalizeEffectiveTheme(value);
    if (theme) {
      document.documentElement.setAttribute("data-theme", theme);
      document.body.setAttribute("data-theme", theme);
    } else {
      document.documentElement.removeAttribute("data-theme");
      document.body.removeAttribute("data-theme");
    }
  }

  const themeRead = new LatestViewRead(resources,
    async () => {
      const stored = readStoredTheme();
      const appTheme = await invoke<string>("get_app_theme");
      return normalizeEffectiveTheme(appTheme) || stored;
    }, applyTheme, () => {
      const stored = readStoredTheme();
      if (stored) applyTheme(stored);
    });
  const queueThemeRead = createCacheSyncQueue(async () => {
    await themeRead.refresh().catch(() => {});
  });

  function syncThemeFromApp(): Promise<void> {
    if (!resources.active) return Promise.resolve();
    return queueThemeRead(["theme"]);
  }

  const agentRead = new LatestViewRead(resources,
    () => invoke<boolean>("document_tabs_agent_is_open"),
    (open) => { agentOpen = open; });

  function toggleAgent(): Promise<void> {
    return run(async () => {
      await invoke("document_tabs_open_agent", { owner });
      await agentRead.refresh();
    });
  }

  function updatePolling(): void {
    if (!resources.active || document.hidden) {
      releasePoll?.();
      releasePoll = null;
      return;
    }
    if (releasePoll) return;
    // Events provide real-time state. Keep the existing 3s recovery cadence,
    // but release the timer while hidden and catch up once on returning.
    releasePoll = resources.interval(() => { void refreshTabs(true); }, 3000);
    void refreshTabs(true);
  }

  async function initializeDocumentTabs(): Promise<void> {
    if (!resources.active) return;
    const storedTheme = readStoredTheme();
    if (storedTheme) applyTheme(storedTheme);
    await acquireResourceGroup(resources, [
      (group) => listen<string>("theme-changed", group.guard((event) => {
        themeRead.invalidate();
        applyTheme(event.payload);
      })),
      (group) => listen("app-theme-changed", group.guard(() => {
        themeRead.invalidate();
        void syncThemeFromApp();
      })),
      (group) => listen<{ owner: string; tabs: DocumentTab[] }>("document-tabs-changed", group.guard((event) => {
        if (!event.payload || event.payload.owner !== owner) return;
        tabsRead.invalidate();
        if (dragTabId !== null) return;
        tabs = event.payload.tabs || [];
        pruneTitleHints(tabs);
        error = "";
      })),
      (group) => listen<{ owner: string; open: boolean }>("document-tabs-agent-visibility", group.guard((event) => {
        if (!event.payload || event.payload.owner !== owner) return;
        agentRead.invalidate();
        agentOpen = !!event.payload.open;
      })),
      (group) => listen<DocumentTabTitleHint>("document-tab-title-hint", group.guard((event) => {
        if (!event.payload || event.payload.owner !== owner) return;
        const target = String(event.payload.target || "");
        if (!target) return;
        const title = String(event.payload.title || "").trim();
        const next = { ...titleHints };
        if (title) next[target] = title;
        else delete next[target];
        titleHints = next;
      })),
      (group) => listen("bookmarks-changed", group.guard(refreshBookmarkIds)),
    ]);
    if (!resources.active) return;
    refreshBookmarkIds();
    const visibilityHandler = resources.guard(updatePolling);
    document.addEventListener("visibilitychange", visibilityHandler);
    resources.own(() => document.removeEventListener("visibilitychange", visibilityHandler));
    updatePolling();
    await Promise.all([syncThemeFromApp(), agentRead.refresh().catch(() => {})]);
  }

  onMount(() => {
    void initializeDocumentTabs().catch((e) => {
      if (resources.active) error = String(e || "タブの通知を登録できませんでした");
    });
  });
  onDestroy(() => resources.dispose());
</script>

<svelte:head>
  <title>{activeTab?.title || "Copilot"}</title>
</svelte:head>

<main class="document-tabs" class:windows={isWindows} class:compact={activeTab?.type === "detective" || activeTab?.type === "paper-check"} data-tauri-drag-region aria-label="Document tabs">
  <section
    class="tab-row"
    data-tauri-drag-region
    role="group"
    aria-label="Window tabs"
    onpointerdown={beginWindowDrag}
    onpointermove={updateWindowDrag}
    onpointerup={endWindowDrag}
    onpointercancel={endWindowDrag}
    onpointerleave={endWindowDrag}
  >
    <div class="traffic-space" aria-hidden="true"></div>

    <div class="tabs" class:reordering={dragActive} role="tablist" aria-label="Open pages">
      <button class="new-tab home-btn" type="button" title="主画面に戻る" aria-label="主画面に戻る" onclick={goMain}>
        <Icon name="house" size={15} />
      </button>
      {#each tabs as tab, i (tab.id)}
        {@const fav = tab.type === "browser" ? tabFaviconUrl(tab.url) : ""}
        <div
          class="tab"
          class:active={tab.active}
          class:dragging={dragTabId === tab.id && dragActive}
          style:transform={dragActive
            ? tab.id === dragTabId
              ? `translateX(${dragDx}px) scale(1.03)`
              : `translateX(${tabShift(i)}px)`
            : "none"}
          role="tab"
          tabindex="0"
          aria-selected={tab.active}
          aria-label={`${kindLabel(tab.type)} ${tab.title || tab.url}`}
          title={tab.title || tab.url}
          onclick={(event) => activateFromClick(event, tab.id)}
          onkeydown={(event) => handleTabKeydown(event, tab.id)}
          onpointerdown={(event) => onTabPointerDown(event, tab.id, i)}
          onpointermove={onTabPointerMove}
          onpointerup={onTabPointerUp}
          onpointercancel={onTabPointerCancel}
        >
          {#if tab.loading}
            <span class="tab-spinner" aria-hidden="true"></span>
          {:else if tab.type === "detective"}
            <img class="favicon det-favicon" src="/naruhodo.png" alt="" aria-hidden="true" draggable="false" />
          {:else if fav && !faviconFailed.has(fav)}
            <img class="favicon" src={fav} alt="" aria-hidden="true" draggable="false" onerror={() => onFaviconError(fav)} />
          {:else}
            <span class="kind-dot" data-kind={tab.type} aria-hidden="true"></span>
          {/if}
          <span class="title">{shortTitle(tab)}</span>
          <button class="close" type="button" title="閉じる" aria-label="閉じる" onclick={(event) => closeTab(event, tab.id)}>
            <Icon name="xmark" size={13} />
          </button>
        </div>
      {/each}
      {#if tabs.length === 0}
        <div class="empty">タブなし</div>
      {/if}
      <button class="new-tab" type="button" title="新しいタブ" aria-label="新しいタブ" disabled={busy} onclick={openNewTab}>
        <Icon name="plus" size={15} />
      </button>
    </div>

    <div class="right-group">
      {#if error}
        <span class="error" title={error}>{error}</span>
      {/if}
      <button
        class="agent"
        class:active={agentOpen}
        type="button"
        title={agentOpen ? "エージェントを閉じる" : "エージェントを開く"}
        aria-label={agentOpen ? "エージェントを閉じる" : "エージェントを開く"}
        aria-pressed={agentOpen}
        disabled={busy}
        onclick={toggleAgent}
      >
        <img class="agent-logo" src={selahLogoUrl} alt="" aria-hidden="true" />
        <span>エージェント</span>
      </button>
      {#if isWindows}
        <div class="window-controls" data-no-window-drag>
          <button class="win-ctrl" type="button" onclick={minimizeWindow} title="最小化" aria-label="最小化">
            <svg width="10" height="1" viewBox="0 0 10 1"><line x1="0" y1="0.5" x2="10" y2="0.5" stroke="currentColor" stroke-width="1"/></svg>
          </button>
          <button class="win-ctrl" type="button" onclick={toggleMaximize} title="最大化" aria-label="最大化">
            <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" stroke-width="1" fill="none"/></svg>
          </button>
          <button class="win-ctrl win-close" type="button" onclick={closeWindow} title="閉じる" aria-label="閉じる">
            <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" stroke-width="1.2"/><line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" stroke-width="1.2"/></svg>
          </button>
        </div>
      {/if}
    </div>
  </section>

  <section class="control-row" data-tauri-drag-region aria-label="Tab controls">
    {#if activeTab && (activeTab.type === "browser" || activeTab.type === "home")}
      <div class="control-group compact">
        <button class="tool-btn icon-only" type="button" title="戻る" aria-label="戻る" disabled={isHomeTab} onclick={browserBack}>
          <Icon name="chevron.left" size={18} />
        </button>
        <button class="tool-btn icon-only" type="button" title="進む" aria-label="進む" disabled={isHomeTab} onclick={browserForward}>
          <Icon name="chevron.right" size={18} />
        </button>
        <button class="tool-btn icon-only" type="button" title="再読み込み" aria-label="再読み込み" onclick={browserReload}>
          <Icon name="arrow.clockwise" size={16} />
        </button>
      </div>
      <form class="url-pill" class:copied onsubmit={(event) => { event.preventDefault(); submitAddress(); }}>
        <span class="url-dot" aria-hidden="true"></span>
        <input
          bind:this={addressEl}
          class="address-input"
          type="text"
          inputmode="url"
          autocapitalize="off"
          autocomplete="off"
          autocorrect="off"
          spellcheck="false"
          placeholder={isHomeTab ? "検索またはURLを入力" : "URLを入力 または 検索"}
          title={isHomeTab ? "検索またはURLを入力" : activeTab.url}
          bind:value={address}
          onfocus={(event) => { editingAddress = true; event.currentTarget.select(); }}
          onblur={() => { editingAddress = false; }}
        />
        <button
          class="address-copy"
          type="button"
          title={copied ? "コピーしました" : "URLをコピー"}
          aria-label="URLをコピー"
          disabled={isHomeTab || !activeTab.url}
          onclick={copyActiveUrl}
        >
          {#if copied}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
          {/if}
        </button>
      </form>
      <div class="control-group compact">
        <button class="tool-btn" type="button" title="システムブラウザで開く" disabled={isHomeTab} onclick={browserExternal}>
          <Icon name="arrow.up.right.square" size={15} />
          <span>外部</span>
        </button>
      </div>
    {:else if activeTab?.type === "reader"}
      {#if activeTitleHint}
        <div class="detail-title-hint" title={activeTitleHint}>{activeTitleHint}</div>
      {/if}
      <div class="control-fill"></div>
      {#each readerControls as control (control.id)}
        <button
          class="tool-btn"
          class:primary={control.primary}
          class:active={control.active}
          class:value={!!control.value}
          type="button"
          title={control.label}
          disabled={busy || control.disabled}
          onclick={() => sendControl(control)}
        >
          <Icon name={iconName(control.icon)} size={15} />
          <span>{control.value || control.label}</span>
        </button>
      {/each}
    {:else if activeTab?.type === "files"}
      <label class="files-search" title="ファイル名やコース名で検索">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
        <input type="text" placeholder="ファイル名やコース名で検索…" bind:value={filesSearch} oninput={onFilesSearchInput} />
      </label>
      <div class="control-fill"></div>
      {#each detailControls as control (control.id)}
        <button
          class="tool-btn"
          class:primary={control.primary}
          class:active={control.active}
          class:value={!!control.value}
          type="button"
          title={control.label}
          disabled={busy || control.disabled}
          onclick={(e) => sendControl(control, e)}
        >
          <Icon name={iconName(control.icon)} size={15} />
          <span>{control.value || control.label}</span>
        </button>
      {/each}
    {:else if activeTab}
      {#if activeTitleHint}
        <div class="detail-title-hint" title={activeTitleHint}>{activeTitleHint}</div>
      {:else if !detailControls.length && showFallbackDetailLabel}
        <div class="detail-label">
          <span class="kind-dot" data-kind={activeTab.type} aria-hidden="true"></span>
          <span>{kindLabel(activeTab.type)}</span>
        </div>
      {/if}
      <div class="control-fill"></div>
      {#each detailControls as control (control.id)}
        <button
          class="tool-btn"
          class:primary={control.primary}
          class:active={control.active}
          data-tone={control.tone || undefined}
          type="button"
          title={control.label}
          disabled={busy || control.disabled}
          onclick={() => sendControl(control)}
        >
          <Icon name={iconName(control.icon)} size={15} />
          <span>{control.value || control.label}</span>
          {#if control.indicator}
            <span class="ctl-dot" class:on={control.indicatorOn} aria-hidden="true"></span>
          {/if}
        </button>
      {/each}
    {:else}
      <div class="detail-label muted">タブを開いてください</div>
    {/if}

    {#if activeBookmark}
      <button
        class="tool-btn icon-only bookmark"
        class:active={isFav}
        type="button"
        title={isFav ? "ブックマークを解除" : "このページをブックマーク"}
        aria-label={isFav ? "ブックマークを解除" : "このページをブックマーク"}
        aria-pressed={isFav}
        disabled={busy}
        onclick={toggleFav}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill={isFav ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" /></svg>
      </button>
    {/if}
  </section>
</main>

<style>
  .document-tabs {
    width: 100vw;
    height: 88px;
    min-height: 88px;
    overflow: hidden;
    display: grid;
    grid-template-rows: 46px 42px;
    color: #1d1d1f;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", "Noto Sans JP", sans-serif;
    background: linear-gradient(180deg, rgba(250,250,252,0.97), rgba(236,237,241,0.94));
    border-bottom: 0.5px solid rgba(0,0,0,0.1);
    backdrop-filter: blur(22px) saturate(1.45);
    -webkit-backdrop-filter: blur(22px) saturate(1.45);
    /* No -webkit-app-region here: WebView2 maps it to native HTCAPTION zones but
       fails to carve out no-drag children (window controls become unclickable).
       Dragging is handled by data-tauri-drag-region / the pointer handlers. */
    -webkit-user-select: none;
    user-select: none;
    box-sizing: border-box;
  }

  /* Detective tab: no toolbar row — collapse the strip to just the tab row.
     The Rust side resizes this webview to 46px to match (COMPACT_STRIP_HEIGHT). */
  .document-tabs.compact {
    height: 46px;
    min-height: 46px;
    grid-template-rows: 46px;
  }

  .document-tabs.compact .control-row {
    display: none;
  }

  .tab-row {
    min-width: 0;
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr) auto;
    align-items: center;
    gap: 7px;
    padding: 5px 10px 0 0;
    box-sizing: border-box;
  }

  .traffic-space {
    width: 72px;
    height: 100%;
  }

  /* Windows has no left traffic-lights: reclaim that 72px and tighten the row's
     right padding so the self-drawn window controls sit flush to the edge. */
  .document-tabs.windows .tab-row {
    grid-template-columns: 8px minmax(0, 1fr) auto;
    padding-right: 0;
  }

  .document-tabs.windows .traffic-space {
    width: 8px;
  }

  .document-tabs.windows .control-row {
    padding-left: 14px;
  }

  .window-controls {
    display: inline-flex;
    align-items: center;
    align-self: stretch;
    margin-left: 4px;
  }

  .win-ctrl {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 100%;
    min-height: 36px;
    padding: 0;
    border: none;
    border-radius: 0;
    background: transparent;
    color: #5d5d63;
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease;
  }

  .win-ctrl:hover {
    background: rgba(0, 0, 0, 0.07);
    color: #1d1d1f;
  }

  .win-ctrl.win-close:hover {
    background: #e81123;
    color: #fff;
  }

  .right-group,
  .control-group {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }

  button,
  input,
  .close,
  .url-pill,
  .address-input,
  .files-search,
  .tabs,
  .tab,
  .control-group {
    -webkit-app-region: no-drag;
  }

  .tabs {
    height: 36px;
    min-width: 0;
    display: flex;
    align-items: end;
    gap: 2px;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tabs::-webkit-scrollbar {
    display: none;
  }

  .tab {
    position: relative;
    height: 34px;
    width: clamp(132px, 18vw, 220px);
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 7px 0 10px;
    border: 0.5px solid transparent;
    border-radius: 9px 9px 0 0;
    background: transparent;
    color: #5d5d63;
    cursor: pointer;
    text-align: left;
    box-sizing: border-box;
    transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  }

  .tab::after {
    content: "";
    position: absolute;
    right: -2px;
    top: 8px;
    bottom: 7px;
    width: 0.5px;
    background: rgba(0,0,0,0.12);
  }

  .tab:hover:not(.active) {
    background: rgba(255,255,255,0.42);
    color: #1d1d1f;
  }

  .tab.active {
    background: rgba(255,255,255,0.94);
    border-color: rgba(0,0,0,0.1);
    border-bottom-color: rgba(255,255,255,0.94);
    color: #1d1d1f;
    box-shadow: 0 -1px 0 rgba(255,255,255,0.75) inset, 0 1px 6px rgba(0,0,0,0.06);
  }

  .tab.active::after,
  .tab:hover::after {
    opacity: 0;
  }

  /* While a drag is active, the other tabs glide aside; the held tab tracks the
     cursor with no transition so it never lags behind. */
  .tabs.reordering .tab:not(.dragging) {
    transition: transform 0.18s cubic-bezier(0.2, 0.8, 0.2, 1), background 0.15s ease, color 0.15s ease;
  }

  .tab.dragging {
    z-index: 5;
    cursor: grabbing;
    background: rgba(255,255,255,0.98);
    border-color: rgba(0,0,0,0.12);
    box-shadow: 0 8px 20px rgba(0,0,0,0.18), 0 2px 6px rgba(0,0,0,0.1);
  }

  .tab.dragging::after {
    opacity: 0;
  }

  .kind-dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: #64748b;
    flex: 0 0 auto;
    box-shadow: 0 0 0 2px rgba(100,116,139,0.12);
  }

  .favicon {
    width: 15px;
    height: 15px;
    flex: 0 0 auto;
    border-radius: 3px;
    object-fit: contain;
  }

  .det-favicon {
    width: 17px;
    height: 17px;
    border-radius: 50%;
    object-fit: cover;
    object-position: center 35%;
  }

  .tab-spinner {
    width: 13px;
    height: 13px;
    flex: 0 0 auto;
    border-radius: 999px;
    border: 1.6px solid rgba(0,0,0,0.18);
    border-top-color: #173b68;
    animation: tab-spin 0.7s linear infinite;
  }

  @keyframes tab-spin {
    to { transform: rotate(360deg); }
  }

  .kind-dot[data-kind="reader"] {
    background: #0f766e;
    box-shadow: 0 0 0 2px rgba(15,118,110,0.12);
  }

  .kind-dot[data-kind="browser"] {
    background: #2563eb;
    box-shadow: 0 0 0 2px rgba(37,99,235,0.12);
  }

  .kind-dot[data-kind="kwic"] {
    background: #b45309;
    box-shadow: 0 0 0 2px rgba(180,83,9,0.12);
  }

  .kind-dot[data-kind="kgc"] {
    background: #7c3aed;
    box-shadow: 0 0 0 2px rgba(124,58,237,0.12);
  }

  .kind-dot[data-kind="files"] {
    background: #0891b2;
    box-shadow: 0 0 0 2px rgba(8,145,178,0.12);
  }

  .kind-dot[data-kind="home"] {
    background: #475569;
    box-shadow: 0 0 0 2px rgba(71,85,105,0.12);
  }

  .kind-dot[data-kind="detective"] {
    background: #b91c1c;
    box-shadow: 0 0 0 2px rgba(185,28,28,0.12);
  }

  .kind-dot[data-kind="paper-check"] {
    background: #173b68;
    box-shadow: 0 0 0 2px rgba(23,59,104,0.12);
  }

  .new-tab {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    margin: 0 0 1px 4px;
    border: none;
    border-radius: 8px;
    padding: 0;
    display: inline-grid;
    place-items: center;
    background: transparent;
    color: #5d5d63;
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease, transform 0.1s ease;
  }

  .new-tab:hover:not(:disabled) {
    background: rgba(0,0,0,0.06);
    color: #1d1d1f;
  }

  .new-tab:active:not(:disabled) {
    transform: scale(0.94);
  }

  .new-tab:disabled {
    opacity: 0.4;
    cursor: default;
  }

  /* Back-to-main button at the very start of the tab strip. */
  .home-btn {
    margin: 0 6px 1px 0;
  }

  .title {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    font-weight: 650;
    line-height: 1;
  }

  .close {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: 999px;
    padding: 0;
    background: transparent;
    display: inline-grid;
    place-items: center;
    color: #86868b;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.12s ease, background 0.12s ease, color 0.12s ease;
  }

  .tab.active .close,
  .tab:hover .close,
  .close:focus-visible {
    opacity: 1;
  }

  .close:hover {
    background: rgba(0,0,0,0.08);
    color: #1d1d1f;
  }

  .empty {
    height: 30px;
    display: flex;
    align-items: center;
    padding: 0 10px;
    border-radius: 8px;
    color: #86868b;
    background: rgba(255,255,255,0.45);
    font-size: 12px;
  }

  .agent {
    height: 28px;
    min-width: 96px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 0 10px;
    color: #173b68;
    font-size: 12px;
    font-weight: 750;
    background: rgba(23,59,104,0.08);
    border: 0.5px solid rgba(23,59,104,0.18);
    cursor: pointer;
    transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease, transform 0.1s ease;
  }

  .agent:hover:not(:disabled) {
    background: rgba(23,59,104,0.13);
    border-color: rgba(23,59,104,0.28);
  }

  .agent.active {
    color: #fff;
    background: #173b68;
    border-color: #173b68;
  }

  .agent.active:hover:not(:disabled) {
    color: #fff;
    background: #244f86;
    border-color: #244f86;
  }

  .agent:active:not(:disabled) {
    transform: scale(0.96);
  }

  .agent:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .agent-logo {
    width: 18px;
    height: 18px;
    object-fit: contain;
    flex: 0 0 auto;
  }

  .error {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #b42318;
    font-size: 11px;
  }

  .control-row {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px 6px 78px;
    border-top: 0.5px solid rgba(0,0,0,0.06);
    box-sizing: border-box;
    overflow: hidden;
  }

  .control-fill {
    flex: 1 1 auto;
  }

  .control-group.compact {
    flex: 0 0 auto;
  }

  .tool-btn {
    height: 30px;
    min-width: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 0 9px;
    border-radius: 8px;
    border: 0.5px solid transparent;
    background: transparent;
    color: #4b5563;
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease, transform 0.1s ease;
  }

  .tool-btn.icon-only {
    width: 32px;
    padding: 0;
  }

  .tool-btn:hover:not(:disabled) {
    background: rgba(0,0,0,0.055);
    border-color: rgba(0,0,0,0.04);
    color: #111827;
  }

  .tool-btn:focus-visible {
    outline: none;
    background: rgba(0,0,0,0.06);
    box-shadow: 0 0 0 2px rgba(23,59,104,0.16);
  }

  .url-pill:focus-visible {
    outline: none;
    background: rgba(255,255,255,0.92);
    border-color: rgba(23,59,104,0.22);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.9), 0 0 0 2px rgba(23,59,104,0.16);
  }

  .tool-btn:active:not(:disabled) {
    transform: scale(0.96);
  }

  .tool-btn:disabled {
    opacity: 0.42;
    cursor: default;
  }

  .tool-btn.primary {
    background: transparent;
    border-color: transparent;
    color: #173b68;
    font-weight: 800;
  }

  .tool-btn.active {
    background: rgba(23,59,104,0.09);
    border-color: transparent;
    color: #173b68;
    box-shadow: none;
    font-weight: 760;
  }

  .tool-btn.active:hover:not(:disabled) {
    background: rgba(23,59,104,0.14);
    border-color: transparent;
    color: #173b68;
  }

  /* On/off status dot used instead of a filled active background. Off is a grey
     dot (the button's own text colour, faded); on is green in both themes. */
  .ctl-dot {
    flex: none;
    width: 5px;
    height: 5px;
    margin-left: 2px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.4;
    transition: background 0.15s ease, opacity 0.15s ease;
  }

  .ctl-dot.on {
    background: #34c759;
    opacity: 1;
  }

  :global([data-theme="dark"]) .ctl-dot.on {
    background: #30d158;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .ctl-dot.on {
      background: #30d158;
    }
  }

  .tool-btn.value {
    min-width: 58px;
  }

  /* Course tools keep their brand color (no filled background). */
  .tool-btn[data-tone="zoom"],
  .tool-btn[data-tone="zoom"]:hover:not(:disabled) {
    color: #2d8cff;
  }

  .tool-btn[data-tone="panopto"],
  .tool-btn[data-tone="panopto"]:hover:not(:disabled) {
    color: #00a651;
  }

  .tool-btn[data-tone="syllabus"],
  .tool-btn[data-tone="syllabus"]:hover:not(:disabled) {
    color: #eab308;
  }

  .tool-btn.bookmark {
    flex: 0 0 auto;
  }

  .tool-btn.bookmark.active {
    color: #d97706;
    background: rgba(217,119,6,0.12);
  }

  .tool-btn.bookmark.active:hover:not(:disabled) {
    color: #d97706;
    background: rgba(217,119,6,0.18);
  }

  .files-search {
    flex: 0 1 320px;
    min-width: 130px;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 11px;
    border-radius: 999px;
    border: 0.5px solid rgba(0,0,0,0.08);
    background: rgba(255,255,255,0.58);
    color: #5d5d63;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.72), 0 1px 2px rgba(0,0,0,0.025);
    transition: background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease, color 0.15s ease;
  }

  .files-search:focus-within {
    background: rgba(255,255,255,0.92);
    border-color: rgba(23,59,104,0.22);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.9), 0 0 0 2px rgba(23,59,104,0.16);
  }

  .files-search svg {
    flex: 0 0 auto;
    opacity: 0.6;
  }

  /* Override the global input{} styling from styles.css so the field reads as
     a flat search pill instead of a nested boxed input. */
  .files-search input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    margin: 0;
    padding: 0;
    border: none;
    border-radius: 0;
    background: none;
    box-shadow: none;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    outline: none;
    font: inherit;
    font-size: 12px;
    color: inherit;
  }

  .files-search input:focus {
    border: none;
    box-shadow: none;
  }

  .files-search input::placeholder {
    color: currentColor;
    opacity: 0.55;
  }

  .url-pill {
    flex: 1 1 auto;
    min-width: 80px;
    height: 30px;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    padding: 0 4px 0 12px;
    border-radius: 999px;
    border: 0.5px solid rgba(0,0,0,0.08);
    background: rgba(255,255,255,0.58);
    color: #5d5d63;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.72), 0 1px 2px rgba(0,0,0,0.025);
    font: inherit;
    font-size: 12px;
    text-align: left;
    transition: background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease, color 0.15s ease;
  }

  /* Override the global input{} styling so the address field reads as a flat
     pill rather than a nested boxed input. */
  .address-input {
    min-width: 0;
    height: 100%;
    margin: 0;
    padding: 0;
    border: none;
    border-radius: 0;
    background: none;
    box-shadow: none;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    outline: none;
    font: inherit;
    font-size: 12px;
    color: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .address-input:focus {
    border: none;
    box-shadow: none;
  }

  .address-input::placeholder {
    color: currentColor;
    opacity: 0.55;
  }

  .address-copy {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 999px;
    padding: 0;
    display: inline-grid;
    place-items: center;
    background: transparent;
    color: inherit;
    opacity: 0.6;
    cursor: pointer;
    transition: background 0.12s ease, opacity 0.12s ease, color 0.12s ease;
  }

  .address-copy:hover:not(:disabled) {
    background: rgba(0,0,0,0.07);
    opacity: 1;
  }

  .address-copy:disabled {
    opacity: 0;
    cursor: default;
    pointer-events: none;
  }

  .url-pill:focus-within {
    background: rgba(255,255,255,0.92);
    border-color: rgba(23,59,104,0.22);
    color: #1d1d1f;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.9), 0 0 0 2px rgba(23,59,104,0.16);
  }

  .url-pill:hover:not(:focus-within) {
    background: rgba(255,255,255,0.84);
    border-color: rgba(0,0,0,0.11);
    color: #1d1d1f;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.9), 0 1px 3px rgba(0,0,0,0.045);
  }

  .url-pill.copied .address-copy {
    color: #2563eb;
    opacity: 1;
  }

  .url-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.5;
  }

  .detail-label {
    height: 30px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
    border-radius: 8px;
    color: #5d5d63;
    background: transparent;
    font-size: 12px;
    font-weight: 700;
  }

  .detail-title-hint {
    height: 30px;
    max-width: min(58vw, 720px);
    min-width: 0;
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #1d1d1f;
    font-size: 13px;
    font-weight: 780;
    line-height: 30px;
  }

  .detail-label.muted {
    color: #86868b;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .document-tabs {
      color: #f5f5f7;
      background: linear-gradient(180deg, rgba(36,37,40,0.97), rgba(28,29,32,0.94));
      border-bottom-color: rgba(255,255,255,0.11);
    }

    :global(:root:not([data-theme="light"])) .control-row {
      border-top-color: rgba(255,255,255,0.08);
    }

    :global(:root:not([data-theme="light"])) .tab.active {
      background: rgba(58,58,62,0.96);
      border-color: rgba(255,255,255,0.12);
      border-bottom-color: rgba(58,58,62,0.96);
      color: #f5f5f7;
      box-shadow: 0 -1px 0 rgba(255,255,255,0.08) inset, 0 1px 7px rgba(0,0,0,0.18);
    }

    :global(:root:not([data-theme="light"])) .tab.dragging {
      background: rgba(64,64,68,0.99);
      border-color: rgba(255,255,255,0.16);
      color: #f5f5f7;
      box-shadow: 0 8px 20px rgba(0,0,0,0.45), 0 2px 6px rgba(0,0,0,0.3);
    }

    :global(:root:not([data-theme="light"])) .tab:hover:not(.active) {
      background: rgba(255,255,255,0.08);
      color: #f5f5f7;
    }

    :global(:root:not([data-theme="light"])) .tab::after {
      background: rgba(255,255,255,0.11);
    }

    :global(:root:not([data-theme="light"])) .close,
    :global(:root:not([data-theme="light"])) .empty {
      color: #98989d;
    }

    :global(:root:not([data-theme="light"])) .win-ctrl {
      color: #98989d;
    }

    :global(:root:not([data-theme="light"])) .win-ctrl:hover {
      background: rgba(255, 255, 255, 0.1);
      color: #f5f5f7;
    }

    :global(:root:not([data-theme="light"])) .win-ctrl.win-close:hover {
      background: #e81123;
      color: #fff;
    }

    :global(:root:not([data-theme="light"])) .new-tab {
      color: #98989d;
    }

    :global(:root:not([data-theme="light"])) .new-tab:hover:not(:disabled) {
      background: rgba(255,255,255,0.1);
      color: #f5f5f7;
    }

    :global(:root:not([data-theme="light"])) .tab-spinner {
      border-color: rgba(255,255,255,0.22);
      border-top-color: #cdddf5;
    }

    :global(:root:not([data-theme="light"])) .agent {
      background: rgba(251,191,36,0.13);
      border-color: rgba(251,191,36,0.24);
      color: #fbbf24;
    }

    :global(:root:not([data-theme="light"])) .agent:hover:not(:disabled) {
      background: rgba(251,191,36,0.2);
      border-color: rgba(251,191,36,0.34);
      color: #fbbf24;
    }

    :global(:root:not([data-theme="light"])) .agent.active {
      color: #1d1d1f;
      background: #fbbf24;
      border-color: #fbbf24;
    }

    :global(:root:not([data-theme="light"])) .empty {
      background: rgba(255,255,255,0.07);
      border-color: rgba(255,255,255,0.1);
      color: #c7c7cc;
    }

    :global(:root:not([data-theme="light"])) .tool-btn,
    :global(:root:not([data-theme="light"])) .detail-label {
      background: transparent;
      border-color: transparent;
      color: #c7c7cc;
    }


    :global(:root:not([data-theme="light"])) .url-pill,
    :global(:root:not([data-theme="light"])) .files-search {
      background: rgba(255,255,255,0.075);
      border-color: rgba(255,255,255,0.11);
      color: #d1d1d6;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.035), 0 1px 2px rgba(0,0,0,0.16);
    }

    :global(:root:not([data-theme="light"])) .detail-title-hint {
      color: #f5f5f7;
    }

    :global(:root:not([data-theme="light"])) .tool-btn:hover:not(:disabled) {
      background: rgba(255,255,255,0.11);
      border-color: rgba(255,255,255,0.06);
      color: #f5f5f7;
    }

    :global(:root:not([data-theme="light"])) .tool-btn[data-tone="zoom"],
    :global(:root:not([data-theme="light"])) .tool-btn[data-tone="zoom"]:hover:not(:disabled) {
      color: #4ba3ff;
    }

    :global(:root:not([data-theme="light"])) .tool-btn[data-tone="panopto"],
    :global(:root:not([data-theme="light"])) .tool-btn[data-tone="panopto"]:hover:not(:disabled) {
      color: #2ec27a;
    }

    :global(:root:not([data-theme="light"])) .tool-btn[data-tone="syllabus"],
    :global(:root:not([data-theme="light"])) .tool-btn[data-tone="syllabus"]:hover:not(:disabled) {
      color: #facc15;
    }

    :global(:root:not([data-theme="light"])) .url-pill:hover:not(:focus-within) {
      background: rgba(255,255,255,0.12);
      border-color: rgba(255,255,255,0.16);
      color: #f5f5f7;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.055), 0 1px 3px rgba(0,0,0,0.2);
    }

    :global(:root:not([data-theme="light"])) .tool-btn:focus-visible {
      background: rgba(255,255,255,0.12);
      box-shadow: 0 0 0 2px rgba(251,191,36,0.18);
    }

    :global(:root:not([data-theme="light"])) .url-pill:focus-within {
      background: rgba(255,255,255,0.13);
      border-color: rgba(251,191,36,0.24);
      color: #f5f5f7;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.055), 0 0 0 2px rgba(251,191,36,0.18);
    }

    :global(:root:not([data-theme="light"])) .address-copy:hover:not(:disabled) {
      background: rgba(255,255,255,0.14);
    }

    :global(:root:not([data-theme="light"])) .url-pill.copied .address-copy {
      color: #93c5fd;
    }

    :global(:root:not([data-theme="light"])) .tool-btn.primary {
      background: transparent;
      border-color: transparent;
      color: #fbbf24;
    }

    :global(:root:not([data-theme="light"])) .tool-btn.active {
      background: rgba(251,191,36,0.14);
      border-color: transparent;
      color: #fbbf24;
      box-shadow: none;
    }

    :global(:root:not([data-theme="light"])) .tool-btn.active:hover:not(:disabled) {
      background: rgba(251,191,36,0.2);
      border-color: transparent;
      color: #fbbf24;
    }

    :global(:root:not([data-theme="light"])) .tool-btn.bookmark.active,
    :global(:root:not([data-theme="light"])) .tool-btn.bookmark.active:hover:not(:disabled) {
      color: #fbbf24;
      background: rgba(251,191,36,0.16);
    }

    :global(:root:not([data-theme="light"])) .error {
      color: #ff9f92;
    }
  }

  :global([data-theme="dark"]) .document-tabs {
    color: #f5f5f7;
    background: linear-gradient(180deg, rgba(36,37,40,0.97), rgba(28,29,32,0.94));
    border-bottom-color: rgba(255,255,255,0.11);
  }

  :global([data-theme="dark"]) .control-row {
    border-top-color: rgba(255,255,255,0.08);
  }

  :global([data-theme="dark"]) .tab.active {
    background: rgba(58,58,62,0.96);
    border-color: rgba(255,255,255,0.12);
    border-bottom-color: rgba(58,58,62,0.96);
    color: #f5f5f7;
    box-shadow: 0 -1px 0 rgba(255,255,255,0.08) inset, 0 1px 7px rgba(0,0,0,0.18);
  }

  :global([data-theme="dark"]) .tab.dragging {
    background: rgba(64,64,68,0.99);
    border-color: rgba(255,255,255,0.16);
    color: #f5f5f7;
    box-shadow: 0 8px 20px rgba(0,0,0,0.45), 0 2px 6px rgba(0,0,0,0.3);
  }

  :global([data-theme="dark"]) .tab:hover:not(.active) {
    background: rgba(255,255,255,0.08);
    color: #f5f5f7;
  }

  :global([data-theme="dark"]) .tab::after {
    background: rgba(255,255,255,0.11);
  }

  :global([data-theme="dark"]) .close,
  :global([data-theme="dark"]) .empty {
    color: #98989d;
  }

  :global([data-theme="dark"]) .win-ctrl {
    color: #98989d;
  }

  :global([data-theme="dark"]) .win-ctrl:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #f5f5f7;
  }

  :global([data-theme="dark"]) .win-ctrl.win-close:hover {
    background: #e81123;
    color: #fff;
  }

  :global([data-theme="dark"]) .new-tab {
    color: #98989d;
  }

  :global([data-theme="dark"]) .new-tab:hover:not(:disabled) {
    background: rgba(255,255,255,0.1);
    color: #f5f5f7;
  }

  :global([data-theme="dark"]) .tab-spinner {
    border-color: rgba(255,255,255,0.22);
    border-top-color: #cdddf5;
  }

  :global([data-theme="dark"]) .agent {
    background: rgba(251,191,36,0.13);
    border-color: rgba(251,191,36,0.24);
    color: #fbbf24;
  }

  :global([data-theme="dark"]) .agent:hover:not(:disabled) {
    background: rgba(251,191,36,0.2);
    border-color: rgba(251,191,36,0.34);
    color: #fbbf24;
  }

  :global([data-theme="dark"]) .agent.active {
    color: #1d1d1f;
    background: #fbbf24;
    border-color: #fbbf24;
  }

  :global([data-theme="dark"]) .empty {
    background: rgba(255,255,255,0.07);
    border-color: rgba(255,255,255,0.1);
    color: #c7c7cc;
  }

  :global([data-theme="dark"]) .tool-btn,
  :global([data-theme="dark"]) .detail-label {
    background: transparent;
    border-color: transparent;
    color: #c7c7cc;
  }

  :global([data-theme="dark"]) .url-pill,
  :global([data-theme="dark"]) .files-search {
    background: rgba(255,255,255,0.075);
    border-color: rgba(255,255,255,0.11);
    color: #d1d1d6;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.035), 0 1px 2px rgba(0,0,0,0.16);
  }

  :global([data-theme="dark"]) .detail-title-hint {
    color: #f5f5f7;
  }

  :global([data-theme="dark"]) .tool-btn:hover:not(:disabled) {
    background: rgba(255,255,255,0.11);
    border-color: rgba(255,255,255,0.06);
    color: #f5f5f7;
  }

  :global([data-theme="dark"]) .tool-btn[data-tone="zoom"],
  :global([data-theme="dark"]) .tool-btn[data-tone="zoom"]:hover:not(:disabled) {
    color: #4ba3ff;
  }

  :global([data-theme="dark"]) .tool-btn[data-tone="panopto"],
  :global([data-theme="dark"]) .tool-btn[data-tone="panopto"]:hover:not(:disabled) {
    color: #2ec27a;
  }

  :global([data-theme="dark"]) .tool-btn[data-tone="syllabus"],
  :global([data-theme="dark"]) .tool-btn[data-tone="syllabus"]:hover:not(:disabled) {
    color: #facc15;
  }

  :global([data-theme="dark"]) .url-pill:hover:not(:focus-within) {
    background: rgba(255,255,255,0.12);
    border-color: rgba(255,255,255,0.16);
    color: #f5f5f7;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.055), 0 1px 3px rgba(0,0,0,0.2);
  }

  :global([data-theme="dark"]) .tool-btn:focus-visible {
    background: rgba(255,255,255,0.12);
    box-shadow: 0 0 0 2px rgba(251,191,36,0.18);
  }

  :global([data-theme="dark"]) .url-pill:focus-within {
    background: rgba(255,255,255,0.13);
    border-color: rgba(251,191,36,0.24);
    color: #f5f5f7;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.055), 0 0 0 2px rgba(251,191,36,0.18);
  }

  :global([data-theme="dark"]) .address-copy:hover:not(:disabled) {
    background: rgba(255,255,255,0.14);
  }

  :global([data-theme="dark"]) .url-pill.copied .address-copy {
    color: #93c5fd;
  }

  :global([data-theme="dark"]) .tool-btn.primary {
    background: transparent;
    border-color: transparent;
    color: #fbbf24;
  }

  :global([data-theme="dark"]) .tool-btn.active {
    background: rgba(251,191,36,0.14);
    border-color: transparent;
    color: #fbbf24;
    box-shadow: none;
  }

  :global([data-theme="dark"]) .tool-btn.active:hover:not(:disabled) {
    background: rgba(251,191,36,0.2);
    border-color: transparent;
    color: #fbbf24;
  }

  :global([data-theme="dark"]) .tool-btn.bookmark.active,
  :global([data-theme="dark"]) .tool-btn.bookmark.active:hover:not(:disabled) {
    color: #fbbf24;
    background: rgba(251,191,36,0.16);
  }

  :global([data-theme="dark"]) .error {
    color: #ff9f92;
  }

  @media (max-width: 760px) {
    .tab-row {
      grid-template-columns: 68px minmax(0, 1fr) auto;
      gap: 6px;
    }

    .tab {
      width: clamp(118px, 34vw, 180px);
    }

    .agent span {
      display: none;
    }

    .agent {
      min-width: 34px;
      padding: 0;
    }

    .control-row {
      padding-left: 72px;
    }

    .tool-btn span:not(.always-show) {
      display: none;
    }

    .tool-btn {
      width: 32px;
      padding: 0;
    }
  }
</style>
