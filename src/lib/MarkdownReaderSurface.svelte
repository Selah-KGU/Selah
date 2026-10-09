<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { emitTo, listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount, tick } from "svelte";
  import DOMPurify from "dompurify";
  import { marked } from "marked";
  import type { LiveWhiteboard } from "./api";
  import { applyAuxiliaryTheme, syncAuxiliaryTheme } from "./auxiliarySurfaceTheme";
  import MarkdownImageLightbox from "./MarkdownImageLightbox.svelte";
  import MarkdownWhiteboard from "./MarkdownWhiteboard.svelte";
  import { splitMarkdownWhiteboards } from "./markdownWhiteboards";
  import { acquireResourceGroup, ResourceScope } from "./resourceScope";
  import { createCacheSyncQueue } from "./cacheSyncQueue";

  interface MarkdownPayload {
    deliveryRevision: string;
    path?: string;
    filename?: string;
    markdown?: string;
    error?: string;
  }

  interface TocItem {
    id: string;
    level: number;
    text: string;
  }

  interface RenderedSegment {
    id: number;
    html?: string;
    board?: LiveWhiteboard;
  }

  interface ControlEvent {
    owner?: string;
    target?: string;
    tabId?: string;
    action: string;
    payload?: unknown;
  }

  function readParam(name: string): string {
    const search = new URLSearchParams(window.location.search);
    const fromSearch = search.get(name);
    if (fromSearch) return fromSearch;
    const rawHash = window.location.hash.startsWith("#") ? window.location.hash.slice(1) : window.location.hash;
    return new URLSearchParams(rawHash).get(name) || "";
  }

  const target = readParam("tabLabel");
  const owner = readParam("ownerLabel") || "document-tabs";
  const FONT_STEPS = [12, 13, 14, 15, 16, 18, 20];
  const FONT_KEY = "selah-md-font-size";
  const TOC_KEY = "selah-md-toc-open";

  let path = $state("");
  let filename = $state("Markdown");
  let markdown = $state("");
  let savedMarkdown = $state("");
  let renderedSegments = $state<RenderedSegment[]>([]);
  let error = $state("");
  let loading = $state(true);
  let editing = $state(false);
  let saving = $state(false);
  let fontSize = $state(14);
  let tocVisible = $state(false);
  let tocItems = $state<TocItem[]>([]);
  let activeHeading = $state("");
  let toastText = $state("");
  let toastError = $state(false);
  let editorValue = $state("");
  let lightboxImage = $state<{ src: string; alt: string } | null>(null);
  let docEl = $state<HTMLElement | null>(null);
  let scrollEl = $state<HTMLDivElement | null>(null);
  const resources = new ResourceScope();
  const initialResources = resources.fork();
  let cancelToast = () => {};
  let lastToolbarTitleHint = "";
  let renderSequence = 0;
  let contentVersion = 0;
  let deliveryRevision = -1n;
  let themeVersion = 0;
  const refreshTheme = createCacheSyncQueue(async () => {
    const version = ++themeVersion;
    await syncAuxiliaryTheme(() => resources.active && version === themeVersion);
  });

  const dirty = $derived(editing && editorValue !== savedMarkdown);

  function restorePrefs(): void {
    try {
      const storedFont = Number(localStorage.getItem(FONT_KEY) || "14");
      if (FONT_STEPS.includes(storedFont)) fontSize = storedFont;
      tocVisible = localStorage.getItem(TOC_KEY) === "1";
    } catch {}
  }

  function persistPrefs(): void {
    try {
      localStorage.setItem(FONT_KEY, String(fontSize));
      localStorage.setItem(TOC_KEY, tocVisible ? "1" : "0");
    } catch {}
  }

  function slugify(text: string, used: Map<string, number>): string {
    const base = text
      .trim()
      .toLowerCase()
      .replace(/[^\p{Letter}\p{Number}]+/gu, "-")
      .replace(/^-+|-+$/g, "") || "section";
    const count = used.get(base) || 0;
    used.set(base, count + 1);
    return count ? `${base}-${count + 1}` : base;
  }

  async function renderHtml(source: string, current: () => boolean): Promise<string> {
    const raw = await marked.parse(source || "");
    if (!current()) return "";
    return DOMPurify.sanitize(raw, {
      FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form"],
      ADD_ATTR: ["target", "rel"],
      FORBID_ATTR: ["onerror", "onload", "onclick", "onmouseover", "onfocus"],
    });
  }

  async function renderMarkdown(source: string): Promise<void> {
    if (!resources.active) return;
    const sequence = ++renderSequence;
    const current = () => resources.active && sequence === renderSequence;
    try {
      const segments: RenderedSegment[] = [];
      for (const block of splitMarkdownWhiteboards(source || "")) {
        if (!current()) return;
        if (block.board) segments.push({ id: segments.length, board: block.board });
        else segments.push({ id: segments.length, html: await renderHtml(block.source || "", current) });
      }
      if (!current()) return;
      renderedSegments = segments;
      await tick();
      if (!current()) return;
      if (scrollEl) scrollEl.scrollTop = 0;
      activeHeading = "";
      buildToc();
      wireLinks();
      wireImages();
      updateControls();
      updateToolbarTitleHint();
    } catch (e) {
      if (!current()) return;
      error = `Markdown表示失敗: ${String(e)}`;
      renderedSegments = [];
      tocItems = [];
      updateControls();
      emitToolbarTitleHint("");
    }
  }

  function buildToc(): void {
    if (!docEl) return;
    const used = new Map<string, number>();
    const items: TocItem[] = [];
    const headings = Array.from(docEl.querySelectorAll("h1,h2,h3,h4,h5,h6"));
    const minLevel = headings.reduce((current, heading) => Math.min(current, Number(heading.tagName.slice(1))), 6);
    headings.forEach((heading) => {
      const text = (heading.textContent || "").trim();
      if (!text) return;
      const id = slugify(text, used);
      heading.id = id;
      items.push({ id, level: Math.min(6, Number(heading.tagName.slice(1)) - minLevel + 1), text });
    });
    tocItems = items;
    if (!items.length) tocVisible = false;
    updateControls();
  }

  function wireLinks(): void {
    if (!docEl) return;
    docEl.querySelectorAll("a[href]").forEach((anchor) => {
      anchor.addEventListener("click", (event) => {
        event.preventDefault();
        const rawHref = anchor.getAttribute("href") || "";
        if (rawHref.startsWith("#")) {
          scrollToHeading(rawHref.slice(1));
          return;
        }
        const href = (anchor as HTMLAnchorElement).href;
        if (/^https?:/i.test(href)) invoke("open_in_system_browser", { url: href }).catch(() => {});
      });
    });
  }

  function wireImages(): void {
    if (!docEl) return;
    docEl.querySelectorAll<HTMLImageElement>("img").forEach((image) => {
      image.addEventListener("click", (event) => {
        if (image.closest("a,button")) return;
        event.stopPropagation();
        lightboxImage = { src: image.currentSrc || image.src, alt: image.alt || "" };
      });
    });
  }

  function applyPayload(payload: MarkdownPayload | null | undefined): void {
    if (!resources.active || !payload || typeof payload.deliveryRevision !== "string" || !/^[0-9]+$/.test(payload.deliveryRevision)) return;
    const revision = BigInt(payload.deliveryRevision);
    if (revision <= deliveryRevision || (path && payload.path && payload.path !== path)) return;
    deliveryRevision = revision;
    contentVersion += 1;
    renderSequence += 1;
    initialResources.dispose();
    loading = false;
    path = payload.path || path;
    filename = payload.filename || filename;
    document.title = filename || "Markdown";
    if (payload.error) {
      error = payload.error;
      markdown = "";
      savedMarkdown = "";
      renderedSegments = [];
      tocItems = [];
      updateControls();
      emitToolbarTitleHint("");
      return;
    }
    error = "";
    markdown = String(payload.markdown || "");
    savedMarkdown = markdown;
    if (!editing) editorValue = markdown;
    void renderMarkdown(markdown);
  }

  function showToast(message: string, isError = false): void {
    if (!resources.active) return;
    toastText = message;
    toastError = isError;
    cancelToast();
    cancelToast = resources.schedule(() => {
      toastText = "";
      toastError = false;
    }, 1800);
  }

  function setTocVisible(value: boolean): void {
    tocVisible = value && tocItems.length > 0;
    persistPrefs();
    updateControls();
  }

  function bumpFont(delta: number): void {
    const idx = FONT_STEPS.indexOf(fontSize);
    const next = FONT_STEPS[Math.min(FONT_STEPS.length - 1, Math.max(0, idx + delta))];
    if (!next || next === fontSize) return;
    fontSize = next;
    persistPrefs();
    updateControls();
  }

  function enterEdit(): void {
    editorValue = savedMarkdown;
    editing = true;
    updateControls();
    emitToolbarTitleHint("");
  }

  function cancelEdit(): void {
    if (dirty && !window.confirm("編集中の内容は破棄されます。よろしいですか?")) return;
    editorValue = savedMarkdown;
    editing = false;
    updateControls();
    void tick().then(updateToolbarTitleHint);
  }

  async function save(): Promise<boolean> {
    if (!resources.active || !path || !dirty || saving) return false;
    const writtenPath = path;
    const writtenContents = editorValue;
    const version = contentVersion;
    const current = () => resources.active && version === contentVersion && writtenPath === path;
    saving = true;
    updateControls();
    try {
      await invoke("write_markdown_file", { path: writtenPath, contents: writtenContents });
      if (!current()) return false;
      savedMarkdown = writtenContents;
      markdown = writtenContents;
      // Preserve text typed while disk IO was in flight, and its dirty state.
      if (editorValue === writtenContents) editing = false;
      await renderMarkdown(writtenContents);
      if (!current()) return false;
      showToast("保存しました");
      return true;
    } catch (e) {
      if (current()) showToast(`保存失敗: ${String(e)}`, true);
      return false;
    } finally {
      if (resources.active) {
        saving = false;
        updateControls();
      }
    }
  }

  function handleReaderKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    const mod = event.metaKey || event.ctrlKey;
    if (mod && event.key.toLowerCase() === "s") {
      event.preventDefault();
      if (editing && dirty) void save();
      return;
    }
    if (mod && event.key.toLowerCase() === "e") {
      event.preventDefault();
      if (editing) cancelEdit();
      else enterEdit();
      return;
    }
    if (mod && (event.key === "+" || event.key === "=")) {
      event.preventDefault();
      bumpFont(1);
      return;
    }
    if (mod && event.key === "-") {
      event.preventDefault();
      bumpFont(-1);
      return;
    }
    if (editing && event.key === "Escape") {
      event.preventDefault();
      cancelEdit();
    }
  }

  function handleBeforeUnload(event: BeforeUnloadEvent): void {
    if (!dirty) return;
    event.preventDefault();
    event.returnValue = "";
  }

  async function share(): Promise<void> {
    if (!resources.active || !path) return;
    const sharedPath = path;
    const version = contentVersion;
    if (dirty && !(await save())) return;
    if (!resources.active || dirty || version !== contentVersion || path !== sharedPath) return;
    await invoke("share_downloaded_file_native", { path: sharedPath }).catch((e) => showToast(`共有失敗: ${String(e)}`, true));
  }

  function reveal(): void {
    if (!path) return;
    invoke("luna_reveal_file", { path }).catch((e) => showToast(`Finder表示失敗: ${String(e)}`, true));
  }

  function openExternal(): void {
    if (!path) return;
    invoke("open_downloaded_file_external", { path }).catch((e) => showToast(`外部アプリ起動失敗: ${String(e)}`, true));
  }

  function updateActiveHeading(): void {
    if (!scrollEl || !docEl || !tocItems.length) return;
    const top = scrollEl.getBoundingClientRect().top + 18;
    let current = tocItems[0]?.id || "";
    for (const item of tocItems) {
      const heading = docEl.querySelector<HTMLElement>(`#${CSS.escape(item.id)}`);
      if (heading && heading.getBoundingClientRect().top <= top) current = item.id;
    }
    activeHeading = current;
  }

  function emitToolbarTitleHint(title: string): void {
    if (!resources.active || !target || title === lastToolbarTitleHint) return;
    lastToolbarTitleHint = title;
    emitTo("document-tabs-strip", "document-tab-title-hint", {
      owner,
      target,
      title,
    }).catch(() => {});
  }

  function readerTitleElement(): HTMLElement | null {
    return docEl?.querySelector<HTMLElement>("h1") || docEl?.querySelector<HTMLElement>("h1,h2,h3,h4,h5,h6") || null;
  }

  function updateToolbarTitleHint(): void {
    if (!scrollEl || !docEl || loading || error || editing) {
      emitToolbarTitleHint("");
      return;
    }
    const titleElement = readerTitleElement();
    if (!titleElement) {
      emitToolbarTitleHint("");
      return;
    }
    const title = String(titleElement.textContent || filename || "").trim();
    const titleRect = titleElement.getBoundingClientRect();
    const rootRect = scrollEl.getBoundingClientRect();
    const hiddenAboveToolbar = titleRect.bottom <= rootRect.top + 8;
    emitToolbarTitleHint(hiddenAboveToolbar ? title : "");
  }

  function handleReaderScroll(): void {
    updateActiveHeading();
    updateToolbarTitleHint();
  }

  function scrollToHeading(id: string): void {
    const heading = docEl?.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    if (!heading || !scrollEl) return;
    // Markdown segments and whiteboard wrappers can establish separate offset
    // parents. Convert viewport geometry to this scroll container's coordinates.
    const top = heading.getBoundingClientRect().top - scrollEl.getBoundingClientRect().top + scrollEl.scrollTop - 12;
    scrollEl.scrollTo({ top, behavior: "smooth" });
  }

  function updateControls(): void {
    if (!resources.active || !target) return;
    const controls = [
      { id: "toc", label: tocVisible ? "目次を隠す" : "目次", action: "reader.toggleToc", icon: "sidebar", active: tocVisible, disabled: !tocItems.length, group: "view" },
      { id: "font-down", label: "縮小", action: "reader.fontDown", icon: "minus", disabled: fontSize <= FONT_STEPS[0], group: "font" },
      { id: "font", label: "文字", action: "reader.noop", icon: "textformat", value: `${fontSize}px`, disabled: true, group: "font" },
      { id: "font-up", label: "拡大", action: "reader.fontUp", icon: "plus", disabled: fontSize >= FONT_STEPS[FONT_STEPS.length - 1], group: "font" },
      editing
        ? { id: "save", label: saving ? "保存中" : dirty ? "保存" : "保存済み", action: "reader.save", icon: "save", primary: true, disabled: saving || !dirty, group: "edit" }
        : { id: "edit", label: "編集", action: "reader.edit", icon: "pencil", disabled: !path || !!error, group: "edit" },
      ...(editing ? [{ id: "cancel", label: "取消", action: "reader.cancel", icon: "xmark", group: "edit" }] : []),
      { id: "share", label: "共有", action: "reader.share", icon: "square.and.arrow.up", disabled: !path || !!error, group: "file" },
      { id: "reveal", label: "Finder", action: "reader.reveal", icon: "folder.open", disabled: !path, group: "file" },
      { id: "external", label: "外部", action: "reader.external", icon: "arrow.up.right.square", disabled: !path, group: "file" },
    ];
    invoke("document_tabs_set_controls", { owner, target, controls }).catch(() => {});
  }

  $effect(() => {
    editing;
    dirty;
    saving;
    updateControls();
  });

  function isCurrentControlEvent(control: ControlEvent | undefined): boolean {
    return control?.owner === owner && control?.target === target;
  }

  function handleControl(event: { payload: ControlEvent }): void {
    const control = event.payload;
    if (!isCurrentControlEvent(control)) return;
    const action = control.action;
    if (action === "reader.toggleToc") setTocVisible(!tocVisible);
    else if (action === "reader.fontDown") bumpFont(-1);
    else if (action === "reader.fontUp") bumpFont(1);
    else if (action === "reader.edit") enterEdit();
    else if (action === "reader.cancel") cancelEdit();
    else if (action === "reader.save") void save();
    else if (action === "reader.share") void share();
    else if (action === "reader.reveal") reveal();
    else if (action === "reader.external") openExternal();
  }

  function waitInitialDelay(delay: number): Promise<boolean> {
    const wait = initialResources.fork();
    return new Promise((resolve) => {
      wait.own(() => resolve(false));
      wait.schedule(() => { resolve(true); wait.dispose(); }, delay);
    });
  }

  async function fetchInitialPayload(): Promise<void> {
    if (!target || !initialResources.active) return;
    const delays = [0, 300, 700, 1500];
    for (const delay of delays) {
      if (delay && !(await waitInitialDelay(delay))) return;
      if (!initialResources.active) return;
      const payload = await invoke<MarkdownPayload | null>("get_pending_markdown_payload", { label: target }).catch(() => null);
      if (!initialResources.active) return;
      if (payload) {
        applyPayload(payload);
        if (!initialResources.active) return;
      }
    }
    if (!initialResources.active) return;
    initialResources.dispose();
    loading = false;
    error = "Markdown payload was not delivered.";
    updateControls();
    emitToolbarTitleHint("");
  }

  async function initializeReader(): Promise<void> {
    if (!target) {
      loading = false;
      error = "Markdown reader target is missing.";
      return;
    }
    await acquireResourceGroup(resources, [
      group => listen<MarkdownPayload>("markdown-content", group.guard((event) => {
        const payload = event.payload;
        applyPayload(payload);
        // ACK is cheap, and matching revisions cannot release a newer reopen.
        if (typeof payload.deliveryRevision === "string" && /^[0-9]+$/.test(payload.deliveryRevision) && BigInt(payload.deliveryRevision) <= deliveryRevision) {
          void invoke("ack_markdown_payload", { label: target, deliveryRevision: payload.deliveryRevision }).catch(() => {});
        }
      }), { target }),
      group => listen<ControlEvent>("document-tab-control", group.guard(handleControl), { target }),
      group => listen<string>("theme-changed", group.guard((event) => {
        themeVersion += 1;
        applyAuxiliaryTheme(event.payload);
      })),
      group => listen("app-theme-changed", group.guard(() => { void refreshTheme(["theme"]); })),
    ]);
    if (!resources.active) return;
    updateControls();
    // Theme IO must not postpone document delivery or the startup fallback.
    void refreshTheme(["theme"]);
    await fetchInitialPayload();
  }

  onMount(() => {
    document.documentElement.setAttribute("data-aux-surface", "markdown-reader");
    document.body.setAttribute("data-aux-surface", "markdown-reader");
    restorePrefs();
    void initializeReader().catch((e) => {
      if (!resources.active) return;
      initialResources.dispose();
      loading = false;
      error = `Markdown初期化失敗: ${String(e)}`;
      updateControls();
    });
  });

  onDestroy(() => {
    resources.dispose();
    renderSequence += 1;
    document.documentElement.removeAttribute("data-aux-surface");
    document.body.removeAttribute("data-aux-surface");
    if (target) {
      if (lastToolbarTitleHint) {
        void emitTo("document-tabs-strip", "document-tab-title-hint", { owner, target, title: "" }).catch(() => {});
      }
      void invoke("document_tabs_set_controls", { owner, target, controls: [] }).catch(() => {});
    }
  });
</script>

<svelte:window onkeydown={handleReaderKeydown} onbeforeunload={handleBeforeUnload} />

<svelte:head>
  <title>{filename || "Markdown"}</title>
</svelte:head>

<main class="reader" style={`--md-font-size:${fontSize}px`}>
  {#if tocVisible && tocItems.length}
    <aside class="toc">
      <div class="toc-header">目次</div>
      {#each tocItems as item}
        <button class:active={item.id === activeHeading} class={`toc-item lv-${Math.min(item.level, 6)}`} type="button" onclick={() => scrollToHeading(item.id)}>
          {item.text}
        </button>
      {/each}
    </aside>
  {/if}

  <section class="main">
    {#if editing}
      <textarea
        class="editor"
        bind:value={editorValue}
        spellcheck="false"
        aria-label="Markdown editor"
      ></textarea>
    {:else}
      <div class="scroll" bind:this={scrollEl} onscroll={handleReaderScroll}>
        <article class="doc" bind:this={docEl}>
          {#if loading}
            <div class="doc-skeleton" aria-busy="true" aria-label="読み込み中">
              <div class="sk sk-title"></div>
              <div class="sk sk-line w95"></div>
              <div class="sk sk-line w100"></div>
              <div class="sk sk-line w82"></div>
              <div class="sk-gap"></div>
              <div class="sk sk-sub"></div>
              <div class="sk sk-line w100"></div>
              <div class="sk sk-line w88"></div>
              <div class="sk sk-line w93"></div>
              <div class="sk sk-line w70"></div>
              <div class="sk-gap"></div>
              <div class="sk sk-line w90"></div>
              <div class="sk sk-line w96"></div>
              <div class="sk sk-line w60"></div>
            </div>
          {:else if error}
            <div class="reader-error">{error}</div>
          {:else}
            {#each renderedSegments as segment (segment.id)}
              {#if segment.board}
                <MarkdownWhiteboard board={segment.board} />
              {:else}
                <div class="markdown-segment reveal">{@html segment.html || ""}</div>
              {/if}
            {/each}
          {/if}
        </article>
      </div>
    {/if}
  </section>

  {#if toastText}
    <div class:error={toastError} class="toast">{toastText}</div>
  {/if}

  {#if lightboxImage}
    <MarkdownImageLightbox src={lightboxImage.src} alt={lightboxImage.alt} onclose={() => lightboxImage = null} />
  {/if}
</main>

<style>
  .reader {
    --reader-bg: #ffffff;
    --reader-sidebar: #f5f6f8;
    --reader-sidebar-hover: #e9edf3;
    --reader-text: #202124;
    --reader-muted: #667085;
    --reader-faint: #98a2b3;
    --reader-border: rgba(15, 23, 42, 0.1);
    --reader-accent: #173b68;
    --reader-accent-soft: rgba(23, 59, 104, 0.09);
    --reader-code: #f4f6f8;
    --reader-table-head: #f8f9fb;
    --reader-error: #b42318;
    color-scheme: light;
    height: 100vh;
    display: flex;
    min-width: 0;
    overflow: hidden;
    background: var(--reader-bg);
    color: var(--reader-text);
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", "Noto Sans JP", sans-serif;
    -webkit-font-smoothing: antialiased;
  }

  :global(html[data-aux-surface="markdown-reader"]),
  :global(body[data-aux-surface="markdown-reader"]),
  :global(body[data-aux-surface="markdown-reader"] #app) {
    background: #ffffff;
  }

  :global(body[data-aux-surface="markdown-reader"] #app::before) {
    display: none;
  }

  :global([data-theme="light"]) .reader {
    --reader-bg: #ffffff;
    --reader-sidebar: #f5f6f8;
    --reader-sidebar-hover: #e9edf3;
    --reader-text: #202124;
    --reader-muted: #667085;
    --reader-faint: #98a2b3;
    --reader-border: rgba(15, 23, 42, 0.1);
    --reader-accent: #173b68;
    --reader-accent-soft: rgba(23, 59, 104, 0.09);
    --reader-code: #f4f6f8;
    --reader-table-head: #f8f9fb;
    --reader-error: #b42318;
    color-scheme: light;
  }

  :global(html[data-theme="dark"][data-aux-surface="markdown-reader"]),
  :global(body[data-theme="dark"][data-aux-surface="markdown-reader"]),
  :global(body[data-theme="dark"][data-aux-surface="markdown-reader"] #app) {
    background: #1c1c1e;
  }

  .toc {
    width: 220px;
    flex: 0 0 220px;
    overflow-y: auto;
    padding: 16px 7px 28px;
    border-right: 0.5px solid var(--reader-border);
    background: var(--reader-sidebar);
    box-sizing: border-box;
  }

  .toc-header {
    padding: 4px 10px 10px;
    color: var(--reader-faint);
    font-size: 10px;
    font-weight: 700;
  }

  .toc-item {
    width: 100%;
    border: none;
    border-radius: 6px;
    padding: 4px 10px;
    background: transparent;
    color: var(--reader-muted);
    font: inherit;
    font-size: 12px;
    line-height: 1.45;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }

  .toc-item:hover,
  .toc-item.active {
    background: var(--reader-sidebar-hover);
    color: var(--reader-accent);
  }

  .toc-item.active {
    font-weight: 750;
  }

  .toc-item.lv-2 { padding-left: 20px; }
  .toc-item.lv-3 { padding-left: 32px; font-size: 11.5px; }
  .toc-item.lv-4 { padding-left: 44px; font-size: 11.5px; color: var(--reader-faint); }
  .toc-item.lv-5,
  .toc-item.lv-6 { padding-left: 56px; font-size: 11px; color: var(--reader-faint); }

  .main {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
  }

  .scroll {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .doc {
    max-width: 780px;
    margin: 0 auto;
    padding: 30px 38px 64px;
    color: var(--reader-text);
    font-size: var(--md-font-size);
    line-height: 1.7;
    user-select: text;
  }

  .doc :global(h1),
  .doc :global(h2),
  .doc :global(h3),
  .doc :global(h4),
  .doc :global(h5),
  .doc :global(h6) {
    margin: 1.35em 0 0.5em;
    line-height: 1.3;
    scroll-margin-top: 16px;
  }

  .doc :global(h1) { font-size: 1.75em; padding-bottom: 0.3em; border-bottom: 0.5px solid var(--reader-border); }
  .doc :global(h2) { font-size: 1.36em; padding-bottom: 0.25em; border-bottom: 0.5px solid var(--reader-border); }
  .doc :global(h3) { font-size: 1.16em; }
  .doc :global(p) { margin: 0.72em 0; }
  .doc :global(a) { color: var(--reader-accent); text-decoration: none; }
  .doc :global(a:hover) { text-decoration: underline; }
  .doc :global(ul),
  .doc :global(ol) { padding-left: 1.6em; margin: 0.65em 0; }
  .doc :global(code) {
    background: var(--reader-code);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: "SF Mono", ui-monospace, Menlo, Consolas, monospace;
    font-size: 0.88em;
  }
  .doc :global(pre) {
    border: 0.5px solid var(--reader-border);
    background: var(--reader-code);
    padding: 12px 14px;
    border-radius: 8px;
    overflow: auto;
  }
  .doc :global(pre code) {
    background: transparent;
    padding: 0;
  }
  .doc :global(blockquote) {
    border-left: 3px solid color-mix(in srgb, var(--reader-accent) 28%, transparent);
    margin: 0.9em 0;
    padding: 0.2em 0.95em;
    color: var(--reader-muted);
  }
  .doc :global(table) {
    border-collapse: collapse;
    width: 100%;
    margin: 1em 0;
  }
  .doc :global(th),
  .doc :global(td) {
    border: 0.5px solid var(--reader-border);
    padding: 6px 8px;
  }
  .doc :global(th) {
    background: var(--reader-table-head);
    text-align: left;
  }
  .doc :global(hr) {
    border: 0;
    border-top: 0.5px solid var(--reader-border);
    margin: 1.5em 0;
  }
  .doc :global(img) {
    max-width: 100%;
    height: auto;
    border-radius: 6px;
    display: block;
    margin: 8px auto;
    cursor: zoom-in;
    transition: opacity 0.15s;
  }

  .doc :global(img:hover) {
    opacity: 0.88;
  }

  .editor {
    width: 100%;
    height: 100%;
    border: none;
    outline: none;
    resize: none;
    padding: 22px 26px 48px;
    box-sizing: border-box;
    background: var(--reader-bg);
    color: var(--reader-text);
    font: 13px/1.65 "SF Mono", ui-monospace, Menlo, Consolas, monospace;
    caret-color: var(--reader-accent);
    user-select: text;
  }

  /* Skeleton placeholder shaped like a document while the file loads. */
  .doc-skeleton {
    --sk-base: color-mix(in srgb, var(--reader-text) 8%, transparent);
    --sk-hi: color-mix(in srgb, var(--reader-text) 15%, transparent);
    display: flex;
    flex-direction: column;
    gap: 13px;
    padding-top: 6px;
    animation: sk-fade-in 0.25s ease both;
  }

  .sk {
    height: 15px;
    border-radius: 6px;
    background: linear-gradient(90deg, var(--sk-base) 25%, var(--sk-hi) 37%, var(--sk-base) 63%);
    background-size: 400% 100%;
    animation: sk-shimmer 1.4s ease-in-out infinite;
  }

  .sk-title { height: 30px; width: 62%; border-radius: 8px; margin-bottom: 4px; }
  .sk-sub { height: 21px; width: 40%; border-radius: 7px; margin-top: 2px; }
  .sk-gap { height: 8px; }

  .w60 { width: 60%; }
  .w70 { width: 70%; }
  .w82 { width: 82%; }
  .w88 { width: 88%; }
  .w90 { width: 90%; }
  .w93 { width: 93%; }
  .w95 { width: 95%; }
  .w96 { width: 96%; }
  .w100 { width: 100%; }

  @keyframes sk-shimmer {
    0% { background-position: 100% 0; }
    100% { background-position: 0 0; }
  }

  @keyframes sk-fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  /* Rendered segments ease in so the skeleton → content swap isn't a hard cut. */
  .markdown-segment.reveal {
    animation: md-reveal 0.32s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }

  @keyframes md-reveal {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @media (prefers-reduced-motion: reduce) {
    .sk { animation: none; background: var(--sk-base); }
    .doc-skeleton,
    .markdown-segment.reveal { animation: none; }
  }

  .reader-error {
    padding: 44px 24px;
    text-align: center;
    color: var(--reader-error);
  }

  .toast {
    position: fixed;
    right: 18px;
    bottom: 18px;
    z-index: 50;
    padding: 8px 12px;
    border-radius: 8px;
    color: #fff;
    background: rgba(29,29,31,0.82);
    font-size: 12px;
    font-weight: 700;
  }

  .toast.error {
    background: rgba(220,38,38,0.9);
  }

  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .reader {
      --reader-bg: #1c1c1e;
      --reader-sidebar: #242426;
      --reader-sidebar-hover: #333336;
      --reader-text: #f5f5f7;
      --reader-muted: #b4b4ba;
      --reader-faint: #85858b;
      --reader-border: rgba(255, 255, 255, 0.1);
      --reader-accent: #e6be32;
      --reader-accent-soft: rgba(230, 190, 50, 0.14);
      --reader-code: #29292c;
      --reader-table-head: #252527;
      --reader-error: #ff9f92;
      color-scheme: dark;
    }
  }

  :global([data-theme="dark"]) .reader {
    --reader-bg: #1c1c1e;
    --reader-sidebar: #242426;
    --reader-sidebar-hover: #333336;
    --reader-text: #f5f5f7;
    --reader-muted: #b4b4ba;
    --reader-faint: #85858b;
    --reader-border: rgba(255, 255, 255, 0.1);
    --reader-accent: #e6be32;
    --reader-accent-soft: rgba(230, 190, 50, 0.14);
    --reader-code: #29292c;
    --reader-table-head: #252527;
    --reader-error: #ff9f92;
    color-scheme: dark;
  }
</style>
