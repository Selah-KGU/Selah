<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    onboardingRecord,
    onboardingVisible,
    updateRecord,
    skipOnboarding,
    completeOnboarding,
    markResume,
    consumeResume,
    type OnboardingPurpose,
  } from "./onboardingState";
  import {
    getAiConfig,
    isDemoActive,
    openSettingsWindow,
    updateAiReadiness,
    resetAiReady,
  } from "../api";
  import {
    getAiReadinessLabel,
    isSttReady,
    loadOnboardingChecks,
    type OnboardingCheckRow,
    type OnboardingStatus,
  } from "./onboardingChecks";
  import { cacheStatus, activeTab } from "../stores";
  import { openExternalUrl } from "../system";
  import selahLogoUrl from "../../assets/logo.png";

  type Step = "welcome" | "purpose" | "provider" | "apikey" | "checklist" | "finish";

  let step = $state<Step>("welcome");
  let purposes = $state<OnboardingPurpose[]>([]);

  // AI step state
  type Provider = "openai" | "gemini";
  let provider = $state<Provider>("openai");
  let apiKey = $state("");
  let testing = $state(false);
  let testDone = $state(false);
  let testMsg = $state("");
  let testOk = $state<boolean | null>(null);

  const PROVIDER_DEFAULTS: Record<Provider, { model: string; baseUrl: string; keyUrl: string; hint: string }> = {
    openai: {
      model: "gpt-5.4-nano",
      baseUrl: "https://api.openai.com/v1",
      keyUrl: "https://platform.openai.com/api-keys",
      hint: "OpenAI API キーは platform.openai.com で取得できます。",
    },
    gemini: {
      model: "gemini-3-flash-preview",
      baseUrl: "",
      keyUrl: "https://aistudio.google.com/app/apikey",
      hint: "Google AI Studio で API キーを取得できます。",
    },
  };

  // Checklist state
  interface ChecklistRow {
    key: string;
    label: string;
    detail: string;
    status: OnboardingStatus;
    actionLabel: string;
    action: () => void | Promise<void>;
  }
  let checklistRows = $state<ChecklistRow[]>([]);
  let checklistLoading = $state(true);

  // Finish capability matrix
  interface CapabilityRow {
    key: string;
    label: string;
    group: "ai" | "base";
    status: "ok" | "warn" | "off";
    note: string;
    action?: { label: string; run: () => void };
  }
  let capabilities = $state<CapabilityRow[]>([]);

  onMount(() => {
    // Restore purposes if returning
    const rec = $onboardingRecord;
    if (rec.purposes?.length) purposes = [...rec.purposes];
  });

  // Re-apply resume token each time the modal becomes visible. The component
  // stays mounted in Dashboard, so onMount alone would never re-fire after a
  // settings detour and the sessionStorage token would leak.
  let wasVisible = false;
  $effect(() => {
    const visible = $onboardingVisible;
    if (visible && !wasVisible) {
      const resume = consumeResume();
      if (resume === "welcome" || resume === "purpose" || resume === "provider"
        || resume === "apikey" || resume === "checklist" || resume === "finish") {
        next(resume);
      }
    }
    wasVisible = visible;
  });

  function close() {
    onboardingVisible.set(false);
  }

  function next(target: Step) {
    step = target;
    if (target === "checklist") void loadChecklist();
    if (target === "finish") void loadCapabilities();
  }

  function togglePurpose(p: OnboardingPurpose) {
    if (purposes.includes(p)) purposes = purposes.filter(x => x !== p);
    else purposes = [...purposes, p];
  }

  function openExternal(url: string) {
    openExternalUrl(url, { allowInDemo: true }).catch(() => {});
  }

  function jumpToSettings(panel: string, resumeStep?: Step) {
    // Remember which onboarding step to come back to
    const resume = resumeStep || (step === "checklist" || step === "finish" ? step : null);
    if (resume) markResume(resume);
    openSettingsWindow(panel).catch(() => {});
    close();
  }

  // ===== Step: API key submit =====
  async function saveAndTest() {
    if (!apiKey.trim()) {
      testOk = false;
      testMsg = "API キーを入力してください。";
      return;
    }
    testing = true;
    testDone = false;
    testMsg = "接続をテスト中...";
    testOk = null;
    try {
      if (isDemoActive()) {
        testOk = true;
        testMsg = "デモモード：接続テストをスキップしました。";
      } else {
        const current = await getAiConfig();
        const defaults = PROVIDER_DEFAULTS[provider];
        const cfg = {
          ...current,
          ai_enabled: true,
          provider,
          api_key: apiKey.trim(),
          model: current.model && current.provider === provider ? current.model : defaults.model,
          base_url: current.base_url && current.provider === provider ? current.base_url : defaults.baseUrl,
        };
        await invoke("save_ai_config", { config: cfg });
        const r = await invoke<string>("ai_test_connection");
        testOk = true;
        testMsg = "接続成功：" + String(r).substring(0, 60);
        resetAiReady();
        await updateAiReadiness();
      }
      updateRecord({ purposes });
      testDone = true;
    } catch (e: any) {
      testOk = false;
      testMsg = friendlyError(e);
    } finally {
      testing = false;
    }
  }

  function friendlyError(e: any): string {
    const s = String(e?.message || e);
    if (/401|Unauthorized|invalid_api_key/i.test(s)) return "API キーが無効です。";
    if (/429|rate limit|Too Many/i.test(s)) return "リクエスト上限に達しました。少し待って再試行してください。";
    if (/timeout|timed out/i.test(s)) return "接続がタイムアウトしました。";
    if (/network|dns|ECONNREFUSED/i.test(s)) return "ネットワークエラー。接続を確認してください。";
    if (/model_not_found|does not exist/i.test(s)) return "指定されたモデルが見つかりません。";
    return s.length > 140 ? s.substring(0, 140) + "..." : s;
  }

  // ===== Step: checklist =====
  async function loadChecklist() {
    checklistLoading = true;
    try {
      const rows = await loadOnboardingChecks(purposes);
      checklistRows = rows.map(rowToChecklistAction);
    } finally {
      checklistLoading = false;
    }
  }

  function rowToChecklistAction(row: OnboardingCheckRow): ChecklistRow {
    return {
      ...row,
      action: () => jumpToSettings(row.panel),
    };
  }

  // ===== Step: finish capabilities =====
  async function loadCapabilities() {
    const caps: CapabilityRow[] = [];
    const [aiReadiness, sttReady] = await Promise.all([
      getAiReadinessLabel(),
      isSttReady(),
    ]);
    const { ready: aiReady, note: aiNote } = aiReadiness;
    const aiStatus: "ok" | "warn" = aiReady ? "ok" : "warn";

    caps.push({
      key: "agent",
      label: "AI Agent",
      group: "ai",
      status: aiStatus,
      note: aiNote,
      action: aiReady
        ? { label: "Agent を開く", run: () => { activeTab.set("agent"); close(); } }
        : { label: "AI 設定へ", run: () => jumpToSettings("ai") },
    });
    caps.push({
      key: "todo-ai",
      label: "TODO AI 分析",
      group: "ai",
      status: aiStatus,
      note: aiNote,
      action: { label: "TODO を開く", run: () => { activeTab.set("todo"); close(); } },
    });
    caps.push({
      key: "notif-ai",
      label: "通知 AI 要約",
      group: "ai",
      status: aiStatus,
      note: aiNote,
      action: { label: "ホームへ", run: () => { activeTab.set("home"); close(); } },
    });
    caps.push({
      key: "schedule-ai",
      label: "時間割 AI 分析",
      group: "ai",
      status: aiStatus,
      note: aiNote,
      action: { label: "時間割を開く", run: () => { activeTab.set("timetable"); close(); } },
    });

    // LIVE transcription only needs the local STT model; AI summaries are covered above.
    const liveReady = sttReady;
    caps.push({
      key: "live",
      label: "LIVE 文字起こし",
      group: "ai",
      status: liveReady ? "ok" : "warn",
      note: liveReady ? "利用可能" : "STT モデル未ダウンロード",
      action: liveReady
        ? { label: "LIVE を開く", run: () => { activeTab.set("live"); close(); } }
        : { label: "AI 設定へ", run: () => jumpToSettings("ai") },
    });

    // Base features (collected from checklistRows when possible)
    const baseRows = checklistRows.length
      ? checklistRows
      : (await loadOnboardingChecks(purposes)).map(rowToChecklistAction);
    for (const row of baseRows) {
      if (row.key === "stt") continue;
      caps.push({
        key: "base-" + row.key,
        label: row.label,
        group: "base",
        status: row.status === "loading" ? "warn" : row.status,
        note: row.detail,
        action: { label: row.actionLabel, run: () => row.action() },
      });
    }

    capabilities = caps;
  }

  function done() {
    completeOnboarding();
  }

  // Status pill helper
  function pillClass(s: OnboardingStatus | "ok" | "warn" | "off") {
    if (s === "ok") return "pill ok";
    if (s === "warn") return "pill warn";
    if (s === "off") return "pill off";
    return "pill";
  }
  function pillLabel(s: OnboardingStatus | "ok" | "warn" | "off") {
    if (s === "ok") return "✓";
    if (s === "warn") return "!";
    if (s === "off") return "—";
    return "…";
  }
</script>

{#if $onboardingVisible}
  <div class="overlay" role="dialog" aria-modal="true" aria-label="初期設定">
    <div class="sheet">
      <header class="head">
        <div class="head-text">
          <div class="head-eyebrow">初期設定</div>
          <h2 class="head-title">
            {#if step === "welcome"}ようこそ
            {:else if step === "purpose"}何に使いますか？
            {:else if step === "provider"}AI プロバイダを選ぶ
            {:else if step === "apikey"}API キーを設定
            {:else if step === "checklist"}基本機能のチェック
            {:else}準備が整いました
            {/if}
          </h2>
        </div>
        <button class="close-btn" onclick={skipOnboarding} aria-label="閉じる">×</button>
      </header>

      <div class="body">
        {#if step === "welcome"}
          <div class="hero">
            <img class="hero-logo" src={selahLogoUrl} alt="Selah" draggable="false" />
            <div class="hero-brand">Selah</div>
            <div class="hero-slogan">新月の下で、知性を繋ぐ。すべての関学生に。</div>
          </div>
          <p class="lead lead-center">時間割・お知らせ・課題・メール・LIVE 文字起こし・AI Agent を一つの画面で。<br />2 分ほどで初期設定を完了します。</p>

          <div class="sync-card">
            <div class="sync-head">データ同期</div>
            {#if $cacheStatus.items.length === 0}
              <div class="sync-empty">ログイン後の初回同期が完了するとここに表示されます。</div>
            {:else}
              <ul class="sync-list">
                {#each $cacheStatus.items as item}
                  <li class="sync-item">
                    <span class={pillClass(item.status === "done" ? "ok" : item.status === "error" ? "warn" : "off")}>
                      {item.status === "done" ? "✓" : item.status === "error" ? "!" : "…"}
                    </span>
                    <span class="sync-label">{item.label}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {:else if step === "purpose"}
          {@const opts = [
            { id: "summary" as const, title: "課題・通知を AI 要約", desc: "Luna / KWIC のお知らせを自動で要約" },
            { id: "agent" as const, title: "AI Agent に相談", desc: "時間割・通知・メール・資料を読める Agent" },
            { id: "live" as const, title: "LIVE 講義を文字起こし", desc: "ローカルで音声認識（モデル DL が必要）" },
            { id: "voice" as const, title: "音声で Agent を呼び出す", desc: "Fn キーで Agent に話しかけられる" },
          ]}
          <p class="lead">使いたい機能を選んでください（あとで変更できます）。選択に応じて必要な設定だけ案内します。</p>
          <div class="purpose-grid">
            {#each opts as o}
              <button
                class="purpose-card"
                class:selected={purposes.includes(o.id)}
                onclick={() => togglePurpose(o.id)}
              >
                <div class="purpose-check">{purposes.includes(o.id) ? "✓" : ""}</div>
                <div class="purpose-text">
                  <div class="purpose-title">{o.title}</div>
                  <div class="purpose-desc">{o.desc}</div>
                </div>
              </button>
            {/each}
          </div>
        {:else if step === "provider"}
          <p class="lead">AI 推論の提供元を選びます。どちらも API キーを入力するだけで使えます。</p>
          <div class="provider-grid">
            <button class="provider-card" class:selected={provider === "openai"} onclick={() => { provider = "openai"; }}>
              <div class="provider-name">OpenAI <span class="badge">推奨</span></div>
              <div class="provider-desc">GPT-5.4 系。応答が速く、ほとんどの機能で安定して動作します。</div>
            </button>
            <button class="provider-card" class:selected={provider === "gemini"} onclick={() => { provider = "gemini"; }}>
              <div class="provider-name">Google Gemini</div>
              <div class="provider-desc">無料枠あり。日本語の長文要約に強い傾向があります。</div>
            </button>
          </div>
          <button class="link-quiet" onclick={() => jumpToSettings("ai", "checklist")}>
            詳しい設定（ローカル AI 等）を開く →
          </button>
        {:else if step === "apikey"}
          <p class="lead">{PROVIDER_DEFAULTS[provider].hint}</p>
          <div class="key-row">
            <input
              type="password"
              bind:value={apiKey}
              placeholder={provider === "openai" ? "sk-..." : "AIza..."}
              autocomplete="off"
              spellcheck="false"
            />
            <button class="link-quiet" onclick={() => openExternal(PROVIDER_DEFAULTS[provider].keyUrl)}>
              キーを取得 →
            </button>
          </div>
          {#if testMsg}
            <div class="test-msg" class:ok={testOk === true} class:err={testOk === false}>
              {testMsg}
            </div>
          {/if}
          <p class="footnote">入力されたキーは端末内のセキュアストアに保存されます。Selah のサーバには送信されません。</p>
        {:else if step === "checklist"}
          <p class="lead">基本機能の状態を確認します。緑は設定済み、黄色は要対応、グレーは任意です。</p>
          {#if checklistLoading}
            <div class="loading">読み込み中...</div>
          {:else}
            <ul class="checklist">
              {#each checklistRows as row}
                <li class="checkrow">
                  <span class={pillClass(row.status)}>{pillLabel(row.status)}</span>
                  <div class="checkrow-body">
                    <div class="checkrow-label">{row.label}</div>
                    <div class="checkrow-detail">{row.detail}</div>
                  </div>
                  <button class="row-action" onclick={() => row.action()}>{row.actionLabel}</button>
                </li>
              {/each}
            </ul>
          {/if}
          <p class="footnote">各ボタンを押すと該当の設定画面に移動します。戻ってきたら設定からいつでも初期設定をやり直せます。</p>
        {:else if step === "finish"}
          <p class="lead">これで Selah を使い始められます。下の一覧から直接機能を開けます。</p>
          {#each ["ai", "base"] as group}
            {@const rows = capabilities.filter(c => c.group === group)}
            {#if rows.length}
              <div class="group-label">{group === "ai" ? "AI 機能" : "基本機能"}</div>
              <ul class="checklist">
                {#each rows as cap}
                  <li class="checkrow">
                    <span class={pillClass(cap.status)}>{pillLabel(cap.status)}</span>
                    <div class="checkrow-body">
                      <div class="checkrow-label">{cap.label}</div>
                      <div class="checkrow-detail">{cap.note}</div>
                    </div>
                    {#if cap.action}
                      <button class="row-action" onclick={() => cap.action!.run()}>{cap.action.label}</button>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          {/each}
        {/if}
      </div>

      <footer class="foot">
        {#if step !== "finish"}
          <button class="btn-ghost" onclick={skipOnboarding}>あとで</button>
        {/if}
        <div class="foot-right">
          {#if step === "welcome"}
            <button class="btn-primary" onclick={() => next("purpose")}>はじめる</button>
          {:else if step === "purpose"}
            <button class="btn-ghost" onclick={() => next("welcome")}>戻る</button>
            <button class="btn-primary" onclick={() => next("provider")} disabled={purposes.length === 0}>次へ</button>
          {:else if step === "provider"}
            <button class="btn-ghost" onclick={() => next("purpose")}>戻る</button>
            <button class="btn-primary" onclick={() => next("apikey")}>次へ</button>
          {:else if step === "apikey"}
            <button class="btn-ghost" onclick={() => next("provider")}>戻る</button>
            <button class="btn-ghost" onclick={() => { updateRecord({ purposes }); next("checklist"); }}>スキップ</button>
            {#if testDone}
              <button class="btn-primary" onclick={() => next("checklist")}>次へ</button>
            {:else}
              <button class="btn-primary" onclick={saveAndTest} disabled={testing}>
                {testing ? "テスト中..." : "保存してテスト"}
              </button>
            {/if}
          {:else if step === "checklist"}
            <button class="btn-primary" onclick={() => next("finish")}>次へ</button>
          {:else if step === "finish"}
            <button class="btn-primary" onclick={done}>完了</button>
          {/if}
        </div>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.36);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 9000;
    animation: fade-in 0.18s ease;
  }
  @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }

  .sheet {
    width: min(640px, calc(100vw - 48px));
    max-height: calc(100vh - 64px);
    background: var(--bg-primary);
    border: 0.5px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.25);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: pop-in 0.22s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes pop-in {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 18px 22px 12px;
    border-bottom: 0.5px solid var(--border);
  }
  .head-text { min-width: 0; }
  .head-eyebrow {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }
  .head-title {
    margin: 4px 0 0;
    font-size: 19px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }
  .close-btn {
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    font-size: 22px;
    line-height: 1;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    cursor: pointer;
  }
  .close-btn:hover { background: var(--bg-hover); color: var(--text-primary); }

  .body {
    padding: 16px 22px;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  .lead {
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--text-secondary);
    margin: 0 0 12px;
  }
  .lead-center { text-align: center; }

  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 8px 0 16px;
  }
  .hero-logo {
    width: 64px;
    height: 64px;
    object-fit: contain;
    margin-bottom: 4px;
    user-select: none;
    -webkit-user-drag: none;
  }
  .hero-brand {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--text-primary);
    line-height: 1.1;
  }
  .hero-slogan {
    font-size: 11.5px;
    color: var(--text-tertiary);
    letter-spacing: 0.02em;
  }

  .footnote {
    margin-top: 12px;
    font-size: 10.5px;
    color: var(--text-tertiary);
    line-height: 1.5;
  }

  .sync-card {
    margin-top: 12px;
    border: 0.5px solid var(--border);
    border-radius: 10px;
    padding: 12px 14px;
    background: var(--bg-secondary);
  }
  .sync-head { font-size: 11px; font-weight: 600; color: var(--text-primary); margin-bottom: 8px; }
  .sync-empty { font-size: 11.5px; color: var(--text-tertiary); }
  .sync-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .sync-item { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-primary); }
  .sync-label { color: var(--text-secondary); }

  .purpose-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: 4px;
  }
  .purpose-card {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 14px;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 10px;
    text-align: left;
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s;
    font-family: inherit;
    color: var(--text-primary);
  }
  .purpose-card:hover { border-color: var(--border-strong); }
  .purpose-card.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-secondary));
  }
  .purpose-check {
    width: 18px; height: 18px; border-radius: 5px;
    border: 1.5px solid var(--border-strong);
    display: flex; align-items: center; justify-content: center;
    font-size: 11px; color: #fff;
    flex-shrink: 0;
  }
  .purpose-card.selected .purpose-check {
    background: var(--accent);
    border-color: var(--accent);
  }
  .purpose-title { font-size: 12.5px; font-weight: 600; }
  .purpose-desc { font-size: 11px; color: var(--text-secondary); margin-top: 2px; line-height: 1.45; }

  .provider-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .provider-card {
    padding: 14px 16px;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 10px;
    text-align: left;
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s;
    font-family: inherit;
    color: var(--text-primary);
  }
  .provider-card:hover { border-color: var(--border-strong); }
  .provider-card.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-secondary));
  }
  .provider-name { font-size: 13.5px; font-weight: 700; display: flex; align-items: center; gap: 8px; }
  .badge {
    font-size: 9.5px; font-weight: 600;
    padding: 2px 6px; border-radius: 4px;
    background: var(--accent); color: #fff;
    letter-spacing: 0.03em;
  }
  .provider-desc { font-size: 11px; color: var(--text-secondary); margin-top: 6px; line-height: 1.5; }

  .link-quiet {
    display: inline-block;
    margin-top: 12px;
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 11px;
    cursor: pointer;
    padding: 4px 0;
    font-family: inherit;
  }
  .link-quiet:hover { color: var(--accent); }

  .key-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 6px;
  }
  .key-row input {
    flex: 1;
    padding: 8px 10px;
    font-size: 12.5px;
    font-family: monospace;
    letter-spacing: 0.04em;
    background: var(--bg-tertiary);
    border: 0.5px solid var(--border-strong);
    border-radius: 8px;
    color: var(--text-primary);
    outline: none;
  }
  .key-row input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .test-msg {
    margin-top: 10px;
    font-size: 11.5px;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--bg-secondary);
    color: var(--text-secondary);
  }
  .test-msg.ok { color: var(--green, #34c759); background: color-mix(in srgb, var(--green, #34c759) 8%, var(--bg-secondary)); }
  .test-msg.err { color: var(--red); background: color-mix(in srgb, var(--red) 8%, var(--bg-secondary)); }

  .checklist {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .checkrow {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    border-radius: 8px;
  }
  .checkrow-body { flex: 1; min-width: 0; }
  .checkrow-label { font-size: 12.5px; font-weight: 600; color: var(--text-primary); }
  .checkrow-detail { font-size: 11px; color: var(--text-secondary); margin-top: 2px; line-height: 1.45; }
  .row-action {
    flex-shrink: 0;
    padding: 5px 12px;
    font-size: 11px;
    font-weight: 600;
    border-radius: 6px;
    border: 0.5px solid var(--border-strong);
    background: var(--bg-hover);
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
  }
  .row-action:hover { background: var(--accent); color: #fff; border-color: var(--accent); }

  .group-label {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
    margin: 14px 2px 6px;
  }
  .group-label:first-child { margin-top: 0; }

  .pill {
    flex-shrink: 0;
    width: 20px; height: 20px;
    border-radius: 50%;
    display: flex; align-items: center; justify-content: center;
    font-size: 11px; font-weight: 700;
    background: var(--bg-tertiary);
    color: var(--text-tertiary);
  }
  .pill.ok { background: var(--green, #34c759); color: #fff; }
  .pill.warn { background: #f5a623; color: #fff; }
  .pill.off { background: var(--bg-tertiary); color: var(--text-tertiary); }

  .loading {
    padding: 16px;
    text-align: center;
    color: var(--text-tertiary);
    font-size: 12px;
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 22px;
    border-top: 0.5px solid var(--border);
    background: var(--bg-secondary);
  }
  .foot-right { display: flex; gap: 8px; }

  .btn-primary, .btn-ghost {
    padding: 7px 16px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 7px;
    cursor: pointer;
    font-family: inherit;
    border: 0.5px solid transparent;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .btn-primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .btn-primary:hover:not(:disabled) { opacity: 0.92; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    border-color: var(--border);
  }
  .btn-ghost:hover { background: var(--bg-hover); color: var(--text-primary); }
</style>
