<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";
  import { getAiConfig, isDemoActive, updateAiReadiness } from "../../api";

  interface LocalModel {
    id: string;
    name: string;
    size_label: string;
    file_size_mb: number;
    downloaded: boolean;
  }

  interface SttModel {
    id: string;
    name: string;
    size_label: string;
    file_size_mb: number;
    downloaded: boolean;
  }

  interface SttConfig {
    selected_model: string;
    language: string;
    execution_backend: string;
    partial_mode: string;
    sensitivity: string;
  }

  interface SttExecutionBackendOption {
    id: string;
    label: string;
    description: string;
    experimental: boolean;
    available: boolean;
    availability_note?: string | null;
  }

  interface AiConfig {
    ai_enabled: boolean;
    provider: string;
    local_model: string;
    api_key: string;
    model: string;
    base_url: string;
    max_tokens: number;
    temperature: number;
    reply_language: string;
    ai_refresh_interval: number;
    live_summary_interval_minutes: number;
  }

  const MODEL_PRESETS: Record<string, string[]> = {
    openai: ["gpt-5.4", "gpt-5.4-mini", "gpt-5.4-nano"],
    gemini: ["gemini-3.1-pro-preview", "gemini-3-flash-preview"],
  };
  const PROVIDER_HINTS: Record<string, string> = {
    openai: "OpenAI API キーは platform.openai.com で取得できます。",
    gemini: "Google AI Studio (aistudio.google.com) で API キーを取得できます。",
  };
  const DEFAULT_URLS: Record<string, string> = { openai: "https://api.openai.com/v1", gemini: "" };
  const DEFAULT_MODELS: Record<string, string> = { openai: "gpt-5.4-nano", gemini: "gemini-3-flash-preview" };
  const DEFAULT_STT_BACKEND_OPTIONS: SttExecutionBackendOption[] = [
    {
      id: "cpu",
      label: "CPU",
      description: "現在の安定ルートです。すべてのビルドで利用できます。",
      experimental: false,
      available: true,
    },
  ];
  const STT_PARTIAL_MODE_OPTIONS = [
    {
      id: "balanced",
      label: "標準",
      description: "現在の partial 更新頻度です。応答性を優先します。",
    },
    {
      id: "power_saver",
      label: "省電",
      description: "partial 更新間隔を広げて、識別中の消費電力を抑えます。",
    },
    {
      id: "final_only",
      label: "最省電",
      description: "partial を止めて final のみ表示します。最も省電力ですが途中経過は出ません。",
    },
  ] as const;
  const STT_SENSITIVITY_OPTIONS = [
    {
      id: "low",
      label: "控えめ",
      description: "小さな物音や相槌を拾いにくくなります。静かな発話は取りこぼす場合があります。",
    },
    {
      id: "normal",
      label: "標準",
      description: "現在の既定値。ほとんどのシーンに適したバランスです。",
    },
    {
      id: "high",
      label: "高感度",
      description: "控えめな声やごく短い発話も積極的に拾います。周囲が騒がしいと誤反応が増えることがあります。",
    },
  ] as const;
  const DEMO_STT_CONFIG_KEY = "selah-demo-stt-config";
  const DEMO_NATIVE_AGENT_KEY = "selah-demo-native-agent-config";
  const DEMO_LOCAL_MODELS_KEY = "selah-demo-local-models";
  const DEMO_STT_MODELS_KEY = "selah-demo-stt-models";

  const DEMO_LOCAL_MODELS: LocalModel[] = [
    { id: "qwen3.5-8b", name: "Qwen 3.5 8B", size_label: "5.1 GB", file_size_mb: 5100, downloaded: true },
    { id: "qwen3.5-2b", name: "Qwen 3.5 2B", size_label: "1.5 GB", file_size_mb: 1500, downloaded: false },
    { id: "llama3.2-3b", name: "Llama 3.2 3B", size_label: "2.0 GB", file_size_mb: 2000, downloaded: false },
  ];

  const DEMO_STT_MODELS: SttModel[] = [
    { id: "sensevoice-ja-en", name: "SenseVoice JA/EN", size_label: "620 MB", file_size_mb: 620, downloaded: true },
    { id: "sensevoice-small", name: "SenseVoice Small", size_label: "310 MB", file_size_mb: 310, downloaded: false },
  ];

  let aiEnabled = $state("true");
  let aiProvider = $state("local");
  let selectedLocalModel = $state("qwen3.5-2b");
  let apiKey = $state("");
  let model = $state("");
  let baseUrl = $state("");
  let maxTokens = $state(0);
  let temperature = $state(0.7);
  let replyLanguage = $state("ja");
  let aiRefreshInterval = $state(360);
  let liveSummaryInterval = $state(5);

  let modelList = $state<LocalModel[]>([]);
  let downloading = $state(false);
  let downloadProgress = $state<{ modelId: string; name: string; percent: number; downloaded: number; total: number } | null>(null);
  let sttModelList = $state<SttModel[]>([]);
  let selectedSttModel = $state("sensevoice-ja-en");
  let sttLanguage = $state("ja");
  let selectedSttExecutionBackend = $state("cpu");
  let selectedSttPartialMode = $state("balanced");
  let selectedSttSensitivity = $state("normal");
  let sttExecutionBackendOptions = $state<SttExecutionBackendOption[]>(DEFAULT_STT_BACKEND_OPTIONS);
  let sttDownloading = $state(false);
  let sttDownloadProgress = $state<{ modelId: string; name: string; percent: number; downloaded: number; total: number } | null>(null);
  let sttTestBusy = $state(false);
  let sttTestMsg = $state("");
  let sttTestOk = $state<boolean | null>(null);
  let floatingOrbEnabled = $state("false");
  let subtitleOverlayEnabled = $state("false");
  let nativeOrbLoaded = false;
  let lastSavedFloatingOrbEnabled = "false";
  let lastSavedSubtitleOverlayEnabled = "false";

  let statusMsg = $state("");
  let statusType = $state<"success" | "error" | "loading" | "">("");
  let saveBusy = $state(false);
  let testBusy = $state(false);
  let localTestBusy = $state(false);
  let localTestMsg = $state("");
  let localTestOk = $state<boolean | null>(null);

  let unlistenDlProgress: (() => void) | null = null;
  let unlistenSttDlProgress: (() => void) | null = null;

  function readDemoState<T>(key: string, fallback: T): T {
    try {
      const raw = localStorage.getItem(key);
      if (!raw) return fallback;
      const parsed = JSON.parse(raw);
      return Array.isArray(fallback) ? parsed : { ...fallback, ...parsed };
    } catch {
      return fallback;
    }
  }

  function writeDemoState<T>(key: string, value: T) {
    try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* ignore */ }
  }

  function isLocal() { return aiProvider === "local"; }

  function currentSttBackendOption() {
    return sttExecutionBackendOptions.find((option) => option.id === selectedSttExecutionBackend) || null;
  }

  function currentSttPartialModeOption() {
    return STT_PARTIAL_MODE_OPTIONS.find((option) => option.id === selectedSttPartialMode) || STT_PARTIAL_MODE_OPTIONS[0];
  }

  function currentSttSensitivityOption() {
    return STT_SENSITIVITY_OPTIONS.find((option) => option.id === selectedSttSensitivity) || STT_SENSITIVITY_OPTIONS[1];
  }

  function getConfig(): AiConfig {
    const p = aiProvider;
    return {
      ai_enabled: aiEnabled === "true",
      provider: p,
      local_model: selectedLocalModel,
      api_key: p === "local" ? "" : apiKey,
      model: p === "local" ? "" : model,
      base_url: p === "local" ? "" : baseUrl,
      max_tokens: Number(maxTokens) || 0,
      temperature: Number(temperature),
      reply_language: replyLanguage,
      ai_refresh_interval: Number(aiRefreshInterval) || 0,
      live_summary_interval_minutes: Number(liveSummaryInterval) || 5,
    };
  }

  function friendlyError(e: any): string {
    const s = String(e);
    if (s.includes("rate limit") || s.includes("429") || s.includes("Too Many")) return "リクエスト上限に達しました。しばらく待ってから再試行してください。";
    if (s.includes("401") || s.includes("Unauthorized") || s.includes("invalid_api_key")) return "APIキーが無効です。";
    if (s.includes("403") || s.includes("Forbidden")) return "アクセスが拒否されました。";
    if (s.includes("timeout") || s.includes("timed out")) return "接続がタイムアウトしました。";
    if (s.includes("network") || s.includes("dns") || s.includes("ECONNREFUSED")) return "ネットワークエラー。接続を確認してください。";
    if (s.includes("model_not_found") || s.includes("does not exist")) return "指定されたモデルが見つかりません。";
    const m = s.match(/API error \(\d+\):\s*(.*)/);
    if (m) return m[1];
    return s.length > 120 ? s.substring(0, 120) + "..." : s;
  }

  function showStatus(msg: string, type: "success" | "error" | "loading") {
    statusMsg = msg;
    statusType = type;
    if (type !== "loading") setTimeout(() => { statusMsg = ""; statusType = ""; }, 4000);
  }

  function onProviderSwitch() {
    if (!isLocal()) {
      const p = aiProvider;
      const pr = MODEL_PRESETS[p] || [];
      if (!model || !pr.includes(model)) model = DEFAULT_MODELS[p] || "";
      if (p === "openai" && !baseUrl) baseUrl = DEFAULT_URLS.openai;
    }
  }

  async function loadModelList() {
    if (isDemoActive()) {
      modelList = readDemoState(DEMO_LOCAL_MODELS_KEY, DEMO_LOCAL_MODELS);
      return;
    }
    try {
      modelList = await invoke<LocalModel[]>("list_local_models");
    } catch (e) {
      console.error("Failed to load models:", e);
    }
  }

  async function loadSttModelList() {
    if (isDemoActive()) {
      sttModelList = readDemoState(DEMO_STT_MODELS_KEY, DEMO_STT_MODELS);
      return;
    }
    try {
      sttModelList = await invoke<SttModel[]>("list_stt_models");
    } catch (e) {
      console.error("Failed to load STT models:", e);
    }
  }

  async function startDownload(modelId: string) {
    if (downloading) return;
    downloading = true;
    const m = modelList.find(x => x.id === modelId);
    if (!m) return;
    downloadProgress = { modelId, name: m.name, percent: 0, downloaded: 0, total: 0 };
    try {
      if (isDemoActive()) {
        const next = modelList.map((item) => item.id === modelId ? { ...item, downloaded: true } : item);
        writeDemoState(DEMO_LOCAL_MODELS_KEY, next);
        modelList = next;
      } else {
      await invoke("download_local_model", { modelId });
      }
      showStatus(m.name + " のダウンロードが完了しました", "success");
      await loadModelList();
      selectedLocalModel = modelId;
      updateAiReadiness().catch(() => {});
    } catch (e) {
      if (String(e) === "cancelled") showStatus("ダウンロードを中止しました", "error");
      else showStatus("ダウンロードエラー: " + friendlyError(e), "error");
    } finally {
      downloading = false;
      downloadProgress = null;
    }
  }

  function cancelDownload() {
    if (isDemoActive()) {
      downloading = false;
      downloadProgress = null;
      showStatus("ダウンロードを中止しました", "error");
      return;
    }
    invoke("cancel_model_download").catch(() => {});
  }

  async function startSttDownload(modelId: string) {
    if (sttDownloading) return;
    sttDownloading = true;
    const m = sttModelList.find(x => x.id === modelId);
    if (!m) return;
    sttDownloadProgress = { modelId, name: m.name, percent: 0, downloaded: 0, total: 0 };
    try {
      if (isDemoActive()) {
        const next = sttModelList.map((item) => item.id === modelId ? { ...item, downloaded: true } : item);
        writeDemoState(DEMO_STT_MODELS_KEY, next);
        sttModelList = next;
      } else {
      await invoke("download_stt_model", { modelId });
      }
      showStatus(m.name + " のダウンロードが完了しました", "success");
      await loadSttModelList();
      selectedSttModel = modelId;
    } catch (e) {
      if (String(e) === "cancelled") showStatus("STT モデルのダウンロードを中止しました", "error");
      else showStatus("STT ダウンロードエラー: " + friendlyError(e), "error");
    } finally {
      sttDownloading = false;
      sttDownloadProgress = null;
    }
  }

  function cancelSttDownload() {
    if (isDemoActive()) {
      sttDownloading = false;
      sttDownloadProgress = null;
      showStatus("STT モデルのダウンロードを中止しました", "error");
      return;
    }
    invoke("cancel_stt_model_download").catch(() => {});
  }

  async function deleteModel(modelId: string) {
    if (!confirm("このモデルを削除しますか？")) return;
    try {
      if (isDemoActive()) {
        const next = modelList.map((item) => item.id === modelId ? { ...item, downloaded: false } : item);
        writeDemoState(DEMO_LOCAL_MODELS_KEY, next);
        modelList = next;
      } else {
      await invoke("delete_local_model", { modelId });
      }
      showStatus("モデルを削除しました", "success");
      await loadModelList();
      updateAiReadiness().catch(() => {});
    } catch (e) {
      showStatus("削除エラー: " + String(e), "error");
    }
  }

  async function deleteSttModel(modelId: string) {
    if (!confirm("この STT モデルを削除しますか？")) return;
    try {
      if (isDemoActive()) {
        const next = sttModelList.map((item) => item.id === modelId ? { ...item, downloaded: false } : item);
        writeDemoState(DEMO_STT_MODELS_KEY, next);
        sttModelList = next;
      } else {
      await invoke("delete_stt_model", { modelId });
      }
      showStatus("STT モデルを削除しました", "success");
      await loadSttModelList();
    } catch (e) {
      showStatus("削除エラー: " + String(e), "error");
    }
  }

  async function loadConfig() {
    try {
      const c = await getAiConfig();
      const stt = isDemoActive()
        ? readDemoState<SttConfig>(DEMO_STT_CONFIG_KEY, {
            selected_model: "sensevoice-ja-en",
            language: "ja",
            execution_backend: "cpu",
            partial_mode: "balanced",
            sensitivity: "normal",
          })
        : await invoke<SttConfig>("get_stt_config");
      sttExecutionBackendOptions = isDemoActive()
        ? DEFAULT_STT_BACKEND_OPTIONS
        : await invoke<SttExecutionBackendOption[]>("list_stt_execution_backends");
      aiEnabled = c.ai_enabled !== false ? "true" : "false";
      aiProvider = c.provider || "local";
      selectedLocalModel = c.local_model || "qwen3.5-2b";
      apiKey = c.api_key || "";
      model = c.model || "";
      baseUrl = c.base_url || "";
      maxTokens = c.max_tokens != null ? c.max_tokens : 0;
      temperature = c.temperature != null ? c.temperature : 0.7;
      replyLanguage = c.reply_language || "ja";
      aiRefreshInterval = c.ai_refresh_interval != null ? c.ai_refresh_interval : 360;
      liveSummaryInterval = c.live_summary_interval_minutes != null ? c.live_summary_interval_minutes : 5;
      selectedSttModel = stt?.selected_model || "sensevoice-ja-en";
      sttLanguage = stt?.language || "ja";
      selectedSttExecutionBackend = stt?.execution_backend || "cpu";
      selectedSttPartialMode = stt?.partial_mode || "balanced";
      selectedSttSensitivity = stt?.sensitivity || "normal";
      if (!sttExecutionBackendOptions.some((option) => option.id === selectedSttExecutionBackend && option.available)) {
        selectedSttExecutionBackend = sttExecutionBackendOptions.find((option) => option.available)?.id || "cpu";
      }
      if (!STT_PARTIAL_MODE_OPTIONS.some((option) => option.id === selectedSttPartialMode)) {
        selectedSttPartialMode = STT_PARTIAL_MODE_OPTIONS[0].id;
      }
      if (!STT_SENSITIVITY_OPTIONS.some((option) => option.id === selectedSttSensitivity)) {
        selectedSttSensitivity = "normal";
      }
      const nativeAgent = isDemoActive()
        ? readDemoState<{ floating_orb_enabled?: boolean; subtitle_overlay_enabled?: boolean }>(DEMO_NATIVE_AGENT_KEY, { floating_orb_enabled: false, subtitle_overlay_enabled: false })
        : await invoke<{ floating_orb_enabled?: boolean; subtitle_overlay_enabled?: boolean }>("get_native_agent_config");
      floatingOrbEnabled = nativeAgent?.floating_orb_enabled ? "true" : "false";
      lastSavedFloatingOrbEnabled = floatingOrbEnabled;
      subtitleOverlayEnabled = nativeAgent?.subtitle_overlay_enabled ? "true" : "false";
      lastSavedSubtitleOverlayEnabled = subtitleOverlayEnabled;
      nativeOrbLoaded = true;
      await loadModelList();
      await loadSttModelList();
    } catch (e) {
      console.error("Failed to load config:", e);
    }
  }

  export async function save() {
    saveBusy = true;
    try {
      if (isDemoActive()) {
        writeDemoState("selah-demo-ai-config", getConfig());
        writeDemoState(DEMO_STT_CONFIG_KEY, {
          selected_model: selectedSttModel,
          language: sttLanguage,
          execution_backend: selectedSttExecutionBackend,
          partial_mode: selectedSttPartialMode,
          sensitivity: selectedSttSensitivity,
        });
        writeDemoState(DEMO_NATIVE_AGENT_KEY, {
          floating_orb_enabled: floatingOrbEnabled === "true",
          subtitle_overlay_enabled: subtitleOverlayEnabled === "true",
        });
      } else {
        await invoke("save_ai_config", { config: getConfig() });
        await invoke("save_stt_config", {
          config: {
            selected_model: selectedSttModel,
            language: sttLanguage,
            execution_backend: selectedSttExecutionBackend,
            partial_mode: selectedSttPartialMode,
            sensitivity: selectedSttSensitivity,
          },
        });
        await invoke("save_native_agent_config", {
          config: { floating_orb_enabled: floatingOrbEnabled === "true", subtitle_overlay_enabled: subtitleOverlayEnabled === "true" },
        });
      }
      lastSavedFloatingOrbEnabled = floatingOrbEnabled;
      lastSavedSubtitleOverlayEnabled = subtitleOverlayEnabled;
      updateAiReadiness().catch(() => {});
    } catch (e) {
      throw e;
    } finally {
      saveBusy = false;
    }
  }

  async function testConnection() {
    testBusy = true;
    showStatus("接続をテスト中...", "loading");
    try {
      let r = "演示モード接続 OK";
      if (isDemoActive()) {
        writeDemoState("selah-demo-ai-config", getConfig());
      } else {
        await invoke("save_ai_config", { config: getConfig() });
        r = await invoke<string>("ai_test_connection");
      }
      showStatus("接続成功: " + r.substring(0, 80), "success");
    } catch (e) {
      showStatus(friendlyError(e), "error");
    } finally {
      testBusy = false;
    }
  }

  async function testLocalModel() {
    localTestBusy = true;
    localTestOk = null;
    localTestMsg = "モデルを読み込み中...";
    try {
      let r = "演示モードのローカルモデルは利用可能です";
      if (isDemoActive()) {
        writeDemoState("selah-demo-ai-config", getConfig());
      } else {
        await invoke("save_ai_config", { config: getConfig() });
        r = await invoke<string>("ai_test_connection");
      }
      localTestOk = true;
      localTestMsg = "成功: " + r.substring(0, 80);
    } catch (e) {
      localTestOk = false;
      localTestMsg = "エラー: " + (typeof e === "string" ? e : (e as any)?.message || JSON.stringify(e));
    } finally {
      localTestBusy = false;
    }
  }

  async function testSttModel() {
    sttTestBusy = true;
    sttTestOk = null;
    sttTestMsg = "STT モデルを読み込み中...";
    try {
      let r = "演示モードの STT モデルは利用可能です";
      if (isDemoActive()) {
        writeDemoState(DEMO_STT_CONFIG_KEY, {
          selected_model: selectedSttModel,
          language: sttLanguage,
          execution_backend: selectedSttExecutionBackend,
          partial_mode: selectedSttPartialMode,
          sensitivity: selectedSttSensitivity,
        });
      } else {
        await invoke("save_stt_config", {
          config: {
            selected_model: selectedSttModel,
            language: sttLanguage,
            execution_backend: selectedSttExecutionBackend,
            partial_mode: selectedSttPartialMode,
            sensitivity: selectedSttSensitivity,
          },
        });
        r = await invoke<string>("stt_test_model");
      }
      sttTestOk = true;
      sttTestMsg = r;
    } catch (e) {
      sttTestOk = false;
      sttTestMsg = "エラー: " + (typeof e === "string" ? e : (e as any)?.message || JSON.stringify(e));
    } finally {
      sttTestBusy = false;
    }
  }

  onMount(async () => {
    await loadConfig();
    if (isDemoActive()) return;
    unlistenDlProgress = await listen<{ percent: number; downloaded: number; total: number }>(
      "model-download-progress",
      (ev) => {
        if (!downloadProgress) return;
        downloadProgress = { ...downloadProgress, percent: ev.payload.percent ?? downloadProgress.percent, downloaded: ev.payload.downloaded ?? 0, total: ev.payload.total ?? 0 };
      }
    );
    unlistenSttDlProgress = await listen<{ percent: number; downloaded: number; total: number }>(
      "stt-model-download-progress",
      (ev) => {
        if (!sttDownloadProgress) return;
        sttDownloadProgress = { ...sttDownloadProgress, percent: ev.payload.percent ?? sttDownloadProgress.percent, downloaded: ev.payload.downloaded ?? 0, total: ev.payload.total ?? 0 };
      }
    );
  });

  onDestroy(() => { unlistenDlProgress?.(); unlistenSttDlProgress?.(); });

  // React to provider/value changes to apply defaults
  $effect(() => { onProviderSwitch(); void aiProvider; });
  $effect(() => {
    if (!nativeOrbLoaded) return;
    if (floatingOrbEnabled === lastSavedFloatingOrbEnabled && subtitleOverlayEnabled === lastSavedSubtitleOverlayEnabled) return;
    lastSavedFloatingOrbEnabled = floatingOrbEnabled;
    lastSavedSubtitleOverlayEnabled = subtitleOverlayEnabled;
    if (isDemoActive()) {
      writeDemoState(DEMO_NATIVE_AGENT_KEY, {
        floating_orb_enabled: floatingOrbEnabled === "true",
        subtitle_overlay_enabled: subtitleOverlayEnabled === "true",
      });
      return;
    }
    invoke("save_native_agent_config", {
      config: { floating_orb_enabled: floatingOrbEnabled === "true", subtitle_overlay_enabled: subtitleOverlayEnabled === "true" },
    }).catch((e) => {
      console.error("Failed to save native agent config:", e);
    });
  });
</script>

<div class="hero-card">
  <div class="hero-icon">
    <svg viewBox="0 0 20 20" fill="none" stroke="#6a3fa0" stroke-width="1.3" stroke-linejoin="round">
      <path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z"/>
      <path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke-width="1"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">AI 設定</h2>
    <p class="panel-desc">AI の有効化、推論方式、モデル、応答言語、自動更新をまとめて管理します。ローカル実行と外部 API のどちらでも、時間割・課題・通知・LIVE 要約まわりの AI 機能をここで整えられます。</p>
  </div>
</div>

<div class="card-label">AI アシスタント</div>
<div class="card">
  <div class="row">
    <span class="row-label">AI 機能</span>
    <div class="row-input">
      <select bind:value={aiEnabled}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
      <div class="hint">有効にすると、時間割・課題・通知・LIVE まわりの AI 機能が使えるようになります。</div>
    </div>
  </div>
  {#if aiEnabled === "true"}
    <div class="row">
      <span class="row-label">回答言語</span>
      <div class="row-input">
        <select bind:value={replyLanguage}>
          <option value="ja">日本語</option>
          <option value="zh">中文</option>
          <option value="en">English</option>
          <option value="ko">한국어</option>
        </select>
        <div class="hint">AI の回答や要約をどの言語で返すかを指定します。</div>
      </div>
    </div>
    <div class="row">
      <span class="row-label">更新間隔</span>
      <div class="row-input">
        <select bind:value={aiRefreshInterval}>
          <option value={0}>無効</option>
          <option value={60}>1時間</option>
          <option value={120}>2時間</option>
          <option value={180}>3時間</option>
          <option value={360}>6時間</option>
          <option value={720}>12時間</option>
          <option value={1440}>24時間</option>
        </select>
        <div class="hint">AI 通知分析と AI 課題分析を自動で再計算する間隔です。</div>
      </div>
    </div>
    <div class="row">
      <span class="row-label">LIVE 要約の生成間隔</span>
      <div class="row-input">
        <select bind:value={liveSummaryInterval}>
          <option value={5}>5分</option>
          <option value={10}>10分</option>
          <option value={15}>15分</option>
          <option value={20}>20分</option>
          <option value={25}>25分</option>
          <option value={30}>30分</option>
        </select>
        <div class="hint">LIVEページで自動的に分割要約を作る間隔です。要約の言語は上の「回答言語」に従います。</div>
      </div>
    </div>
  {/if}
</div>

{#if aiEnabled === "true"}
  <div class="card-label">AI アシスタントの推論</div>
  <div class="card section-intro-card">
    <div class="section-intro">
      ホーム・課題・通知・LIVE 要約で使う推論設定です。{isLocal() ? "ローカルモデルなら端末内で完結します。" : "外部 API を使う場合は接続先とモデルをここで調整します。"}
    </div>
  </div>
  <div class="card">
    <div class="row">
      <span class="row-label">推論方法</span>
      <div class="row-input">
        <select bind:value={aiProvider}>
          <option value="local">ローカルモデル</option>
          <option value="openai">OpenAI API</option>
          <option value="gemini">Google Gemini API</option>
        </select>
        <div class="hint">
          {isLocal() ? "ローカルモデルはこのデバイス上で実行されます。通信なしで使いたいときに向いています。" : "API 利用時のみ、API キー・モデル名・ベース URL・最大トークン・Temperature が有効です。"}
        </div>
      </div>
    </div>
  </div>
  {#if isLocal()}
    <div class="card-label">AI アシスタントのモデル</div>
    <div class="card">
      {#each modelList as m}
        <div class="model-row">
          <input type="radio" class="model-radio" name="localModel" value={m.id} bind:group={selectedLocalModel} />
          <div class="model-info">
            <div class="model-name">{m.name}</div>
            <div class="model-meta">{m.size_label}</div>
            {#if m.id === "qwen3.5-2b"}
              <div class="model-meta model-note-standard">標準: ホームのお知らせAI要約 / 時間割のAI日程分析は利用不可</div>
            {:else if m.id === "qwen3.5-4b"}
              <div class="model-meta model-note-high">高品質: すべてのAI分析機能を利用可能</div>
            {/if}
          </div>
          <span class="model-badge" class:downloaded={m.downloaded}>
            {m.downloaded ? "DL済み" : m.file_size_mb + "MB"}
          </span>
          <div class="model-actions">
            {#if !m.downloaded}
              <button class="btn-test" onclick={() => startDownload(m.id)} disabled={downloading}>ダウンロード</button>
            {:else}
              <button class="btn-test danger" onclick={() => deleteModel(m.id)}>削除</button>
            {/if}
          </div>
        </div>
      {/each}
      {#if modelList.length === 0}
        <div class="model-empty">モデル情報を読み込み中...</div>
      {/if}
    </div>
    {#if downloadProgress}
      <div class="card download-progress">
        <div class="dl-head">
          <span class="dl-name">{downloadProgress.name}</span>
          <span class="dl-percent">{downloadProgress.percent}%</span>
          <button class="btn-test danger" onclick={cancelDownload}>中止</button>
        </div>
        <div class="dl-bar"><div class="dl-fill" style="width:{downloadProgress.percent}%"></div></div>
        {#if downloadProgress.total}
          <div class="dl-size">{(downloadProgress.downloaded / 1048576).toFixed(1)} / {(downloadProgress.total / 1048576).toFixed(1)} MB</div>
        {/if}
      </div>
    {/if}
    <div class="action-row">
      <button class="btn-test" onclick={testLocalModel} disabled={localTestBusy}>
        {localTestBusy ? "テスト中..." : "推論テスト"}
      </button>
      {#if localTestMsg}
        <span class="hint" class:ok={localTestOk === true} class:ng={localTestOk === false}>{localTestMsg}</span>
      {/if}
    </div>
  {:else}
    <div class="card-label">AI アシスタントの API 設定</div>
    <div class="card">
      <div class="row">
        <span class="row-label">API キー</span>
        <div class="row-input">
          <div class="key-row">
            <input type="password" bind:value={apiKey} placeholder="APIキーを入力" autocomplete="off" spellcheck="false" />
            <button class="btn-test" onclick={testConnection} disabled={testBusy}>テスト</button>
          </div>
          <div class="hint">{PROVIDER_HINTS[aiProvider] || ""}</div>
        </div>
      </div>
      <div class="row">
        <span class="row-label">モデル名</span>
        <div class="row-input">
          <input type="text" bind:value={model} placeholder="gpt-5.4-nano" spellcheck="false" />
          <div class="presets">
            {#each (MODEL_PRESETS[aiProvider] || []) as preset}
              <button class="preset" onclick={() => (model = preset)}>{preset}</button>
            {/each}
          </div>
        </div>
      </div>
      {#if aiProvider !== "gemini"}
        <div class="row">
          <span class="row-label">ベース URL</span>
          <div class="row-input">
            <input type="text" bind:value={baseUrl} placeholder="https://api.openai.com/v1" spellcheck="false" />
            <div class="hint">OpenAI 互換 API のエンドポイント</div>
          </div>
        </div>
      {/if}
      <div class="row">
        <span class="row-label">最大トークン</span>
        <div class="row-input">
          <input type="number" bind:value={maxTokens} min="0" step="1024" placeholder="0" />
          <div class="hint">API モデル用の出力上限です。</div>
        </div>
      </div>
      <div class="row">
        <span class="row-label">Temperature</span>
        <div class="row-input">
          <div class="range-row">
            <input type="range" min="0" max="2" step="0.1" bind:value={temperature} />
            <span class="range-val">{Number(temperature).toFixed(1)}</span>
          </div>
          <div class="hint">API モデル用のサンプリング設定です。</div>
        </div>
      </div>
    </div>
  {/if}

{/if}

<div class="card-label">Agent フローティング</div>
<div class="card">
  <div class="row">
    <span class="row-label">フローティング入口</span>
    <div class="row-input">
      <select bind:value={floatingOrbEnabled}>
        <option value="false">無効</option>
        <option value="true">有効</option>
      </select>
      <div class="hint">macOS ネイティブの Agent フローティング入口を使います。無効にすると、表示中の入口もすぐ閉じます。</div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">リアルタイム字幕</span>
    <div class="row-input">
      <select bind:value={subtitleOverlayEnabled}>
        <option value="false">無効</option>
        <option value="true">有効</option>
      </select>
      <div class="hint">macOS / Windows ネイティブのリアルタイム字幕浮窗を表示します。Live 録課セッション中に最新の文字起こし内容を画面下部に表示します。</div>
    </div>
  </div>
</div>

<div class="card-label">AI 音声文字起こし</div>
<div class="card">
  <div class="row">
    <span class="row-label">STT 実行バックエンド</span>
    <div class="row-input">
      <select bind:value={selectedSttExecutionBackend}>
        {#each sttExecutionBackendOptions as option}
          <option value={option.id} disabled={!option.available}>
            {option.label}{option.experimental ? "（実験）" : ""}{option.available ? "" : " - 利用不可"}
          </option>
        {/each}
      </select>
      <div class="hint">{currentSttBackendOption()?.description || "現在の安定ルートは CPU です。"}</div>
      {#if currentSttBackendOption()?.availability_note}
        <div class="hint">{currentSttBackendOption()?.availability_note}</div>
      {/if}
    </div>
  </div>
  <div class="row">
    <span class="row-label">AI 音声認識言語</span>
    <div class="row-input">
      <select bind:value={sttLanguage}>
        <option value="ja">日本語</option>
        <option value="zh">中文</option>
        <option value="en">English</option>
        <option value="ko">한국어</option>
        <option value="yue">広東語</option>
        <option value="auto">自動検出</option>
      </select>
      <div class="hint">音声認識の対象言語を選択します。「自動検出」は誤認識する場合があります。</div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">STT 省電モード</span>
    <div class="row-input">
      <select bind:value={selectedSttPartialMode}>
        {#each STT_PARTIAL_MODE_OPTIONS as option}
          <option value={option.id}>{option.label}</option>
        {/each}
      </select>
      <div class="hint">{currentSttPartialModeOption().description}</div>
      <div class="hint">保存後は、LIVE 録音中でも次の partial 更新判定からそのまま反映されます。</div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">音声感度</span>
    <div class="row-input">
      <select bind:value={selectedSttSensitivity}>
        {#each STT_SENSITIVITY_OPTIONS as option}
          <option value={option.id}>{option.label}</option>
        {/each}
      </select>
      <div class="hint">{currentSttSensitivityOption().description}</div>
      <div class="hint">高くするほど積極的に発話を検出します。変更は次回の録音開始から反映されます。</div>
    </div>
  </div>
  {#each sttModelList as m}
    <div class="model-row">
      <input type="radio" class="model-radio" name="sttModel" value={m.id} bind:group={selectedSttModel} />
      <div class="model-info">
        <div class="model-name">{m.name}</div>
        <div class="model-meta">{m.size_label}</div>
      </div>
      <span class="model-badge" class:downloaded={m.downloaded}>
        {m.downloaded ? "DL済み" : m.file_size_mb + "MB"}
      </span>
      <div class="model-actions">
        {#if !m.downloaded}
          <button class="btn-test" onclick={() => startSttDownload(m.id)} disabled={sttDownloading}>ダウンロード</button>
        {:else}
          <button class="btn-test danger" onclick={() => deleteSttModel(m.id)}>削除</button>
        {/if}
      </div>
    </div>
  {/each}
  {#if sttModelList.length === 0}
    <div class="model-empty">STT モデル情報を読み込み中...</div>
  {/if}
</div>
{#if sttDownloadProgress}
  <div class="card download-progress">
    <div class="dl-head">
      <span class="dl-name">{sttDownloadProgress.name}</span>
      <span class="dl-percent">{sttDownloadProgress.percent}%</span>
      <button class="btn-test danger" onclick={cancelSttDownload}>中止</button>
    </div>
    <div class="dl-bar"><div class="dl-fill" style="width:{sttDownloadProgress.percent}%"></div></div>
    {#if sttDownloadProgress.total}
      <div class="dl-size">{(sttDownloadProgress.downloaded / 1048576).toFixed(1)} / {(sttDownloadProgress.total / 1048576).toFixed(1)} MB</div>
    {/if}
  </div>
{/if}
<div class="action-row">
  <button class="btn-test" onclick={testSttModel} disabled={sttTestBusy}>
    {sttTestBusy ? "テスト中..." : "AI 音声文字起こしテスト"}
  </button>
  {#if sttTestMsg}
    <span class="hint" class:ok={sttTestOk === true} class:ng={sttTestOk === false}>{sttTestMsg}</span>
  {/if}
</div>

{#if statusMsg}
  <div class="status-msg {statusType}" style="margin-top:10px;">{statusMsg}</div>
{/if}

<style>
  .section-intro-card {
    padding: 12px 14px;
  }
  .section-intro {
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .model-row {
    padding: 10px 14px;
    border-top: 0.5px solid var(--border);
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .model-row:first-child { border-top: none; }
  .model-radio { width: 14px; height: 14px; accent-color: var(--accent); cursor: pointer; flex-shrink: 0; }
  .model-info { flex: 1; min-width: 0; }
  .model-name { font-size: 12px; font-weight: 600; color: var(--text-primary); }
  .model-meta { font-size: 10.5px; color: var(--text-secondary); margin-top: 1px; }
  .model-note-standard { color: color-mix(in srgb, var(--text-primary) 72%, var(--red) 28%); font-weight: 500; }
  .model-note-high { color: color-mix(in srgb, var(--text-primary) 78%, var(--green, #34c759) 22%); font-weight: 500; }
  .model-badge {
    font-size: 9px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    white-space: nowrap;
  }
  .model-badge.downloaded {
    background: color-mix(in srgb, var(--green, #34c759) 15%, transparent);
    color: var(--green, #34c759);
  }
  .model-actions { display: flex; gap: 4px; flex-shrink: 0; }
  .model-empty { padding: 14px; font-size: 12px; color: var(--text-secondary); }

  :global(.settings-main .btn-test.danger) {
    color: var(--red);
    border-color: color-mix(in srgb, var(--red) 30%, transparent);
  }
  :global(.settings-main .btn-test.danger:hover:not(:disabled)) {
    background: var(--red);
    color: #fff;
    border-color: var(--red);
  }

  .download-progress { padding: 10px 14px; margin-top: -4px; }
  .dl-head { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .dl-name { font-size: 12px; font-weight: 600; flex: 1; }
  .dl-percent { font-size: 11px; color: var(--text-secondary); }
  .dl-bar {
    height: 4px;
    background: var(--border-strong);
    border-radius: 2px;
    overflow: hidden;
  }
  .dl-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.2s;
  }
  .dl-size {
    font-size: 10px;
    color: var(--text-tertiary);
    margin-top: 3px;
  }

  .action-row {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 10px;
  }
  .action-row .hint { margin-top: 0; }
  .action-row .hint.ok { color: var(--green, #34c759); }
  .action-row .hint.ng { color: var(--red); }

  .key-row { display: flex; align-items: center; gap: 6px; }
  .key-row input { flex: 1; }

  .presets { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 5px; }
  .preset {
    font-size: 10px;
    font-family: inherit;
    padding: 2px 8px;
    border-radius: 4px;
    border: 0.5px solid var(--border-strong);
    background: var(--bg-hover);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.12s;
  }
  .preset:hover {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }

  .range-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .range-row input[type="range"] {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: var(--border-strong);
    outline: none;
    border: none;
    padding: 0;
  }
  .range-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--accent);
    cursor: pointer;
  }
  .range-val {
    font-size: 12px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    min-width: 28px;
    text-align: right;
  }
</style>
