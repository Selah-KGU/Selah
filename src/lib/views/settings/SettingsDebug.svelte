<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { getTaskSnapshot, onTaskChange } from "../../stores";
  import type { TaskInfo } from "../../stores";
  import { fetchPage } from "../../api";

  interface DebugInfo {
    app_version: string;
    tauri_version: string;
    auth_status: string;
    username: string;
    cookie_count: number;
    timestamp: string;
    os: string;
    arch: string;
    stt_configured_backend: string;
    stt_configured_partial_mode: string;
    stt_runtime_backend: string;
    stt_runtime_state: string;
    stt_active_caller: string;
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

  let activeSection = $state<"info" | "network" | "logs" | "tasks">("info");
  let debugInfo = $state<DebugInfo | null>(null);
  let pingResults = $state<PingResult[]>([]);
  let logEntries = $state<LogEntry[]>([]);
  let isPinging = $state(false);
  let consoleInput = $state("");
  let tasks = $state<TaskInfo[]>([]);
  let taskTick = $state(0);

  let browserPath = $state("/uniasv2/UnSSOLoginControl2");
  let browserHtml = $state("");
  let browserLoading = $state(false);
  let browserError = $state("");

  let notifTestMsg = $state("Selah テスト通知");
  let unlistenSttState: (() => void) | null = null;
  let unlistenSttConfigChanged: (() => void) | null = null;

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

  async function fetchDebugInfo() {
    try {
      debugInfo = await invoke<DebugInfo>("debug_info");
    } catch (e: any) {
      addLog("error", `デバッグ情報取得失敗: ${e}`);
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

  async function sendTestNotification() {
    try {
      await invoke("debug_test_notification", { title: "Selah", body: notifTestMsg });
      addLog("info", `テスト通知送信: "${notifTestMsg}"`);
    } catch (e: any) {
      addLog("error", `通知送信失敗: ${e}`);
    }
  }

  async function runConsoleCommand() {
    if (!consoleInput.trim()) return;
    const cmd = consoleInput.trim();
    consoleInput = "";
    addLog("debug", `> ${cmd}`);
    try {
      if (cmd === "info") await fetchDebugInfo();
      else if (cmd === "ping") await runPing();
      else if (cmd === "clear") logEntries = [];
      else if (cmd.startsWith("luna-page ")) {
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
      } else if (cmd === "help") addLog("info", "コマンド: info, ping, fetch <path>, luna-page <path>, notif, clear, help");
      else addLog("warn", `不明なコマンド: ${cmd} (helpで一覧表示)`);
    } catch (e: any) { addLog("error", String(e)); }
  }

  onMount(() => {
    tasks = getTaskSnapshot();
    const unsubTasks = onTaskChange(() => { tasks = getTaskSnapshot(); });
    const taskTickTimer = setInterval(() => {
      if (activeSection === "tasks") taskTick++;
    }, 2000);
    void listen("stt-state", () => {
      void fetchDebugInfo();
    }).then((fn) => {
      unlistenSttState = fn;
    });
    void listen("stt-config-changed", () => {
      void fetchDebugInfo();
    }).then((fn) => {
      unlistenSttConfigChanged = fn;
    });

    const prebootLogs = (window as any).__SELAH_PREBOOT_LOGS__ as { type: string; message: string; time: string }[] | undefined;
    if (prebootLogs) {
      const imported = prebootLogs.map(log => {
        const level = (["info", "warn", "error", "debug"].includes(log.type) ? log.type : "info") as LogEntry["level"];
        return { time: log.time, level, message: log.message };
      });
      logEntries = [...logEntries, ...imported];
      delete (window as any).__SELAH_PREBOOT_LOGS__;
    }
    addLog("info", "デバッグパネル初期化完了");

    void fetchDebugInfo();

    return () => {
      unlistenSttState?.();
      unlistenSttConfigChanged?.();
      unsubTasks();
      clearInterval(taskTickTimer);
    };
  });
</script>

<div class="hero-card">
  <div class="hero-icon" style="background:linear-gradient(135deg,rgba(142,142,147,0.15),rgba(88,86,214,0.15));">
    <svg viewBox="0 0 20 20" fill="none" stroke="#5856d6" stroke-width="1.3">
      <path d="M7 3a3 3 0 016 0v2H7zM4 9l2-1 1-2 3 3 3-3 1 2 2 1-1 2 1 2-2 1-1 2-3-3-3 3-1-2-2-1 1-2z" stroke-linejoin="round"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">デバッグ</h2>
    <p class="panel-desc">アプリ情報、定期タスク、ネットワーク診断、ログを確認できます。問題のトラブルシューティングに使用します。</p>
  </div>
</div>

<div class="tab-bar">
  <button class:active={activeSection === "info"} onclick={() => { activeSection = "info"; void fetchDebugInfo(); }}>
    情報
  </button>
  <button class:active={activeSection === "tasks"} onclick={() => { activeSection = "tasks"; tasks = getTaskSnapshot(); }}>
    タスク <span class="tab-count">({tasks.length})</span>
  </button>
  <button class:active={activeSection === "network"} onclick={() => activeSection = "network"}>
    通信
  </button>
  <button class:active={activeSection === "logs"} onclick={() => activeSection = "logs"}>
    ログ <span class="tab-count">({logEntries.length})</span>
  </button>
</div>

<div class="debug-body">
  {#if activeSection === "info"}
    <div class="section">
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
      </div>

      <h4>音声認識</h4>
      {#if debugInfo}
        <div class="info-grid">
          <div class="info-row"><span class="info-key">STT Config</span><span class="info-val">{debugInfo.stt_configured_backend}</span></div>
          <div class="info-row"><span class="info-key">STT Partial</span><span class="info-val">{debugInfo.stt_configured_partial_mode}</span></div>
          <div class="info-row"><span class="info-key">STT Runtime</span><span class="info-val">{debugInfo.stt_runtime_backend}</span></div>
          <div class="info-row"><span class="info-key">STT State</span><span class="info-val">{debugInfo.stt_runtime_state}</span></div>
          <div class="info-row"><span class="info-key">STT Caller</span><span class="info-val mono">{debugInfo.stt_active_caller}</span></div>
        </div>
      {/if}
    </div>

  {:else if activeSection === "tasks"}
    <div class="section">
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
                      {void taskTick, Math.round((Date.now() - t.lastRunTs) / 1000)}s ago
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
    <div class="section">
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

      <h4>テスト通知</h4>
      <div class="info-grid">
        <div class="info-row">
          <span class="info-key">メッセージ</span>
          <span class="info-val" style="flex:1;gap:6px;">
            <input
              type="text"
              class="notif-input"
              bind:value={notifTestMsg}
              placeholder="テスト通知メッセージ"
              onkeydown={(e) => e.key === "Enter" && sendTestNotification()}
            />
            <button class="sm-btn" onclick={sendTestNotification}>送信</button>
          </span>
        </div>
      </div>

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
    <div class="section">
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

<style>
  .tab-bar {
    display: flex;
    background: var(--bg-secondary);
    border-radius: 8px 8px 0 0;
    border: 0.5px solid var(--border);
    border-bottom: none;
    padding: 0 8px;
  }
  .tab-bar button {
    padding: 6px 12px;
    font-size: 11px;
    font-weight: 500;
    font-family: inherit;
    color: var(--text-tertiary);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    cursor: pointer;
    transition: color 0.15s;
  }
  .tab-bar button:hover {
    color: var(--text-primary);
  }
  .tab-bar button.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .tab-count {
    font-weight: 400;
    font-size: 10px;
    color: var(--text-tertiary);
  }

  .debug-body {
    border: 0.5px solid var(--border);
    border-top: none;
    border-radius: 0 0 8px 8px;
    background: var(--bg-primary);
    padding: 12px;
    font-size: 12px;
  }

  .section h4 {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 12px 0 6px;
  }
  .section h4:first-child {
    margin-top: 0;
  }
  .count {
    font-weight: 400;
    color: var(--text-tertiary);
  }
  .section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .info-grid {
    background: var(--bg-secondary);
    border-radius: 8px;
    overflow: hidden;
  }
  .info-row {
    display: flex;
    align-items: center;
    padding: 5px 10px;
    border-bottom: 0.5px solid var(--border);
    gap: 8px;
  }
  .info-row:last-child {
    border-bottom: none;
  }
  .info-key {
    color: var(--text-secondary);
    font-size: 11px;
    width: 100px;
    flex-shrink: 0;
  }
  .info-val {
    font-size: 11px;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .mono {
    font-family: "SF Mono", "Menlo", monospace;
    font-size: 10px;
  }
  .truncate {
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot-status {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-tertiary);
    flex-shrink: 0;
  }
  .dot-status.ok {
    background: var(--green, #34c759);
  }
  .dot-status.ng {
    background: var(--red);
  }
  .dot-status.running {
    background: var(--orange, #ff9500);
    animation: task-pulse 1s ease-in-out infinite;
  }
  .task-running {
    color: var(--orange, #ff9500);
    font-size: 11px;
  }
  .task-tier-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 8px 0 3px;
    padding-left: 2px;
  }
  .task-tier-label:first-child {
    margin-top: 0;
  }

  .muted {
    color: var(--text-tertiary);
    font-size: 11px;
    margin: 2px 0 6px;
  }

  .action-bar {
    display: flex;
    gap: 8px;
    margin-top: 10px;
    align-items: center;
  }
  .action-btn {
    padding: 5px 14px;
    font-size: 11px;
    font-weight: 500;
    font-family: inherit;
    color: #fff;
    background: var(--accent);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    margin-top: 4px;
    transition: opacity 0.15s;
  }
  .action-btn:hover {
    opacity: 0.85;
  }
  .action-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .sm-btn {
    padding: 2px 8px;
    font-size: 10px;
    font-family: inherit;
    color: var(--text-secondary);
    background: transparent;
    border: 0.5px solid var(--border);
    border-radius: 4px;
    cursor: pointer;
  }
  .sm-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .log-scroll {
    max-height: 280px;
    overflow: auto;
    background: var(--bg-secondary);
    border-radius: 8px;
    padding: 4px;
    margin-top: 6px;
  }
  .log-line {
    padding: 2px 6px;
    font-family: "SF Mono", "Menlo", monospace;
    font-size: 10.5px;
    line-height: 1.6;
    border-radius: 3px;
    display: flex;
    gap: 4px;
  }
  .log-line:hover {
    background: var(--bg-hover);
  }
  .lt {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }
  .ll {
    font-weight: 600;
    flex-shrink: 0;
  }
  .lm {
    word-break: break-all;
    white-space: pre-wrap;
  }
  .log-info .ll {
    color: var(--green, #34c759);
  }
  .log-warn .ll {
    color: var(--orange, #ff9500);
  }
  .log-error .ll {
    color: var(--red);
  }
  .log-error .lm {
    color: var(--red);
  }
  .log-debug .ll {
    color: var(--blue, #007aff);
  }

  .cli-input {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    border-radius: 8px;
    padding: 6px 8px;
    transition: border-color 0.2s, box-shadow 0.2s;
  }
  .cli-input:focus-within {
    border-color: var(--blue);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
  }
  .cli-prompt {
    color: var(--green, #34c759);
    font-family: "SF Mono", monospace;
    font-weight: 700;
    font-size: 12px;
  }
  .cli-input input {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-family: "SF Mono", "Menlo", monospace;
    font-size: 11px;
    outline: none;
    padding: 0;
  }
  .cli-input input::placeholder {
    color: var(--text-tertiary);
  }

  .browser-url-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    border-radius: 8px;
    padding: 6px 8px;
    margin-bottom: 8px;
    transition: border-color 0.2s, box-shadow 0.2s;
  }
  .browser-url-bar:focus-within {
    border-color: var(--blue);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
  }
  .browser-prefix {
    font-size: 10px;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }
  .browser-url-bar input {
    flex: 1;
    border: none;
    background: transparent;
    font-family: "SF Mono", "Menlo", monospace;
    font-size: 11px;
    color: var(--text-primary);
    outline: none;
    padding: 0;
  }
  .browser-url-bar input::placeholder {
    color: var(--text-tertiary);
  }
  .browser-error {
    padding: 6px 10px;
    border-radius: 6px;
    background: rgba(255, 59, 48, 0.08);
    color: var(--red);
    font-size: 11px;
    margin-bottom: 8px;
    border: 0.5px solid rgba(255, 59, 48, 0.2);
  }
  .browser-pre {
    font-size: 10px;
    font-family: "SF Mono", "Menlo", monospace;
    color: var(--text-primary);
    white-space: pre-wrap;
    word-break: break-all;
    -webkit-user-select: text;
    user-select: text;
    margin: 0;
    padding: 4px;
  }

  .notif-input {
    flex: 1;
    border: 0.5px solid var(--border);
    background: var(--bg-primary);
    border-radius: 4px;
    padding: 3px 6px;
    font-size: 11px;
    font-family: inherit;
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.2s;
  }
  .notif-input:focus {
    border-color: var(--blue);
  }

  @keyframes task-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }
</style>
