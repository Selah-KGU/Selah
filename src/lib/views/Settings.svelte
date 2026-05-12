<script lang="ts">
  import { onDestroy } from "svelte";
  import { activeSettingsPanel, devModeActive } from "../stores";
  import type { SettingsPanel } from "../stores";
  import SettingsAi from "./settings/SettingsAi.svelte";
  import SettingsSession from "./settings/SettingsSession.svelte";
  import SettingsMail from "./settings/SettingsMail.svelte";
  import SettingsCalendar from "./settings/SettingsCalendar.svelte";
  import SettingsNotification from "./settings/SettingsNotification.svelte";
  import SettingsDownload from "./settings/SettingsDownload.svelte";
  import SettingsAbout from "./settings/SettingsAbout.svelte";
  import SettingsDebug from "./settings/SettingsDebug.svelte";

  interface NavItem {
    id: SettingsPanel;
    label: string;
    icon: string;
    devOnly?: boolean;
  }

  type SettingSearchItem = readonly [SettingsPanel, string, string, string?, boolean?];

  const navItems: NavItem[] = [
    { id: "ai", label: "AI 設定", icon: "sparkles" },
    { id: "session", label: "セッション", icon: "lock" },
    { id: "mail", label: "メール", icon: "envelope" },
    { id: "calendar", label: "カレンダー", icon: "calendar" },
    { id: "notification", label: "通知", icon: "bell" },
    { id: "download", label: "ダウンロード", icon: "arrow.down" },
    { id: "about", label: "このアプリについて", icon: "info" },
    { id: "debug", label: "デバッグ", icon: "wrench", devOnly: true },
  ];

  const settingSearchItems: SettingSearchItem[] = [
    ["ai", "AI アシスタント", "AI 機能", "有効 無効 home todo live"],
    ["ai", "AI アシスタント", "回答言語", "日本語 中文 english 要約"],
    ["ai", "AI アシスタント", "更新間隔", "通知分析 課題分析 自動再計算"],
    ["ai", "AI アシスタント", "LIVE 要約の生成間隔", "live 講義 分割要約"],
    ["ai", "AI アシスタントの推論", "推論方法", "local api ローカル openai gemini"],
    ["ai", "AI アシスタントのモデル", "ローカルモデル", "qwen llama download metal vulkan"],
    ["ai", "AI アシスタントの API 設定", "API キー", "openai gemini key"],
    ["ai", "AI アシスタントの API 設定", "モデル名", "model gpt gemini"],
    ["ai", "AI アシスタントの API 設定", "ベース URL", "endpoint openai compatible 互換"],
    ["ai", "AI アシスタントの API 設定", "最大トークン", "token 出力上限"],
    ["ai", "AI アシスタントの API 設定", "Temperature", "sampling サンプリング"],
    ["ai", "Agent 音声ショートカット", "ショートカット", "voice 音声 agent カプセル"],
    ["ai", "Agent 音声ショートカット", "トリガーキー", "fn hotkey keyboard"],
    ["ai", "Agent 音声ショートカット", "リアルタイム字幕", "subtitle live 字幕 overlay"],
    ["ai", "AI 音声文字起こし", "STT 実行バックエンド", "cpu metal 音声認識"],
    ["ai", "AI 音声文字起こし", "AI 音声認識言語", "日本語 英語 自動検出 language"],
    ["ai", "AI 音声文字起こし", "STT 省電モード", "partial 省電 節電 live"],
    ["ai", "AI 音声文字起こし", "音声感度", "感度 発話検出 vad"],
    ["session", "セッション状態", "KG Course", "ログイン 認証 session"],
    ["session", "セッション状態", "Luna LMS", "ログイン 認証 session"],
    ["session", "セッション状態", "KWIC Portal", "ログイン 認証 session"],
    ["mail", "Microsoft Azure AD", "アカウント", "microsoft mail outlook graph"],
    ["mail", "Azure AD クライアント ID", "クライアント ID", "azure client mail"],
    ["calendar", "学期設定", "春学期開始日", "spring term 日付"],
    ["calendar", "学期設定", "秋学期開始日", "fall term 日付"],
    ["calendar", "同期先", "Google Calendar", "google calendar 同期"],
    ["calendar", "自動同期", "Google Calendar", "自動 同期"],
    ["calendar", "自動同期", "同期間隔", "interval schedule"],
    ["calendar", "Google Calendar API 設定", "クライアント ID", "google cloud oauth"],
    ["calendar", "Google Calendar API 設定", "シークレット", "secret oauth"],
    ["notification", "通知設定", "通知権限", "permission toast native"],
    ["notification", "受信カテゴリ", "呼出し・重要なお知らせ", "重要 呼出し"],
    ["notification", "受信カテゴリ", "学部・研究科からのお知らせ", "学部 研究科"],
    ["notification", "受信カテゴリ", "授業のお知らせ", "授業 class"],
    ["notification", "受信カテゴリ", "その他", "other"],
    ["notification", "受信カテゴリ", "メール", "mail"],
    ["notification", "授業通知の詳細", "一般の授業通知", "kwic kg-course luna"],
    ["notification", "授業通知の詳細", "Luna お知らせ・資料", "資料 material"],
    ["notification", "授業通知の詳細", "Luna 課題・レポート", "課題 report"],
    ["notification", "授業通知の詳細", "Luna テスト・小テスト", "quiz test"],
    ["notification", "授業通知の詳細", "Luna 掲示板・コメント", "forum comment"],
    ["notification", "授業通知の詳細", "Luna アンケート", "survey"],
    ["notification", "授業通知の詳細", "Luna 出席", "attendance"],
    ["download", "保存先", "フォルダ", "download 保存 folder 教材"],
    ["download", "自動分類", "コース別分類", "course folder 分類"],
    ["about", "このアプリについて", "ソースコード", "github"],
    ["about", "このアプリについて", "ライセンス", "license polyform noncommercial"],
    ["about", "このアプリについて", "第三者ライセンス", "third party notice dependency"],
    ["about", "アプリ更新", "現在", "version バージョン"],
    ["about", "アプリ更新", "進行状況", "update progress"],
    ["about", "アプリ更新", "新しいバージョン", "update release"],
    ["about", "アプリ更新", "手動更新", "manual download"],
    ["debug", "デバッグ", "通知状態", "notification debug log", true],
  ];
  const AUTO_SAVE_DEBOUNCE_MS = 1600;

  let settingSearchQuery = $state("");
  let baseNav = $derived(navItems.filter(n => !n.devOnly || $devModeActive));
  let visibleNav = $derived(baseNav);
  let searchResults = $derived.by(() => {
    const query = settingSearchQuery.trim().toLowerCase();
    if (!query) return [];
    return settingSearchItems
      .filter(item => !item[4] || $devModeActive)
      .filter(item => {
        const panelLabel = navItems.find(nav => nav.id === item[0])?.label || item[0];
        const haystack = [item[2], item[1], panelLabel, item[0], item[3] || ""].join(" ").toLowerCase();
        return haystack.includes(query);
      })
      .slice(0, 24);
  });
  let saveBusy = $state(false);
  let savePendingPanel = $state<SettingsPanel | null>(null);
  let saveState = $state<"idle" | "saving" | "saved" | "error">("idle");
  let saveHint = $state("変更は自動保存されます");
  let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let hintClearTimer: ReturnType<typeof setTimeout> | null = null;

  let aiPanel = $state<any>(null);
  let mailPanel = $state<any>(null);
  let calendarPanel = $state<any>(null);
  let notificationPanel = $state<any>(null);
  let downloadPanel = $state<any>(null);

  const saveEnabledPanels: SettingsPanel[] = ["ai", "mail", "calendar", "notification", "download"];
  let shouldShowSaveBar = $derived(saveEnabledPanels.includes($activeSettingsPanel));

  // If dev mode gets turned off while debug panel is active, bounce back to About
  $effect(() => {
    if (!$devModeActive && $activeSettingsPanel === "debug") {
      activeSettingsPanel.set("about");
    }
  });

  async function saveCurrentPanel(panel: SettingsPanel = $activeSettingsPanel) {
    if (!saveEnabledPanels.includes(panel)) return;
    if (saveBusy) {
      savePendingPanel = panel;
      return;
    }
    saveBusy = true;
    saveState = "saving";
    saveHint = "保存中...";
    try {
      switch (panel) {
        case "ai":
          await aiPanel?.save?.();
          break;
        case "mail":
          await mailPanel?.save?.();
          break;
        case "calendar":
          await calendarPanel?.save?.();
          break;
        case "notification":
          await notificationPanel?.save?.();
          break;
        case "download":
          await downloadPanel?.save?.();
          break;
      }
      saveState = "saved";
      saveHint = "自動保存しました";
      if (hintClearTimer) clearTimeout(hintClearTimer);
      hintClearTimer = setTimeout(() => {
        saveState = "idle";
        saveHint = "変更は自動保存されます";
      }, 1800);
    } catch {
      saveState = "error";
      saveHint = "保存に失敗しました";
    } finally {
      saveBusy = false;
      if (savePendingPanel) {
        const next = savePendingPanel;
        savePendingPanel = null;
        if (next === $activeSettingsPanel) {
          void saveCurrentPanel(next);
        }
      }
    }
  }

  function queueAutoSave() {
    if (!shouldShowSaveBar) return;
    const panelAtTrigger = $activeSettingsPanel;
    if (autoSaveTimer) clearTimeout(autoSaveTimer);
    autoSaveTimer = setTimeout(() => {
      if (panelAtTrigger === $activeSettingsPanel) {
        void saveCurrentPanel(panelAtTrigger);
      }
    }, AUTO_SAVE_DEBOUNCE_MS);
  }

  async function switchPanel(panel: SettingsPanel) {
    if (panel === $activeSettingsPanel) return;
    if (saveEnabledPanels.includes($activeSettingsPanel)) {
      if (autoSaveTimer) {
        clearTimeout(autoSaveTimer);
        autoSaveTimer = null;
      }
      await saveCurrentPanel($activeSettingsPanel);
    }
    activeSettingsPanel.set(panel);
  }

  async function openSearchResult(panel: SettingsPanel) {
    await switchPanel(panel);
    settingSearchQuery = "";
  }

  function panelLabel(panel: SettingsPanel) {
    return navItems.find(item => item.id === panel)?.label || panel;
  }

  onDestroy(() => {
    if (autoSaveTimer) clearTimeout(autoSaveTimer);
    if (hintClearTimer) clearTimeout(hintClearTimer);
  });

  function iconPath(id: string): string {
    switch (id) {
      case "sparkles":
        return "M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2zM15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z";
      case "lock":
        return "M3 5h14v10H3zM7 5V3.5a3 3 0 016 0V5";
      case "envelope":
        return "M2 4h16v12H2zM2 4l8 7 8-7";
      case "calendar":
        return "M2.5 3.5h15v13h-15zM2.5 7.5h15M6 2v3M14 2v3";
      case "bell":
        return "M10 2.5a5 5 0 015 5v3l1.5 2H3.5l1.5-2v-3a5 5 0 015-5zM8 14.5a2 2 0 004 0";
      case "arrow.down":
        return "M10 3v10M6 9l4 4 4-4M3 14v2a1 1 0 001 1h12a1 1 0 001-1v-2";
      case "info":
        return "M10 17.5a7.5 7.5 0 100-15 7.5 7.5 0 000 15zM10 9v4.5M10 7v0.1";
      case "wrench":
        return "M15 7a3 3 0 11-5 0 3 3 0 015 0zM12.5 9.5l-7 7M5 15l1.5 1.5";
      default:
        return "";
    }
  }
</script>

<div class="settings-root">
  <div class="settings-topbar">
    <div class="settings-title-wrap">
      <h1 class="settings-title">設定</h1>
      <div class="settings-subtitle">必要な項目を検索して、すばやく移動できます。</div>
    </div>
    <label class="settings-search" aria-label="設定項目を検索">
      <input
        type="search"
        bind:value={settingSearchQuery}
        placeholder="設定項目を検索"
        autocomplete="off"
        spellcheck="false"
      />
    </label>
  </div>

  <div class="settings-body">
    <aside class="settings-sidebar">
      <div class="nav-list">
        {#each visibleNav as item}
          <button
            class="nav-item"
            class:active={$activeSettingsPanel === item.id}
            onclick={() => { void switchPanel(item.id); }}
          >
            <svg class="nav-icon" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">
              <path d={iconPath(item.icon)} />
            </svg>
            <span class="nav-label">{item.label}</span>
          </button>
        {/each}
      </div>
    </aside>

    <main class="settings-main">
      <div class="settings-scroll" oninput={queueAutoSave} onchange={queueAutoSave}>
        {#if settingSearchQuery.trim()}
          <div class="search-results-panel">
            <div class="search-results-head">
              <div class="card-label">検索結果</div>
              <div class="search-count">{searchResults.length} 件</div>
            </div>
            {#if searchResults.length}
              <div class="search-results-list">
                {#each searchResults as result}
                  <button class="search-result" onclick={() => { void openSearchResult(result[0]); }}>
                    <div class="search-result-main">
                      <span class="search-result-title">{result[2]}</span>
                      <span class="search-result-group">{result[1]}</span>
                    </div>
                    <span class="search-result-panel">{panelLabel(result[0])}</span>
                  </button>
                {/each}
              </div>
            {:else}
              <div class="search-empty">該当する設定項目がありません</div>
            {/if}
          </div>
        {:else if $activeSettingsPanel === "ai"}
          <SettingsAi bind:this={aiPanel} />
        {:else if $activeSettingsPanel === "session"}
          <SettingsSession />
        {:else if $activeSettingsPanel === "mail"}
          <SettingsMail bind:this={mailPanel} />
        {:else if $activeSettingsPanel === "calendar"}
          <SettingsCalendar bind:this={calendarPanel} />
        {:else if $activeSettingsPanel === "notification"}
          <SettingsNotification bind:this={notificationPanel} />
        {:else if $activeSettingsPanel === "download"}
          <SettingsDownload bind:this={downloadPanel} />
        {:else if $activeSettingsPanel === "about"}
          <SettingsAbout />
        {:else if $activeSettingsPanel === "debug" && $devModeActive}
          <SettingsDebug />
        {/if}
      </div>

      {#if shouldShowSaveBar}
        <div class="settings-bottom-save">
          <span class="auto-save-hint" class:saving={saveState === "saving"} class:saved={saveState === "saved"} class:error={saveState === "error"}>
            {saveHint}
          </span>
        </div>
      {/if}
    </main>
  </div>
</div>

<style>
  .settings-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    margin: -24px;
    background: transparent;
  }

  .settings-topbar {
    display: flex;
    align-items: center;
    gap: 18px;
    flex-shrink: 0;
    padding: 16px 24px 14px;
    border-bottom: 0.5px solid var(--border);
  }

  .settings-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .settings-sidebar {
    width: 168px;
    flex-shrink: 0;
    padding: 16px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-right: 0.5px solid var(--border);
    overflow-y: auto;
  }

  .nav-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 7px;
    font-size: 12px;
    font-weight: 400;
    color: var(--text-primary);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    transition: background 0.12s, color 0.12s;
  }

  .nav-item:hover {
    background: var(--bg-hover);
  }

  .nav-item.active {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
    font-weight: 500;
  }

  .nav-icon {
    width: 15px;
    height: 15px;
    flex-shrink: 0;
    opacity: 0.75;
  }

  .nav-item.active .nav-icon {
    opacity: 1;
  }

  .nav-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .settings-main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding: 18px 24px 0;
  }

  .settings-title-wrap {
    flex: 1;
    min-width: 0;
  }

  .settings-title {
    margin: 0;
    font-size: 22px;
    line-height: 1.15;
    font-weight: 700;
    letter-spacing: 0;
    color: var(--text-primary);
  }

  .settings-subtitle {
    margin-top: 3px;
    font-size: 11px;
    line-height: 1.35;
    color: var(--text-tertiary);
  }

  .settings-search {
    position: relative;
    display: flex;
    align-items: center;
    width: min(280px, 42%);
    min-width: 190px;
    height: 32px;
    border-radius: 8px;
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    color: var(--text-tertiary);
    transition: border-color 0.15s, box-shadow 0.15s, background 0.15s;
  }

  .settings-search:focus-within {
    background: var(--bg-primary);
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .settings-search input {
    width: 100%;
    min-width: 0;
    height: 100%;
    padding: 0 12px;
    border: none;
    outline: none;
    border-radius: inherit;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: 12px;
    -webkit-user-select: text;
    user-select: text;
  }

  .settings-search input::placeholder {
    color: var(--text-tertiary);
  }

  .settings-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 14px;
  }

  .search-results-panel {
    padding-bottom: 12px;
  }

  .search-results-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  .search-results-head .card-label {
    margin-top: 0;
  }

  .search-count {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .search-results-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .search-result {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    padding: 10px 12px;
    border: 0.5px solid var(--border);
    border-radius: 8px;
    background: var(--bg-secondary);
    color: var(--text-primary);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, transform 0.12s;
  }

  .search-result:hover {
    background: var(--bg-hover);
    border-color: var(--border-strong);
  }

  .search-result-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .search-result-title {
    font-size: 12.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-result-group {
    font-size: 10.5px;
    color: var(--text-tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-result-panel {
    flex-shrink: 0;
    font-size: 10.5px;
    color: var(--accent);
  }

  .search-empty {
    padding: 28px 12px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-tertiary);
    text-align: center;
    background: var(--bg-secondary);
    border-radius: 8px;
  }

  .settings-bottom-save {
    display: flex;
    justify-content: flex-end;
    padding: 10px 0 12px;
    background: var(--bg-primary);
    border-top: 0.5px solid var(--border);
  }

  .auto-save-hint {
    font-size: 11px;
    color: var(--text-secondary);
    transition: color 0.15s ease;
  }

  .auto-save-hint.saving {
    color: var(--accent);
  }

  .auto-save-hint.saved {
    color: var(--green, #34c759);
    font-weight: 600;
  }

  .auto-save-hint.error {
    color: var(--red);
  }

  :global(.settings-main .hero-card) {
    background: var(--bg-secondary);
    border-radius: 10px;
    box-shadow: 0 0.5px 1px rgba(0,0,0,0.04), 0 1px 3px rgba(0,0,0,0.04);
    margin-bottom: 16px;
    padding: 10px 14px;
    display: flex;
    gap: 10px;
  }
  :global(.settings-main .hero-icon) {
    width: 30px; height: 30px;
    border-radius: 7px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.15), rgba(0, 122, 255, 0.15));
  }
  :global(.settings-main .hero-icon svg) {
    width: 18px; height: 18px;
  }
  :global(.settings-main .hero-text) { flex: 1; min-width: 0; }
  :global(.settings-main .panel-title) {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.01em;
    margin: 0;
  }
  :global(.settings-main .panel-desc) {
    font-size: 10.5px;
    color: var(--text-secondary);
    line-height: 1.4;
  }
  :global(.settings-main .card-label) {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.01em;
    padding: 0;
    margin: 14px 2px 6px;
  }
  :global(.settings-main .card-label:first-child) { margin-top: 0; }
  :global(.settings-main .card) {
    background: var(--bg-secondary);
    border-radius: 10px;
    box-shadow: 0 0.5px 1px rgba(0,0,0,0.04), 0 1px 3px rgba(0,0,0,0.04);
    margin-bottom: 10px;
    overflow: hidden;
  }
  :global(.settings-main .row) {
    padding: 9px 14px;
    border-top: 0.5px solid var(--border);
    display: flex;
    align-items: center;
    gap: 10px;
  }
  :global(.settings-main .card > .row:first-child) { border-top: none; }
  :global(.settings-main .row-label) {
    font-size: 12px;
    color: var(--text-primary);
    white-space: nowrap;
    min-width: 86px;
  }
  :global(.settings-main .row-input) { flex: 1; min-width: 0; }
  :global(.settings-main .row-input input),
  :global(.settings-main .row-input select) {
    width: 100%;
    padding: 5px 8px;
    font-size: 12px;
    font-family: inherit;
    color: var(--text-primary);
    background: var(--bg-tertiary);
    border: 0.5px solid var(--border-strong);
    border-radius: 8px;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  :global(.settings-main .row-input input:focus),
  :global(.settings-main .row-input select:focus) {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent);
  }
  :global(.settings-main .row-input input[type="password"]) {
    font-family: monospace;
    letter-spacing: 0.05em;
  }
  :global(.settings-main .hint) {
    font-size: 10.5px;
    color: var(--text-tertiary);
    margin-top: 3px;
    line-height: 1.4;
  }
  :global(.settings-main .btn-test) {
    flex-shrink: 0;
    padding: 5px 10px;
    font-size: 10.5px;
    font-weight: 600;
    font-family: inherit;
    border-radius: 6px;
    cursor: pointer;
    border: 0.5px solid var(--border-strong);
    background: var(--bg-hover);
    color: var(--text-secondary);
    white-space: nowrap;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  :global(.settings-main .btn-test:hover:not(:disabled)) {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  :global(.settings-main .btn-test:disabled) {
    opacity: 0.5;
    cursor: not-allowed;
  }
  :global(.settings-main .session-indicator) {
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  :global(.settings-main .session-dot) {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  :global(.settings-main .session-dot.ok) { background: var(--green, #34c759); }
  :global(.settings-main .session-dot.ng) { background: var(--red); }
  :global(.settings-main .spinner-sm) {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: settings-spin 0.6s linear infinite;
  }
  :global(.settings-main .status-msg) {
    font-size: 11px;
    line-height: 1.4;
  }
  :global(.settings-main .status-msg.success) { color: var(--green, #34c759); }
  :global(.settings-main .status-msg.error) { color: var(--red); }
  :global(.settings-main .status-msg.loading) { color: var(--text-secondary); }

  @keyframes settings-spin {
    to { transform: rotate(360deg); }
  }
</style>
