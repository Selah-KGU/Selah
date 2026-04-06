<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { mailAuthState, cachedFetch, onCacheUpdate, invalidateCache, getCacheTimestamp } from "../stores";
  import { mailCheckSession, mailOpenLogin, mailFetchInbox, mailFetchMessage, mailFetchProfile } from "../api";
  import type { MailMessage, MailDetail } from "../api";
  import Icon from "../Icon.svelte";
  import { invoke } from "@tauri-apps/api/core";

  let loading = $state(true);
  let error = $state("");
  let messages = $state<MailMessage[]>([]);
  let selectedMessage = $state<MailDetail | null>(null);
  let loadingDetail = $state(false);
  let loadingMore = $state(false);
  let refreshing = $state(false);
  let page = $state(0);
  const PAGE_SIZE = 20;

  let unlistenLogin: (() => void) | null = null;

  // SWR: pick up background poll refreshes
  const unsubMail = onCacheUpdate<MailMessage[]>("mail_inbox", (fresh) => {
    if (fresh && !selectedMessage) {
      messages = fresh;
      lastFetchTs = getCacheTimestamp("mail_inbox");
    }
  });

  onMount(async () => {
    unlistenLogin = await listen<{ email: string; displayName: string }>("mail-login-success", async (event) => {
      mailAuthState.set({
        authenticated: true,
        email: event.payload.email,
        displayName: event.payload.displayName,
      });
      await loadInbox();
    });

    try {
      const status = await mailCheckSession();
      if (status.authenticated) {
        mailAuthState.set({ authenticated: true, email: status.email, displayName: status.display_name });
        // Fetch profile in background for display info
        mailFetchProfile().then(profile => {
          mailAuthState.set({
            authenticated: true,
            email: profile.mail || profile.userPrincipalName || "",
            displayName: profile.displayName || "",
          });
        }).catch(() => {});
        await loadInbox();
      } else {
        loading = false;
      }
    } catch {
      loading = false;
    }
  });

  onDestroy(() => { unlistenLogin?.(); unsubMail(); stopTick(); });

  async function loadInbox() {
    loading = messages.length === 0;
    refreshing = messages.length > 0;
    error = "";
    try {
      const fetcher = () => mailFetchInbox(PAGE_SIZE, 0);
      messages = await cachedFetch("mail_inbox", fetcher);
      lastFetchTs = getCacheTimestamp("mail_inbox");
      startTick();
      page = 0;
    } catch (e: any) {
      error = typeof e === "string" ? e : e?.message ?? "読み込みに失敗しました";
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  async function manualRefresh() {
    invalidateCache("mail_inbox");
    await loadInbox();
  }

  async function loadMore() {
    loadingMore = true;
    try {
      const next = await mailFetchInbox(PAGE_SIZE, (page + 1) * PAGE_SIZE);
      if (next.length > 0) {
        messages = [...messages, ...next];
        page += 1;
      }
    } catch { /* ignore */ }
    loadingMore = false;
  }

  async function openMessage(msg: MailMessage) {
    loadingDetail = true;
    try {
      selectedMessage = await mailFetchMessage(msg.id);
      messages = messages.map(m => m.id === msg.id ? { ...m, isRead: true } : m);
    } catch (e: any) {
      error = typeof e === "string" ? e : e?.message ?? "メール読み込み失敗";
    }
    loadingDetail = false;
  }

  function closeDetail() {
    selectedMessage = null;
  }

  function formatDate(iso: string | null): string {
    if (!iso) return "";
    const d = new Date(iso);
    const now = new Date();
    const isToday = d.toDateString() === now.toDateString();
    if (isToday) {
      return d.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
    }
    const yesterday = new Date(now);
    yesterday.setDate(yesterday.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) {
      return "昨日";
    }
    const isThisYear = d.getFullYear() === now.getFullYear();
    if (isThisYear) {
      return `${d.getMonth() + 1}/${d.getDate()}`;
    }
    return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
  }

  function formatFullDate(iso: string | null): string {
    if (!iso) return "";
    const d = new Date(iso);
    return d.toLocaleDateString("ja-JP", {
      year: "numeric", month: "long", day: "numeric",
      weekday: "short", hour: "2-digit", minute: "2-digit",
    });
  }

  function senderName(msg: MailMessage): string {
    if (!msg.from?.emailAddress) return "不明";
    return msg.from.emailAddress.name || msg.from.emailAddress.address || "不明";
  }

  function senderInitial(msg: MailMessage): string {
    const name = senderName(msg);
    if (!name || name === "不明") return "?";
    // Use first char — handles CJK, latin, etc.
    return name.charAt(0).toUpperCase();
  }

  // Simple hash for avatar color
  function avatarColor(msg: MailMessage): string {
    const addr = msg.from?.emailAddress?.address || senderName(msg);
    let h = 0;
    for (let i = 0; i < addr.length; i++) h = ((h << 5) - h + addr.charCodeAt(i)) | 0;
    const hue = ((h % 360) + 360) % 360;
    return `hsl(${hue}, 45%, 55%)`;
  }

  let lastFetchTs = $state<number | null>(null);
  let nowTick = $state(Date.now());
  let tickInterval: ReturnType<typeof setInterval> | null = null;

  function startTick() {
    if (!tickInterval) tickInterval = setInterval(() => { nowTick = Date.now(); }, 30_000);
  }
  function stopTick() {
    if (tickInterval) { clearInterval(tickInterval); tickInterval = null; }
  }

  function updatedAgoText(): string {
    if (!lastFetchTs) return "";
    const diff = Math.floor((nowTick - lastFetchTs) / 1000);
    if (diff < 60) return "たった今更新";
    const mins = Math.floor(diff / 60);
    if (mins < 60) return `${mins}分前に更新`;
    const hrs = Math.floor(mins / 60);
    return `${hrs}時間前に更新`;
  }

  function handleBodyClick(e: MouseEvent) {
    const target = (e.target as HTMLElement)?.closest("a");
    if (!target) return;
    const href = target.getAttribute("href");
    if (!href) return;
    if (href.startsWith("http://") || href.startsWith("https://")) {
      e.preventDefault();
      e.stopPropagation();
      invoke("open_external_url", { url: href }).catch(console.error);
    }
  }

  let unreadCount = $derived(messages.filter(m => !m.isRead).length);
  let agoText = $derived(updatedAgoText());
</script>

<div class="mail-view">
  {#if !$mailAuthState.authenticated && !loading}
    <div class="login-prompt">
      <div class="login-icon-wrap">
        <svg width="56" height="56" viewBox="0 0 56 56" fill="none">
          <rect width="56" height="56" rx="14" fill="var(--accent)" opacity="0.08"/>
          <rect x="12" y="16" width="32" height="24" rx="3" stroke="var(--accent)" stroke-width="1.8" fill="none"/>
          <polyline points="12,16 28,31 44,16" stroke="var(--accent)" stroke-width="1.8" fill="none" stroke-linejoin="round"/>
        </svg>
      </div>
      <h2>Microsoft 365 メール</h2>
      <p>大学のメールアカウントに接続して、<br/>受信メールをこのアプリで確認できます。</p>
      <button class="login-btn" onclick={() => mailOpenLogin()}>
        <svg width="16" height="16" viewBox="0 0 21 21" style="margin-right:6px;">
          <rect width="10" height="10" fill="#f25022"/>
          <rect x="11" width="10" height="10" fill="#7fba00"/>
          <rect y="11" width="10" height="10" fill="#00a4ef"/>
          <rect x="11" y="11" width="10" height="10" fill="#ffb900"/>
        </svg>
        Microsoft でサインイン
      </button>
      <span class="login-hint">設定 → メール からクライアント ID を変更できます</span>
    </div>
  {:else if loading && messages.length === 0}
    <div class="center-state">
      <div class="spinner"></div>
      <span>メールを読み込み中...</span>
    </div>
  {:else if error && messages.length === 0}
    <div class="center-state">
      <p class="error-text">{error}</p>
      <button class="retry-btn" onclick={loadInbox}>再試行</button>
    </div>
  {:else}
    <!-- Header -->
    <div class="mail-header">
      {#if selectedMessage}
        <button class="back-btn" onclick={closeDetail}>
          <Icon name="chevron.left" size={16} />
          <span>戻る</span>
        </button>
      {:else}
        <div class="header-title">
          <h2>受信トレイ</h2>
          {#if unreadCount > 0}
            <span class="unread-badge">{unreadCount}</span>
          {/if}
          {#if agoText}
            <span class="updated-ago">{agoText}</span>
          {/if}
        </div>
      {/if}
      <div class="header-actions">
        {#if !selectedMessage}
          <button class="refresh-btn" class:spinning={refreshing} onclick={manualRefresh} disabled={refreshing} title="更新">
            <Icon name="arrow.clockwise" size={14} />
          </button>
        {/if}
      </div>
    </div>

    {#if selectedMessage}
      <!-- Detail -->
      <div class="mail-detail">
        {#if loadingDetail}
          <div class="center-state"><div class="spinner"></div></div>
        {:else}
          <div class="detail-header">
            <h3 class="detail-subject">{selectedMessage.subject || "(件名なし)"}</h3>
            <div class="detail-sender-row">
              <div class="detail-avatar" style="background:{avatarColor({ from: selectedMessage.from } as MailMessage)}">
                {(selectedMessage.from?.emailAddress?.name || "?").charAt(0).toUpperCase()}
              </div>
              <div class="detail-sender-info">
                <div class="detail-from-name">
                  {selectedMessage.from?.emailAddress?.name || selectedMessage.from?.emailAddress?.address || "不明"}
                </div>
                <div class="detail-from-email">
                  {selectedMessage.from?.emailAddress?.address || ""}
                </div>
              </div>
              <div class="detail-date">{formatFullDate(selectedMessage.receivedDateTime)}</div>
            </div>
            {#if selectedMessage.toRecipients?.length}
              <div class="detail-recipients">
                <span class="recipients-label">To</span>
                {#each selectedMessage.toRecipients as r}
                  <span class="recipient-chip">{r.emailAddress.name || r.emailAddress.address}</span>
                {/each}
              </div>
            {/if}
          </div>
          <div class="detail-body" onclick={handleBodyClick}>
            {#if selectedMessage.body?.content}
              {#if selectedMessage.body.contentType === "html"}
                {@html selectedMessage.body.content}
              {:else}
                <pre class="plain-text">{selectedMessage.body.content}</pre>
              {/if}
            {:else}
              <p class="empty-body">本文はありません</p>
            {/if}
          </div>
        {/if}
      </div>
    {:else}
      <!-- Message list -->
      <div class="mail-list">
        {#each messages as msg (msg.id)}
          <button
            class="mail-item"
            class:unread={!msg.isRead}
            onclick={() => openMessage(msg)}
          >
            <div class="avatar" style="background:{avatarColor(msg)}">
              {senderInitial(msg)}
            </div>
            <div class="mail-content">
              <div class="mail-top-row">
                <span class="mail-sender">{senderName(msg)}</span>
                <span class="mail-date">{formatDate(msg.receivedDateTime)}</span>
              </div>
              <div class="mail-subject">
                {msg.subject || "(件名なし)"}
                {#if msg.hasAttachments}
                  <Icon name="paperclip" size={11} />
                {/if}
              </div>
              <div class="mail-preview">{msg.bodyPreview || ""}</div>
            </div>
            {#if !msg.isRead}
              <div class="unread-dot"></div>
            {/if}
          </button>
        {/each}

        {#if messages.length >= (page + 1) * PAGE_SIZE}
          <button class="load-more-btn" onclick={loadMore} disabled={loadingMore}>
            {loadingMore ? "読み込み中..." : "さらに表示"}
          </button>
        {/if}

        {#if messages.length === 0 && !loading}
          <div class="center-state" style="height:300px">
            <Icon name="envelope" size={32} />
            <span style="margin-top:8px">受信メールはありません</span>
          </div>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .mail-view {
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  /* ---- Login prompt ---- */
  .login-prompt {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 10px;
    text-align: center;
    color: var(--text-secondary);
  }

  .login-icon-wrap { margin-bottom: 4px; }

  .login-prompt h2 {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .login-prompt p {
    font-size: 13px;
    max-width: 320px;
    line-height: 1.6;
    margin: 0;
    color: var(--text-tertiary);
  }

  .login-btn {
    display: flex;
    align-items: center;
    margin-top: 6px;
    padding: 9px 20px;
    background: var(--text-primary);
    color: var(--bg-primary, #fff);
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: opacity 0.15s;
  }

  .login-btn:hover { opacity: 0.85; }

  .login-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 4px;
  }

  /* ---- Shared states ---- */
  .center-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 200px;
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .error-text { color: var(--text-secondary); }

  .spinner {
    width: 18px;
    height: 18px;
    border: 2px solid var(--glass-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .retry-btn {
    padding: 6px 16px;
    background: var(--bg-hover);
    border: 0.5px solid var(--glass-border);
    border-radius: 6px;
    font-size: 12px;
    color: var(--text-primary);
    cursor: pointer;
  }

  /* ---- Header ---- */
  .mail-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 12px;
    flex-shrink: 0;
  }

  .header-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .mail-header h2 {
    font-size: 20px;
    font-weight: 700;
    margin: 0;
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .unread-badge {
    font-size: 11px;
    font-weight: 600;
    background: var(--accent);
    color: white;
    padding: 1px 7px;
    border-radius: 10px;
    min-width: 20px;
    text-align: center;
  }

  .updated-ago {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-tertiary);
    margin-left: 2px;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .refresh-btn {
    padding: 6px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all 0.15s;
  }

  .refresh-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .refresh-btn:disabled { cursor: default; }
  .refresh-btn.spinning :global(.icon) { animation: spin 0.8s linear infinite; }

  .back-btn {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 5px 10px 5px 6px;
    background: var(--bg-hover);
    border: 0.5px solid var(--glass-border);
    border-radius: 7px;
    font-size: 13px;
    color: var(--accent);
    cursor: pointer;
    transition: all 0.15s;
  }

  .back-btn:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  /* ---- Mail list ---- */
  .mail-list {
    flex: 1;
    overflow-y: auto;
    margin: 0 -24px;
    padding: 0 24px;
  }

  .mail-item {
    display: flex;
    align-items: flex-start;
    width: 100%;
    padding: 14px 12px;
    background: transparent;
    border: none;
    border-bottom: 0.5px solid var(--glass-border);
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
    gap: 12px;
    border-radius: 0;
    position: relative;
  }

  .mail-item:first-child {
    border-top: 0.5px solid var(--glass-border);
  }

  .mail-item:hover {
    background: var(--bg-hover);
  }

  .mail-item.unread .mail-sender { font-weight: 600; color: var(--text-primary); }
  .mail-item.unread .mail-subject { font-weight: 500; color: var(--text-primary); }

  .avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    font-weight: 600;
    color: white;
    margin-top: 1px;
  }

  .mail-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .mail-top-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }

  .mail-sender {
    font-size: 13px;
    font-weight: 400;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }

  .mail-date {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .mail-subject {
    font-size: 13px;
    font-weight: 400;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .mail-preview {
    font-size: 12px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.4;
  }

  .unread-dot {
    position: absolute;
    left: 4px;
    top: 50%;
    transform: translateY(-50%);
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }

  .load-more-btn {
    width: 100%;
    padding: 14px;
    background: transparent;
    border: none;
    font-size: 13px;
    color: var(--accent);
    cursor: pointer;
    transition: background 0.15s;
  }

  .load-more-btn:hover { background: var(--bg-hover); }
  .load-more-btn:disabled { color: var(--text-tertiary); cursor: default; }

  /* ---- Mail detail ---- */
  .mail-detail {
    flex: 1;
    overflow-y: auto;
    margin: 0 -24px;
    padding: 0 24px;
  }

  .detail-header {
    padding-bottom: 16px;
    border-bottom: 0.5px solid var(--glass-border);
  }

  .detail-subject {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0 0 14px;
    line-height: 1.35;
    letter-spacing: -0.02em;
  }

  .detail-sender-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .detail-avatar {
    width: 34px;
    height: 34px;
    border-radius: 50%;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    font-weight: 600;
    color: white;
  }

  .detail-sender-info {
    flex: 1;
    min-width: 0;
  }

  .detail-from-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .detail-from-email {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .detail-date {
    font-size: 11px;
    color: var(--text-tertiary);
    flex-shrink: 0;
    text-align: right;
  }

  .detail-recipients {
    margin-top: 10px;
    font-size: 12px;
    color: var(--text-tertiary);
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .recipients-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .recipient-chip {
    background: var(--bg-hover);
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 11px;
    color: var(--text-secondary);
  }

  .detail-body {
    padding: 20px 0;
    font-size: 14px;
    line-height: 1.7;
    color: var(--text-primary);
    word-break: break-word;
  }

  .detail-body :global(img) { max-width: 100%; height: auto; }
  .detail-body :global(a) { color: var(--accent); }

  .detail-body :global(table) {
    border-collapse: collapse;
    max-width: 100%;
    overflow-x: auto;
    display: block;
  }

  .detail-body :global(td),
  .detail-body :global(th) {
    border: 1px solid var(--glass-border);
    padding: 4px 8px;
    font-size: 13px;
  }

  .plain-text {
    white-space: pre-wrap;
    font-family: inherit;
    font-size: 14px;
    margin: 0;
  }

  .empty-body {
    color: var(--text-tertiary);
    font-style: italic;
  }
</style>
