<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { getTaskSnapshot, onTaskChange } from "../../stores";
  import type { TaskInfo } from "../../stores";
  import { fetchPage, isDemoActive, refreshBackendTaskStatuses } from "../../api";
  import { appUpdateState } from "../../updater";

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
    stt_configured_sensitivity: string;
    stt_runtime_backend: string;
    stt_runtime_state: string;
    stt_active_caller: string;
    stt_runtime_note: string;
    stt_runtime_error: string;
    notification_debug: NotificationDebugInfo;
  }

  interface NotificationSourceDebugInfo {
    source: string;
    authenticated: boolean;
    initialized: boolean;
    has_seen_state: boolean;
    seen_count: number;
  }

  interface NotificationDebugInfo {
    poll_running: boolean;
    delivery_note: string;
    bootstrap_mode: string;
    suppress_push: boolean;
    bootstrap_complete: boolean;
    bootstrap_started_at_epoch: number | null;
    bootstrap_started_ago_secs: number | null;
    grace_period_secs: number;
    authenticated_sources: string[];
    sources: NotificationSourceDebugInfo[];
    last_sync: NotificationLastSyncDebugInfo;
    recent_events: NotificationEventDebugInfo[];
  }

  interface NotificationLastSyncDebugInfo {
    started_at_epoch: number | null;
    finished_at_epoch: number | null;
    status: string;
    error: string;
    bootstrap_mode: string;
    suppress_push: boolean;
    dispatched: number;
    failed: number;
    suppressed: number;
    muted: number;
    seeded_sources: string[];
    fetch_failures: string[];
  }

  interface NotificationEventDebugInfo {
    at_epoch: number;
    source: string;
    status: string;
    title: string;
    body: string;
    detail: string;
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
  let notifSyncing = $state(false);
  let unlistenSttState: (() => void) | null = null;
  let unlistenSttConfigChanged: (() => void) | null = null;
  let unlistenSttInfo: (() => void) | null = null;
  let unlistenSttError: (() => void) | null = null;
  let unlistenSttRuntimeDebugChanged: (() => void) | null = null;

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
      if (isDemoActive()) {
        browserHtml = `<!doctype html><html><body><h1>Debug Demo</h1><p>path: ${browserPath}</p></body></html>`;
        addLog("info", `ブラウザ: ${browserPath} を演示モードで表示`);
        return;
      }
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
      if (isDemoActive()) {
        debugInfo = {
          app_version: "demo",
          tauri_version: "demo",
          auth_status: "demo",
          username: "demo_user",
          cookie_count: 0,
          timestamp: new Date().toISOString(),
          os: navigator.platform || "DemoOS",
          arch: "universal",
          stt_configured_backend: "demo",
          stt_configured_partial_mode: "balanced",
          stt_configured_sensitivity: "標準",
          stt_runtime_backend: "demo",
          stt_runtime_state: "idle",
          stt_active_caller: "none",
          stt_runtime_note: "",
          stt_runtime_error: "",
          notification_debug: {
            poll_running: false,
            delivery_note: "demo",
            bootstrap_mode: "silent",
            suppress_push: true,
            bootstrap_complete: false,
            bootstrap_started_at_epoch: null,
            bootstrap_started_ago_secs: null,
            grace_period_secs: 360,
            authenticated_sources: [],
            sources: [
              { source: "kgc", authenticated: false, initialized: false, has_seen_state: false, seen_count: 0 },
              { source: "luna", authenticated: false, initialized: false, has_seen_state: false, seen_count: 0 },
              { source: "kwic", authenticated: false, initialized: false, has_seen_state: false, seen_count: 0 },
              { source: "mail", authenticated: false, initialized: false, has_seen_state: false, seen_count: 0 },
            ],
            last_sync: {
              started_at_epoch: null,
              finished_at_epoch: null,
              status: "idle",
              error: "",
              bootstrap_mode: "silent",
              suppress_push: true,
              dispatched: 0,
              failed: 0,
              suppressed: 0,
              muted: 0,
              seeded_sources: [],
              fetch_failures: [],
            },
            recent_events: [],
          },
        };
        return;
      }
      debugInfo = await invoke<DebugInfo>("debug_info");
    } catch (e: any) {
      addLog("error", `デバッグ情報取得失敗: ${e}`);
    }
  }

  async function runPing() {
    isPinging = true;
    pingResults = [];
    addLog("info", "ネットワーク診断を開始...");
    if (isDemoActive()) {
      pingResults = targets.map((target, idx) => ({
        target: target.url,
        reachable: true,
        status_code: 200,
        latency_ms: 40 + idx * 12,
        error: "",
      }));
      addLog("info", "演示モードの接続テストを表示しました");
      isPinging = false;
      return;
    }
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
      if (isDemoActive()) {
        addLog("info", `演示通知: "${notifTestMsg}"`);
        return;
      }
      await invoke("debug_test_notification", { title: "Selah", body: notifTestMsg });
      addLog("info", `テスト通知送信: "${notifTestMsg}"`);
    } catch (e: any) {
      addLog("error", `通知送信失敗: ${e}`);
    }
  }

  async function syncNotificationsNow() {
    notifSyncing = true;
    try {
      if (isDemoActive()) {
        addLog("info", "演示モード: 通知同期をスキップ");
        return;
      }
      await invoke("notification_sync_now");
      addLog("info", "通知同期を手動実行しました");
      await fetchDebugInfo();
    } catch (e: any) {
      addLog("error", `通知同期失敗: ${e}`);
    } finally {
      notifSyncing = false;
    }
  }

  function boolLabel(value: boolean): string {
    return value ? "yes" : "no";
  }

  function formatEpoch(epoch: number | null): string {
    if (!epoch) return "-";
    return new Date(epoch * 1000).toLocaleString("ja-JP");
  }

  function formatBytes(bytes: number | null): string {
    if (bytes == null || bytes <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    let value = bytes;
    let index = 0;
    while (value >= 1024 && index < units.length - 1) {
      value /= 1024;
      index++;
    }
    return `${value >= 100 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
  }

  function updaterProgressLabel(): string {
    if ($appUpdateState.progressPercent != null) return `${$appUpdateState.progressPercent}%`;
    if ($appUpdateState.downloadedBytes > 0) return formatBytes($appUpdateState.downloadedBytes);
    return "-";
  }

  function formatEventTime(epoch: number): string {
    return new Date(epoch * 1000).toLocaleTimeString("ja-JP");
  }

  function formatInterval(ms: number): string {
    if (!ms) return "-";
    const min = Math.round(ms / 60_000);
    if (min < 60) return `${min}min`;
    const hours = ms / 3_600_000;
    return `${Number.isInteger(hours) ? hours.toFixed(0) : hours.toFixed(1)}h`;
  }

  async function refreshTaskSnapshot() {
    await refreshBackendTaskStatuses().catch((err) => {
      addLog("warn", `タスク状態更新失敗: ${err}`);
    });
    tasks = getTaskSnapshot();
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
        if (isDemoActive()) {
          addLog("info", `Title: Luna Demo\nSize: 96\nLinks:\n/course/view.php?id=demo-course-1`);
          return;
        }
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
    // Only tick when the tasks subsection is actually visible — otherwise
    // we're firing a 2s timer for a value that nothing reads.
    let taskTickTimer: ReturnType<typeof setInterval> | null = null;
    function syncTaskTimer() {
      if (activeSection === "tasks") {
        if (!taskTickTimer) {
          taskTickTimer = setInterval(() => { taskTick++; }, 2000);
        }
      } else if (taskTickTimer) {
        clearInterval(taskTickTimer);
        taskTickTimer = null;
      }
    }
    syncTaskTimer();
    const stopTaskTimerEffect = $effect.root(() => {
      $effect(() => { activeSection; syncTaskTimer(); });
    });
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
    void listen("stt-info", () => {
      void fetchDebugInfo();
    }).then((fn) => {
      unlistenSttInfo = fn;
    });
    void listen("stt-error", () => {
      void fetchDebugInfo();
    }).then((fn) => {
      unlistenSttError = fn;
    });
    void listen("stt-runtime-debug-changed", () => {
      void fetchDebugInfo();
    }).then((fn) => {
      unlistenSttRuntimeDebugChanged = fn;
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
      unlistenSttInfo?.();
      unlistenSttError?.();
      unlistenSttRuntimeDebugChanged?.();
      unsubTasks();
      if (taskTickTimer) clearInterval(taskTickTimer);
      stopTaskTimerEffect();
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
  <button class:active={activeSection === "tasks"} onclick={() => { activeSection = "tasks"; void refreshTaskSnapshot(); }}>
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

      <h4>自動更新</h4>
      <div class="info-grid">
        <div class="info-row"><span class="info-key">Phase</span><span class="info-val mono">{$appUpdateState.phase}</span></div>
        <div class="info-row"><span class="info-key">Checking</span><span class="info-val">{boolLabel($appUpdateState.checking)}</span></div>
        <div class="info-row"><span class="info-key">Available</span><span class="info-val">{boolLabel($appUpdateState.available)}</span></div>
        <div class="info-row"><span class="info-key">Version</span><span class="info-val mono">{$appUpdateState.version || "-"}</span></div>
        <div class="info-row"><span class="info-key">Progress</span><span class="info-val mono">{updaterProgressLabel()}</span></div>
        <div class="info-row"><span class="info-key">Downloaded</span><span class="info-val mono">{formatBytes($appUpdateState.downloadedBytes)}{#if $appUpdateState.totalBytes} / {formatBytes($appUpdateState.totalBytes)}{/if}</span></div>
        <div class="info-row span-2"><span class="info-key">Status</span><span class="info-val">{$appUpdateState.status}</span></div>
      </div>

      <h4>音声認識</h4>
      {#if debugInfo}
        <div class="info-grid">
          <div class="info-row"><span class="info-key">STT Config</span><span class="info-val">{debugInfo.stt_configured_backend}</span></div>
          <div class="info-row"><span class="info-key">STT Partial</span><span class="info-val">{debugInfo.stt_configured_partial_mode}</span></div>
          <div class="info-row"><span class="info-key">STT Sensitivity</span><span class="info-val">{debugInfo.stt_configured_sensitivity}</span></div>
          <div class="info-row"><span class="info-key">STT Runtime</span><span class="info-val">{debugInfo.stt_runtime_backend}</span></div>
          <div class="info-row"><span class="info-key">STT State</span><span class="info-val">{debugInfo.stt_runtime_state}</span></div>
          <div class="info-row"><span class="info-key">STT Caller</span><span class="info-val mono">{debugInfo.stt_active_caller}</span></div>
          <div class="info-row"><span class="info-key">STT Note</span><span class="info-val">{debugInfo.stt_runtime_note || "-"}</span></div>
          <div class="info-row"><span class="info-key">STT Error</span><span class="info-val">{debugInfo.stt_runtime_error || "-"}</span></div>
        </div>
      {/if}

      <div class="section-header">
        <h4>通知</h4>
        <div class="header-actions">
          <button class="tool-btn" onclick={() => void fetchDebugInfo()}>更新</button>
          <button class="tool-btn primary-soft" onclick={syncNotificationsNow} disabled={notifSyncing}>
            {notifSyncing ? "同期中..." : "今すぐ同期"}
          </button>
        </div>
      </div>
      {#if debugInfo}
        <div class="info-grid">
          <div class="info-row"><span class="info-key">Poll Running</span><span class="info-val">{debugInfo.notification_debug.poll_running ? "running" : "idle"}</span></div>
          <div class="info-row"><span class="info-key">Bootstrap Mode</span><span class="info-val mono">{debugInfo.notification_debug.bootstrap_mode}</span></div>
          <div class="info-row"><span class="info-key">Suppress Push</span><span class="info-val">{boolLabel(debugInfo.notification_debug.suppress_push)}</span></div>
          <div class="info-row"><span class="info-key">Bootstrap Done</span><span class="info-val">{boolLabel(debugInfo.notification_debug.bootstrap_complete)}</span></div>
          <div class="info-row"><span class="info-key">Started At</span><span class="info-val mono">{formatEpoch(debugInfo.notification_debug.bootstrap_started_at_epoch)}</span></div>
          <div class="info-row"><span class="info-key">Started Ago</span><span class="info-val mono">{debugInfo.notification_debug.bootstrap_started_ago_secs !== null ? `${debugInfo.notification_debug.bootstrap_started_ago_secs}s` : "-"}</span></div>
          <div class="info-row"><span class="info-key">Grace</span><span class="info-val mono">{debugInfo.notification_debug.grace_period_secs}s</span></div>
          <div class="info-row"><span class="info-key">Authed Sources</span><span class="info-val mono">{debugInfo.notification_debug.authenticated_sources.length ? debugInfo.notification_debug.authenticated_sources.join(", ") : "-"}</span></div>
          <div class="info-row"><span class="info-key">Dispatch Note</span><span class="info-val">{debugInfo.notification_debug.delivery_note}</span></div>
        </div>

        <div class="info-grid compact-grid">
          {#each debugInfo.notification_debug.sources as source}
            <div class="info-row">
              <span class="info-key source-key mono">{source.source}</span>
              <span class="info-val source-flags mono">
                auth={boolLabel(source.authenticated)}
                init={boolLabel(source.initialized)}
                state={boolLabel(source.has_seen_state)}
                seen={source.seen_count}
              </span>
            </div>
          {/each}
        </div>

        <h4>通知 Last Sync</h4>
        <div class="info-grid compact-grid">
          <div class="info-row"><span class="info-key">Status</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.status || "-"}</span></div>
          <div class="info-row"><span class="info-key">Started</span><span class="info-val mono">{formatEpoch(debugInfo.notification_debug.last_sync.started_at_epoch)}</span></div>
          <div class="info-row"><span class="info-key">Finished</span><span class="info-val mono">{formatEpoch(debugInfo.notification_debug.last_sync.finished_at_epoch)}</span></div>
          <div class="info-row"><span class="info-key">Sync Mode</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.bootstrap_mode || "-"}</span></div>
          <div class="info-row"><span class="info-key">Suppress Push</span><span class="info-val">{boolLabel(debugInfo.notification_debug.last_sync.suppress_push)}</span></div>
          <div class="info-row"><span class="info-key">Dispatched</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.dispatched}</span></div>
          <div class="info-row"><span class="info-key">Failed</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.failed}</span></div>
          <div class="info-row"><span class="info-key">Suppressed</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.suppressed}</span></div>
          <div class="info-row"><span class="info-key">Muted</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.muted}</span></div>
          <div class="info-row"><span class="info-key">Seeded</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.seeded_sources.length ? debugInfo.notification_debug.last_sync.seeded_sources.join(", ") : "-"}</span></div>
          <div class="info-row"><span class="info-key">Fetch Errors</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.fetch_failures.length ? debugInfo.notification_debug.last_sync.fetch_failures.join(" | ") : "-"}</span></div>
          <div class="info-row"><span class="info-key">Sync Error</span><span class="info-val mono">{debugInfo.notification_debug.last_sync.error || "-"}</span></div>
        </div>

        <h4>通知 Events</h4>
        {#if debugInfo.notification_debug.recent_events.length > 0}
          <div class="log-scroll">
            {#each debugInfo.notification_debug.recent_events.slice().reverse() as event}
              <div class="log-line" class:log-error={event.status === "failed"} class:log-warn={event.status === "suppressed"} class:log-info={event.status === "dispatched" || event.status === "seeded"}>
                <span class="lt">{formatEventTime(event.at_epoch)}</span>
                <span class="ll">[{event.source}/{event.status}]</span>
                <span class="lm">{event.title} :: {event.body}{event.detail ? ` (${event.detail})` : ""}</span>
              </div>
            {/each}
          </div>
        {:else}
          <p class="muted">通知イベントはまだありません</p>
        {/if}
      {/if}
    </div>

  {:else if activeSection === "tasks"}
    <div class="section">
      <div class="section-header">
        <h4>定期タスク</h4>
        <button class="tool-btn" onclick={() => { void refreshTaskSnapshot(); }}>更新</button>
      </div>
      {#each ["volatile", "stable", "system"] as tier}
        {@const tierTasks = tasks.filter(t => t.tier === tier)}
        {#if tierTasks.length > 0}
          <div class="task-tier-label">
            {tier === "volatile" ? "高頻度" : tier === "stable" ? "低頻度" : "システム"}
          </div>
          <div class="info-grid task-grid">
            {#each tierTasks as t}
              <div class="info-row task-row" title={t.key}>
                <span class="info-key task-name">{t.label}</span>
                <span class="info-val task-status">
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
                    <span class="muted inline-muted">未実行</span>
                  {/if}
                </span>
                <span class="task-meta mono">
                  {#if t.lastRunTs}
                    {formatInterval(t.intervalMs)} · {void taskTick, Math.round((Date.now() - t.lastRunTs) / 1000)}s ago
                  {:else}
                    {formatInterval(t.intervalMs)} · --
                  {/if}
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
        <div class="info-grid result-grid">
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
          <span class="info-val notif-control">
            <input
              type="text"
              class="notif-input"
              bind:value={notifTestMsg}
              placeholder="テスト通知メッセージ"
              onkeydown={(e) => e.key === "Enter" && sendTestNotification()}
            />
            <button class="tool-btn input-action" onclick={sendTestNotification}>送信</button>
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
        <button class="tool-btn primary browser-go-btn" onclick={browserNavigate} disabled={browserLoading}>
          {browserLoading ? "..." : "Go"}
        </button>
      </div>
      {#if browserError}
        <div class="browser-error">{browserError}</div>
      {/if}
      {#if browserHtml}
        <div class="log-scroll browser-html-scroll">
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
        <button class="tool-btn danger-soft" onclick={() => logEntries = []}>クリア</button>
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
    gap: 4px;
    background: var(--bg-secondary);
    border-radius: 10px;
    border: 0.5px solid var(--border);
    padding: 4px;
    margin-bottom: 10px;
    overflow-x: auto;
    box-shadow: inset 0 1px 0 color-mix(in srgb, #fff 40%, transparent);
  }
  .tab-bar button {
    min-height: 30px;
    padding: 0 12px;
    font-size: 11px;
    font-weight: 600;
    font-family: inherit;
    color: var(--text-tertiary);
    background: transparent;
    border: none;
    border-radius: 7px;
    cursor: pointer;
    white-space: nowrap;
    transition: color 0.15s, background 0.15s, box-shadow 0.15s, transform 0.15s;
  }
  .tab-bar button:hover {
    color: var(--text-primary);
    background: color-mix(in srgb, var(--text-primary) 6%, transparent);
  }
  .tab-bar button.active {
    color: var(--accent);
    background: var(--bg-primary);
    box-shadow: var(--shadow-sm);
  }
  .tab-bar button.active:hover {
    transform: translateY(-1px);
  }
  .tab-count {
    font-weight: 400;
    font-size: 10px;
    color: var(--text-tertiary);
  }

  .debug-body {
    border-radius: 10px;
    background: var(--bg-primary);
    padding: 6px;
    font-size: 12px;
    min-width: 0;
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
    gap: 10px;
    margin-top: 12px;
    margin-bottom: 7px;
  }
  .section-header:first-child {
    margin-top: 0;
  }
  .section-header h4 {
    margin: 0;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .info-grid {
    background: var(--bg-secondary);
    border-radius: 8px;
    border: 0.5px solid var(--border);
    overflow: hidden;
    min-width: 0;
  }
  .compact-grid {
    margin-top: 8px;
  }
  .info-row {
    display: flex;
    align-items: center;
    min-height: 34px;
    padding: 7px 10px;
    border-bottom: 0.5px solid var(--border);
    gap: 8px;
    min-width: 0;
  }
  .info-row:last-child {
    border-bottom: none;
  }
  .info-key {
    color: var(--text-secondary);
    font-size: 11px;
    width: 118px;
    flex-shrink: 0;
    line-height: 1.35;
  }
  .source-key {
    text-transform: uppercase;
  }
  .info-val {
    font-size: 11px;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }
  .source-flags {
    flex-wrap: wrap;
    row-gap: 2px;
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
  .task-grid {
    margin-bottom: 10px;
  }
  .task-row {
    display: grid;
    grid-template-columns: minmax(150px, 1fr) minmax(86px, auto) minmax(96px, auto);
    align-items: center;
    column-gap: 12px;
  }
  .task-name {
    width: auto;
    min-width: 0;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .task-status {
    justify-content: flex-start;
    white-space: nowrap;
  }
  .task-meta {
    justify-self: end;
    color: var(--text-tertiary);
    font-size: 10px;
    white-space: nowrap;
  }
  .task-tier-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 12px 0 5px;
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
  .inline-muted {
    margin: 0;
  }

  .action-bar {
    display: flex;
    gap: 8px;
    margin-top: 10px;
    align-items: center;
  }
  .action-btn {
    min-height: 27px;
    padding: 0 10px;
    font-size: 10.5px;
    font-weight: 600;
    font-family: inherit;
    color: #fff;
    background: var(--accent);
    border: none;
    border-radius: 7px;
    cursor: pointer;
    margin: 4px 0 10px;
    box-shadow: var(--shadow-sm);
    transition: background 0.15s, box-shadow 0.15s, opacity 0.15s, transform 0.15s;
  }
  .action-btn:hover {
    opacity: 0.9;
    box-shadow: var(--shadow-md);
    transform: translateY(-1px);
  }
  .action-btn:disabled {
    opacity: 0.5;
    cursor: default;
    box-shadow: none;
    transform: none;
  }
  .tool-btn {
    min-height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 0 8px;
    font-size: 10px;
    font-weight: 600;
    font-family: inherit;
    color: var(--text-secondary);
    background: var(--bg-primary);
    border: 0.5px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
    box-shadow: 0 1px 0 color-mix(in srgb, #fff 36%, transparent);
    transition: background 0.15s, border-color 0.15s, color 0.15s, box-shadow 0.15s, transform 0.15s, opacity 0.15s;
  }
  .tool-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
    border-color: var(--border-strong);
    box-shadow: var(--shadow-sm);
    transform: translateY(-1px);
  }
  .tool-btn:disabled {
    opacity: 0.5;
    cursor: default;
    box-shadow: none;
    transform: none;
  }
  .tool-btn.primary,
  .tool-btn.primary-soft {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
    background: color-mix(in srgb, var(--accent) 9%, var(--bg-primary));
  }
  .tool-btn.primary:hover,
  .tool-btn.primary-soft:hover {
    background: color-mix(in srgb, var(--accent) 14%, var(--bg-primary));
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .tool-btn.primary {
    min-width: 40px;
  }
  .tool-btn.danger-soft {
    color: var(--red);
    border-color: color-mix(in srgb, var(--red) 24%, var(--border));
    background: color-mix(in srgb, var(--red) 7%, var(--bg-primary));
  }
  .tool-btn.danger-soft:hover {
    background: color-mix(in srgb, var(--red) 12%, var(--bg-primary));
    border-color: color-mix(in srgb, var(--red) 38%, var(--border));
  }
  .tool-btn.input-action {
    align-self: stretch;
    min-height: 24px;
  }

  .log-scroll {
    max-height: 280px;
    overflow: auto;
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    border-radius: 8px;
    padding: 4px;
    margin-top: 6px;
  }
  .browser-html-scroll {
    max-height: 300px;
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
    min-width: 0;
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
    min-width: 0;
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
  .browser-go-btn {
    min-height: 24px;
    flex-shrink: 0;
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
    min-width: 0;
    border: 0.5px solid var(--border);
    background: var(--bg-primary);
    border-radius: 6px;
    padding: 2px 6px;
    font-size: 11px;
    font-family: inherit;
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.2s;
  }
  .notif-input:focus {
    border-color: var(--blue);
  }
  .notif-control {
    flex: 1;
    gap: 5px;
  }
  .result-grid {
    margin-top: 0;
  }

  @keyframes task-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  @media (max-width: 720px) {
    .section-header {
      align-items: flex-start;
      flex-direction: column;
    }
    .header-actions {
      justify-content: flex-start;
    }
    .info-row {
      align-items: flex-start;
    }
    .info-key {
      width: 106px;
    }
    .task-row {
      grid-template-columns: 1fr auto;
      row-gap: 4px;
    }
    .task-name {
      grid-column: 1 / -1;
      white-space: normal;
    }
    .task-meta {
      justify-self: end;
    }
    .browser-url-bar {
      align-items: stretch;
      flex-wrap: wrap;
    }
    .browser-url-bar input {
      min-width: 180px;
    }
  }

  @media (max-width: 520px) {
    .debug-body {
      padding: 10px;
    }
    .info-row {
      flex-direction: column;
      gap: 3px;
    }
    .info-key {
      width: auto;
    }
    .info-val {
      width: 100%;
    }
    .task-row {
      display: grid;
      grid-template-columns: 1fr;
    }
    .task-meta {
      justify-self: start;
    }
  }
</style>
