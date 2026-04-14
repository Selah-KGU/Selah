<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { authState, lunaAuthState, kwicAuthState, debugVisible, getTaskSnapshot, onTaskChange } from "./stores";
  import type { TaskInfo } from "./stores";
  import Icon from "./Icon.svelte";
  import { fetchPage, triggerRelogin } from "./api";
  import { nativeNotify } from "./notify";

  interface DebugInfo {
    app_version: string;
    tauri_version: string;
    auth_status: string;
    username: string;
    cookie_count: number;
    timestamp: string;
    os: string;
    arch: string;
  }

  interface PingResult {
    target: string;
    reachable: boolean;
    status_code: number;
    latency_ms: number;
    error: string;
  }

  interface LogEntry {
    time: string;
    level: "info" | "warn" | "error" | "debug";
    message: string;
  }

  interface SessionCheck {
    kg: { valid: boolean; username: string; checking: boolean };
    luna: { valid: boolean; checking: boolean };
    kwic: { valid: boolean; checking: boolean };
  }

  let activeSection = $state<"info" | "network" | "logs" | "tasks">("info");
  let debugInfo = $state<DebugInfo | null>(null);
  let pingResults = $state<PingResult[]>([]);
  let logEntries = $state<LogEntry[]>([]);
  let isPinging = $state(false);
  let consoleInput = $state("");
  let tasks = $state<TaskInfo[]>([]);

  // Session check state
  let sessionCheck = $state<SessionCheck>({
    kg: { valid: false, username: "", checking: false },
    luna: { valid: false, checking: false },
    kwic: { valid: false, checking: false },
  });
  let isValidatingSession = $state(false);

  // Browser state
  let browserPath = $state("/uniasv2/UnSSOLoginControl2");
  let browserHtml = $state("");
  let browserLoading = $state(false);
  let browserError = $state("");

  async function browserNavigate() {
    browserLoading = true;
    browserError = "";
    try {
      browserHtml = await fetchPage(browserPath);
      addLog("info", `ブラウザ: ${browserPath} を取得 (${browserHtml.length} bytes)`);
    } catch (e: any) {
      browserError = e?.message || "ページ取得に失敗しました";
      browserHtml = "";
      addLog("error", `ブラウザ: ${browserError}`);
    } finally {
      browserLoading = false;
    }
  }

  // Dragging state
  let panelX = $state(Math.round(window.innerWidth / 2 - 280));
  let panelY = $state(80);
  let dragging = $state(false);
  let dragOffsetX = 0;
  let dragOffsetY = 0;

  const targets = [
    { name: "KG Course", url: "https://kg-course.kwansei.ac.jp" },
    { name: "SSO/Okta", url: "https://sso.kwansei.ac.jp" },
    { name: "Luna", url: "https://luna.kwansei.ac.jp" },
  ];

  function now(): string {
    return new Date().toLocaleTimeString("ja-JP");
  }

  function addLog(level: LogEntry["level"], message: string) {
    logEntries = [...logEntries, { time: now(), level, message }];
    if (logEntries.length > 200) logEntries = logEntries.slice(-200);
  }

  onMount(() => {
    const onError = (e: ErrorEvent) => {
      addLog("error", `${e.message} @ ${e.filename}:${e.lineno}`);
    };
    const onRejection = (e: PromiseRejectionEvent) => {
      addLog("error", `Unhandled: ${e.reason?.message || e.reason}`);
    };
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);

    // Subscribe to task registry updates
    tasks = getTaskSnapshot();
    const unsubTasks = onTaskChange(() => { tasks = getTaskSnapshot(); });

    const origWarn = console.warn;
    const origError = console.error;
    console.warn = (...args: any[]) => {
      addLog("warn", args.join(" "));
      origWarn.apply(console, args);
    };
    console.error = (...args: any[]) => {
      addLog("error", args.join(" "));
      origError.apply(console, args);
    };

    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
      unsubTasks();
      console.warn = origWarn;
      console.error = origError;
    };
  });

  function handleKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === "D") {
      e.preventDefault();
      debugVisible.update((v) => !v);
    }
  }

  $effect(() => {
    if ($debugVisible && !debugInfo) {
      fetchDebugInfo();
      validateAllSessions();
    }
  });

  async function fetchDebugInfo() {
    try {
      debugInfo = await invoke<DebugInfo>("debug_info");
    } catch (e: any) {
      addLog("error", `デバッグ情報取得失敗: ${e}`);
    }
  }

  async function validateAllSessions() {
    isValidatingSession = true;
    // KG Session
    sessionCheck.kg.checking = true;
    try {
      const s = await invoke<{ valid: boolean; username: string; display_name: string; student_id: string }>("validate_session");
      sessionCheck.kg = { valid: s.valid, username: s.valid ? (s.student_id || s.username) : "", checking: false };
      addLog(s.valid ? "info" : "warn", `KG Session: ${s.valid ? "有効" : "無効"} ${s.valid ? s.student_id : ""}`);
    } catch (e: any) {
      sessionCheck.kg = { valid: false, username: "", checking: false };
      addLog("error", `KG Session check failed: ${e}`);
    }
    // Luna Session
    sessionCheck.luna.checking = true;
    try {
      const ok = await invoke<boolean>("luna_check_session");
      sessionCheck.luna = { valid: ok, checking: false };
      addLog(ok ? "info" : "warn", `Luna Session: ${ok ? "有効" : "無効"}`);
    } catch (e: any) {
      sessionCheck.luna = { valid: false, checking: false };
      addLog("error", `Luna Session check failed: ${e}`);
    }
    // KWIC Portal Session
    sessionCheck.kwic.checking = true;
    try {
      const ok = await invoke<boolean>("kwic_check_session");
      sessionCheck.kwic = { valid: ok, checking: false };
      addLog(ok ? "info" : "warn", `KWIC Session: ${ok ? "有効" : "無効"}`);
    } catch (e: any) {
      sessionCheck.kwic = { valid: false, checking: false };
      addLog("error", `KWIC Session check failed: ${e}`);
    }
    isValidatingSession = false;
  }

  async function handleRelogin() {
    addLog("info", "再ログインを開始...");
    try {
      await triggerRelogin();
      addLog("info", "再ログイン成功");
      await validateAllSessions();
    } catch (e: any) {
      addLog("error", `再ログイン失敗: ${e}`);
    }
  }

  async function runPing() {
    isPinging = true;
    pingResults = [];
    addLog("info", "ネットワーク診断を開始...");
    for (const target of targets) {
      try {
        const result = await invoke<PingResult>("debug_ping", { target: target.url });
        pingResults = [...pingResults, result];
        addLog(result.reachable ? "info" : "error",
          result.reachable
            ? `${target.name}: ${result.status_code} (${result.latency_ms}ms)`
            : `${target.name}: 到達不可 - ${result.error}`);
      } catch (e: any) {
        pingResults = [...pingResults, { target: target.url, reachable: false, status_code: 0, latency_ms: 0, error: String(e) }];
        addLog("error", `${target.name}: ${e}`);
      }
    }
    isPinging = false;
  }

  async function runConsoleCommand() {
    if (!consoleInput.trim()) return;
    const cmd = consoleInput.trim();
    consoleInput = "";
    addLog("debug", `> ${cmd}`);
    try {
      if (cmd === "info") await fetchDebugInfo();
      else if (cmd === "ping") await runPing();
      else if (cmd === "session") await validateAllSessions();
      else if (cmd === "relogin") await handleRelogin();
      else if (cmd === "clear") logEntries = [];
      else if (cmd === "luna") {
        const ok = await invoke<boolean>("luna_check_session");
        addLog("info", `Luna session: ${ok ? "active" : "inactive"}`);
      } else if (cmd.startsWith("luna-page ")) {
        const path = cmd.slice(10).trim();
        addLog("info", `Luna fetching ${path}...`);
        const html = await invoke<string>("luna_fetch_page", { path });
        const titleMatch = html.match(/<title>([^<]*)<\/title>/);
        const links = [...html.matchAll(/href="([^"]*?)"/g)].map(m => m[1]).filter(l => l.startsWith("/") && !l.startsWith("/css") && !l.startsWith("/js"));
        const uniqueLinks = [...new Set(links)].slice(0, 30);
        addLog("info", `Title: ${titleMatch?.[1] || "?"}\nSize: ${html.length}\nLinks:\n${uniqueLinks.join("\n")}`);
      } else if (cmd.startsWith("fetch ")) {
        const path = cmd.slice(6).trim();
        browserPath = path;
        await browserNavigate();
      } else if (cmd === "notif" || cmd === "notification") {
        await sendTestNotification();
      } else if (cmd === "help") addLog("info", "コマンド: info, ping, session, relogin, fetch <path>, luna, luna-page <path>, notif, clear, help");
      else addLog("warn", `不明なコマンド: ${cmd} (helpで一覧表示)`);
    } catch (e: any) { addLog("error", String(e)); }
  }

  // Notification test
  let notifTestMsg = $state("Selah テスト通知");

  async function sendTestNotification() {
    try {
      await nativeNotify("Selah", notifTestMsg);
      addLog("info", `テスト通知送信: "${notifTestMsg}"`);
    } catch (e: any) {
      addLog("error", `通知送信失敗: ${e}`);
    }
  }

  // Drag handlers
  function startDrag(e: MouseEvent) {
    dragging = true;
    dragOffsetX = e.clientX - panelX;
    dragOffsetY = e.clientY - panelY;
    e.preventDefault();
  }

  function onMouseMove(e: MouseEvent) {
    if (!dragging) return;
    panelX = Math.max(0, Math.min(window.innerWidth - 200, e.clientX - dragOffsetX));
    panelY = Math.max(0, Math.min(window.innerHeight - 100, e.clientY - dragOffsetY));
  }

  function onMouseUp() {
    dragging = false;
  }

  // Import pre-boot logs from index.html collector
  if (typeof window !== "undefined" && (window as any).__SELAH_PREBOOT_LOGS__) {
    const prebootLogs: { type: string; message: string; time: string }[] = (window as any).__SELAH_PREBOOT_LOGS__;
    const imported = prebootLogs.map(log => {
      const level = (["info", "warn", "error", "debug"].includes(log.type) ? log.type : "info") as LogEntry["level"];
      return { time: log.time, level, message: log.message };
    });
    logEntries = [...logEntries, ...imported];
    delete (window as any).__SELAH_PREBOOT_LOGS__;
  }
  addLog("info", "デバッグパネル初期化完了");
</script>

<svelte:window onkeydown={handleKeydown} onmousemove={onMouseMove} onmouseup={onMouseUp} />

{#if $debugVisible}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="debug-overlay" onclick={() => debugVisible.set(false)}></div>
  <div
    class="debug-window"
    style="left:{panelX}px; top:{panelY}px;"
    role="dialog"
    aria-label="Debug Console"
  >
    <!-- Window chrome -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="win-titlebar" onmousedown={startDrag}>
      <div class="win-dots">
        <button class="dot dot-close" onclick={() => debugVisible.set(false)} title="閉じる"></button>
        <span class="dot dot-min"></span>
        <span class="dot dot-max"></span>
      </div>
      <span class="win-title">デバッグコンソール</span>
      <span class="win-badge" class:badge-ok={$authState.authenticated} class:badge-ng={!$authState.authenticated}>
        {$authState.authenticated ? "認証済" : "未認証"}
      </span>
    </div>

    <!-- Tabs -->
    <div class="tab-bar">
      <button class:active={activeSection === "info"} onclick={() => { activeSection = "info"; fetchDebugInfo(); }}>
        情報・セッション
      </button>
      <button class:active={activeSection === "tasks"} onclick={() => { activeSection = "tasks"; tasks = getTaskSnapshot(); }}>
        タスク <span class="tab-count">({tasks.length})</span>
      </button>
      <button class:active={activeSection === "network"} onclick={() => activeSection = "network"}>
        通信・ブラウザ
      </button>
      <button class:active={activeSection === "logs"} onclick={() => activeSection = "logs"}>
        ログ <span class="tab-count">({logEntries.length})</span>
      </button>
    </div>

    <!-- Content -->
    <div class="win-body">
      {#if activeSection === "info"}
        <div class="section" style="animation: fade-in 0.2s ease;">
          <!-- App info -->
          <h4>アプリケーション</h4>
          {#if debugInfo}
            <div class="info-grid">
              <div class="info-row"><span class="info-key">Version</span><span class="info-val">{debugInfo.app_version}</span></div>
              <div class="info-row"><span class="info-key">Tauri</span><span class="info-val">{debugInfo.tauri_version}</span></div>
              <div class="info-row"><span class="info-key">OS / Arch</span><span class="info-val">{debugInfo.os} / {debugInfo.arch}</span></div>
              <div class="info-row"><span class="info-key">Cookies</span><span class="info-val">{debugInfo.cookie_count}</span></div>
              <div class="info-row"><span class="info-key">Time</span><span class="info-val mono">{debugInfo.timestamp}</span></div>
            </div>
          {:else}
            <p class="muted">読み込み中...</p>
          {/if}

          <h4>フロントエンド</h4>
          <div class="info-grid">
            <div class="info-row"><span class="info-key">URL</span><span class="info-val mono truncate">{typeof window !== "undefined" ? window.location.href : "-"}</span></div>
            <div class="info-row">
              <span class="info-key">IPC Bridge</span>
              <span class="info-val">
                <span class="dot-status"
                  class:ok={(typeof window !== "undefined") && !!(window as any).__TAURI_INTERNALS__}
                  class:ng={(typeof window !== "undefined") && !(window as any).__TAURI_INTERNALS__}
                ></span>
                {(typeof window !== "undefined") && (window as any).__TAURI_INTERNALS__ ? "接続済" : "未接続"}
              </span>
            </div>
            <div class="info-row">
              <span class="info-key">Store Auth</span>
              <span class="info-val">
                <span class="dot-status" class:ok={$authState.authenticated} class:ng={!$authState.authenticated}></span>
                {$authState.authenticated ? $authState.username : "未認証"}
              </span>
            </div>
            <div class="info-row">
              <span class="info-key">Store Luna</span>
              <span class="info-val">
                <span class="dot-status" class:ok={$lunaAuthState.authenticated} class:ng={!$lunaAuthState.authenticated}></span>
                {$lunaAuthState.authenticated ? "認証済" : "未認証"}
              </span>
            </div>
            <div class="info-row">
              <span class="info-key">Store KWIC</span>
              <span class="info-val">
                <span class="dot-status" class:ok={$kwicAuthState.authenticated} class:ng={!$kwicAuthState.authenticated}></span>
                {$kwicAuthState.authenticated ? "認証済" : "未認証"}
              </span>
            </div>
          </div>
        </div>

      {:else if activeSection === "tasks"}
        <div class="section" style="animation: fade-in 0.2s ease;">
          <div class="section-header">
            <h4>定期タスク</h4>
            <button class="sm-btn" onclick={() => { tasks = getTaskSnapshot(); }}>更新</button>
          </div>
          {#each ["volatile", "stable", "system"] as tier}
            {@const tierTasks = tasks.filter(t => t.tier === tier)}
            {#if tierTasks.length > 0}
              <div class="task-tier-label">
                {tier === "volatile" ? "高頻度 (5min)" : tier === "stable" ? "低頻度 (12h)" : "システム"}
              </div>
              <div class="info-grid" style="margin-bottom:8px;">
                {#each tierTasks as t}
                  <div class="info-row" title={t.key}>
                    <span class="info-key" style="width:140px">{t.label}</span>
                    <span class="info-val" style="flex:1; justify-content:space-between;">
                      <span style="display:flex;align-items:center;gap:4px;">
                        {#if t.running}
                          <span class="dot-status running"></span>
                          <span class="task-running">実行中</span>
                        {:else if t.lastOk === true}
                          <span class="dot-status ok"></span>
                          <span>成功</span>
                        {:else if t.lastOk === false}
                          <span class="dot-status ng"></span>
                          <span>失敗</span>
                        {:else}
                          <span class="dot-status"></span>
                          <span class="muted">未実行</span>
                        {/if}
                      </span>
                      <span class="mono" style="color:var(--text-tertiary);font-size:10px;">
                        {#if t.lastRunTs}
                          {Math.round((Date.now() - t.lastRunTs) / 1000)}s ago
                        {:else}
                          --
                        {/if}
                      </span>
                    </span>
                  </div>
                {/each}
              </div>
            {/if}
          {/each}
          {#if tasks.length === 0}
            <p class="muted">バックグラウンドポーリング未開始</p>
          {/if}
        </div>

      {:else if activeSection === "network"}
        <div class="section" style="animation: fade-in 0.2s ease;">
          <h4>ネットワーク診断</h4>
          <button class="action-btn" onclick={runPing} disabled={isPinging}>
            {isPinging ? "診断中..." : "接続テスト実行"}
          </button>
          {#if pingResults.length > 0}
            <div class="info-grid" style="margin-top:10px;">
              {#each pingResults as p}
                <div class="info-row">
                  <span class="info-key truncate">{p.target}</span>
                  <span class="info-val">
                    <span class="dot-status" class:ok={p.reachable} class:ng={!p.reachable}></span>
                    {p.reachable ? `${p.status_code} · ${p.latency_ms}ms` : `NG · ${p.error.slice(0, 60)}`}
                  </span>
                </div>
              {/each}
            </div>
          {/if}

          <h4>内部ブラウザ</h4>
          <div class="browser-url-bar">
            <span class="browser-prefix">kg-course.kwansei.ac.jp</span>
            <input
              type="text"
              bind:value={browserPath}
              placeholder="/path"
              onkeydown={(e) => e.key === "Enter" && browserNavigate()}
            />
            <button class="action-btn" style="margin-top:0;" onclick={browserNavigate} disabled={browserLoading}>
              {browserLoading ? "..." : "Go"}
            </button>
          </div>
          {#if browserError}
            <div class="browser-error">{browserError}</div>
          {/if}
          {#if browserHtml}
            <div class="log-scroll" style="max-height:300px;">
              <pre class="browser-pre">{browserHtml}</pre>
            </div>
          {:else if !browserLoading}
            <p class="muted">パスを入力して Go を押してページを取得</p>
          {/if}
        </div>

      {:else if activeSection === "logs"}
        <div class="section" style="animation: fade-in 0.2s ease;">
          <div class="section-header">
            <h4>ログ <span class="count">({logEntries.length})</span></h4>
            <button class="sm-btn" onclick={() => logEntries = []}>クリア</button>
          </div>
          <div class="log-scroll">
            {#each logEntries as entry}
              <div class="log-line log-{entry.level}">
                <span class="lt">{entry.time}</span>
                <span class="ll">[{entry.level.toUpperCase()}]</span>
                <span class="lm">{entry.message}</span>
              </div>
            {/each}
          </div>
          <div class="cli-input">
            <span class="cli-prompt">&gt;</span>
            <input
              type="text"
              bind:value={consoleInput}
              onkeydown={(e) => { if (e.key === "Enter") runConsoleCommand(); }}
              placeholder="コマンドを入力... (help)"
            />
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Backdrop */
  .debug-overlay {
    position: fixed;
    inset: 0;
    z-index: 9998;
    background: rgba(0, 0, 0, 0.15);
    animation: fade-in 0.15s ease;
  }

  /* Floating window */
  .debug-window {
    position: fixed;
    z-index: 9999;
    width: 560px;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-primary);
    border: 0.5px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-lg), 0 0 0 0.5px rgba(0, 0, 0, 0.1);
    overflow: hidden;
    font-size: 12px;
    animation: debug-appear 0.25s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }

  @keyframes debug-appear {
    from { opacity: 0; transform: scale(0.95) translateY(8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  /* macOS window titlebar */
  .win-titlebar {
    display: flex;
    align-items: center;
    height: 36px;
    padding: 0 12px;
    background: var(--bg-secondary);
    border-bottom: 0.5px solid var(--border);
    cursor: grab;
    -webkit-user-select: none;
    user-select: none;
    gap: 8px;
  }
  .win-titlebar:active { cursor: grabbing; }

  .win-dots { display: flex; gap: 6px; align-items: center; }
  .dot {
    width: 12px; height: 12px; border-radius: 50%;
    border: none; padding: 0; cursor: default;
  }
  .dot-close {
    background: #ff5f57; cursor: pointer; transition: filter 0.15s;
  }
  .dot-close:hover { filter: brightness(0.85); }
  .dot-min { background: #febc2e; }
  .dot-max { background: #28c840; }

  .win-title {
    flex: 1; text-align: center;
    font-size: 12px; font-weight: 500;
    color: var(--text-secondary); pointer-events: none;
  }

  .win-badge {
    font-size: 10px; padding: 1px 6px; border-radius: 4px;
    pointer-events: none; font-weight: 500;
  }
  .badge-ok { background: rgba(52, 199, 89, 0.15); color: var(--green, #34c759); }
  .badge-ng { background: rgba(255, 59, 48, 0.12); color: var(--red); }

  /* Tab bar */
  .tab-bar {
    display: flex;
    background: var(--bg-secondary);
    border-bottom: 0.5px solid var(--border);
    padding: 0 8px;
  }
  .tab-bar button {
    padding: 6px 12px; font-size: 11px; font-weight: 500;
    color: var(--text-tertiary); background: transparent;
    border: none; border-bottom: 2px solid transparent;
    border-radius: 0; cursor: pointer; transition: color 0.15s;
  }
  .tab-bar button:hover { color: var(--text-primary); background: transparent; }
  .tab-bar button.active { color: var(--accent); border-bottom-color: var(--accent); }
  .tab-count { font-weight: 400; font-size: 10px; color: var(--text-tertiary); }

  /* Body */
  .win-body { flex: 1; overflow: auto; padding: 12px; }

  .section h4 {
    font-size: 11px; font-weight: 600;
    color: var(--text-secondary); text-transform: uppercase;
    letter-spacing: 0.05em; margin: 12px 0 6px;
  }
  .section h4:first-child { margin-top: 0; }
  .count { font-weight: 400; color: var(--text-tertiary); }
  .section-header { display: flex; justify-content: space-between; align-items: center; }

  /* Info rows */
  .info-grid { background: var(--bg-secondary); border-radius: 8px; overflow: hidden; }
  .info-row {
    display: flex; align-items: center;
    padding: 5px 10px; border-bottom: 0.5px solid var(--border); gap: 8px;
  }
  .info-row:last-child { border-bottom: none; }
  .info-key { color: var(--text-secondary); font-size: 11px; width: 100px; flex-shrink: 0; }
  .info-val { font-size: 11px; color: var(--text-primary); display: flex; align-items: center; gap: 4px; }
  .mono { font-family: "SF Mono", "Menlo", monospace; font-size: 10px; }
  .truncate { max-width: 320px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* Status dot */
  .dot-status {
    display: inline-block; width: 7px; height: 7px;
    border-radius: 50%; background: var(--text-tertiary); flex-shrink: 0;
  }
  .dot-status.ok { background: var(--green, #34c759); }
  .dot-status.ng { background: var(--red); }

  .muted { color: var(--text-tertiary); font-size: 11px; margin: 2px 0 6px; }

  /* Buttons */
  .action-btn {
    padding: 5px 14px; font-size: 11px; font-weight: 500;
    color: var(--text-on-accent); background: var(--accent);
    border: none; border-radius: 6px; cursor: pointer; margin-top: 4px;
    transition: opacity 0.15s, transform 0.1s;
  }
  .action-btn:hover { opacity: 0.85; }
  .action-btn:active { transform: scale(0.97); }
  .action-btn:disabled { opacity: 0.5; cursor: default; }
  .btn-warn {
    background: var(--orange, #ff9500);
  }

  .sm-btn {
    padding: 2px 8px; font-size: 10px; color: var(--text-secondary);
    background: transparent; border: 0.5px solid var(--border);
    border-radius: 4px; cursor: pointer;
  }
  .sm-btn:hover { color: var(--text-primary); background: var(--bg-hover); }

  /* Log list */
  .log-scroll {
    max-height: 240px; overflow: auto;
    background: var(--bg-secondary); border-radius: 8px;
    padding: 4px; margin-top: 6px;
  }
  .log-line {
    padding: 2px 6px; font-family: "SF Mono", "Menlo", monospace;
    font-size: 10.5px; line-height: 1.6; border-radius: 3px;
    display: flex; gap: 4px;
  }
  .log-line:hover { background: var(--bg-hover); }
  .lt { color: var(--text-tertiary); flex-shrink: 0; }
  .ll { font-weight: 600; flex-shrink: 0; }
  .lm { word-break: break-all; white-space: pre-wrap; }
  .log-info .ll { color: var(--green, #34c759); }
  .log-warn .ll { color: var(--orange, #ff9500); }
  .log-error .ll { color: var(--red); }
  .log-error .lm { color: var(--red); }
  .log-debug .ll { color: var(--blue, #007aff); }

  /* Console input */
  .cli-input {
    display: flex; align-items: center; gap: 6px; margin-top: 6px;
    background: var(--bg-secondary); border: 0.5px solid var(--border);
    border-radius: 8px; padding: 6px 8px;
    transition: border-color 0.2s, box-shadow 0.2s;
  }
  .cli-input:focus-within { border-color: var(--blue); box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1); }
  .cli-prompt { color: var(--green, #34c759); font-family: "SF Mono", monospace; font-weight: 700; font-size: 12px; }
  .cli-input input {
    flex: 1; background: transparent; border: none;
    color: var(--text-primary); font-family: "SF Mono", "Menlo", monospace;
    font-size: 11px; outline: none; padding: 0;
  }
  .cli-input input::placeholder { color: var(--text-tertiary); }

  /* Browser section */
  .browser-url-bar {
    display: flex; align-items: center; gap: 6px;
    background: var(--bg-secondary); border: 0.5px solid var(--border);
    border-radius: 8px; padding: 6px 8px; margin-bottom: 8px;
    transition: border-color 0.2s, box-shadow 0.2s;
  }
  .browser-url-bar:focus-within { border-color: var(--blue); box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1); }
  .browser-prefix { font-size: 10px; color: var(--text-tertiary); flex-shrink: 0; }
  .browser-url-bar input {
    flex: 1; border: none; background: transparent;
    font-family: "SF Mono", "Menlo", monospace; font-size: 11px;
    color: var(--text-primary); outline: none; padding: 0;
  }
  .browser-url-bar input::placeholder { color: var(--text-tertiary); }
  .browser-error {
    padding: 6px 10px; border-radius: 6px;
    background: rgba(255, 59, 48, 0.08); color: var(--red);
    font-size: 11px; margin-bottom: 8px;
    border: 0.5px solid rgba(255, 59, 48, 0.2);
  }
  .browser-pre {
    font-size: 10px; font-family: "SF Mono", "Menlo", monospace;
    color: var(--text-primary); white-space: pre-wrap; word-break: break-all;
    -webkit-user-select: text; user-select: text; margin: 0; padding: 4px;
  }

  /* Task observer */
  .task-tier-label {
    font-size: 10px; font-weight: 600; color: var(--text-tertiary);
    text-transform: uppercase; letter-spacing: 0.04em;
    margin: 8px 0 3px; padding-left: 2px;
  }
  .task-tier-label:first-child { margin-top: 0; }
  .dot-status.running {
    background: var(--orange, #ff9500);
    animation: task-pulse 1s ease-in-out infinite;
  }
  .task-running { color: var(--orange, #ff9500); font-size: 11px; }
  @keyframes task-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }
</style>
