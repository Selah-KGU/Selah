<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getAiConfig, isDemoActive } from "../api";
  import { activeTab } from "../stores";
  import { reopenOnboarding } from "./onboardingState";
  import { listen } from "@tauri-apps/api/event";

  type Status = "ok" | "warn";
  interface Chip {
    key: string;
    label: string;
    status: Status;
    title: string;
    run: () => void;
  }

  let chips = $state<Chip[]>([]);
  let loading = $state(true);
  let unlisten: (() => void) | null = null;

  async function compute() {
    let aiReady = false;
    try {
      const cfg = await getAiConfig();
      aiReady = cfg.ai_enabled !== false && !!cfg.api_key?.trim();
    } catch { /* ignore */ }

    let sttReady = false;
    try {
      const models: any[] = isDemoActive() ? [] : await invoke("list_stt_models");
      const cfg: any = isDemoActive() ? null : await invoke("get_stt_config");
      const sel = models.find((m) => m.id === cfg?.selected_model);
      sttReady = !!sel?.downloaded;
    } catch { /* ignore */ }

    const aiTitle = aiReady ? "利用可能" : "API キー未設定（クリックで初期設定）";
    const aiStatus: Status = aiReady ? "ok" : "warn";

    const liveStatus: Status = aiReady && sttReady ? "ok" : "warn";
    const liveTitle = !aiReady && !sttReady ? "AI と STT モデルが未設定"
      : !aiReady ? "AI 未設定"
      : !sttReady ? "STT モデル未ダウンロード"
      : "利用可能";

    chips = [
      { key: "agent", label: "Agent", status: aiStatus, title: aiTitle,
        run: () => aiReady ? activeTab.set("agent") : reopenOnboarding() },
      { key: "todo", label: "TODO 分析", status: aiStatus, title: aiTitle,
        run: () => aiReady ? activeTab.set("todo") : reopenOnboarding() },
      { key: "notif", label: "通知要約", status: aiStatus, title: aiTitle,
        run: () => aiReady ? activeTab.set("home") : reopenOnboarding() },
      { key: "schedule", label: "時間割分析", status: aiStatus, title: aiTitle,
        run: () => aiReady ? activeTab.set("timetable") : reopenOnboarding() },
      { key: "live", label: "LIVE 文字起こし", status: liveStatus, title: liveTitle,
        run: () => liveStatus === "ok" ? activeTab.set("live") : reopenOnboarding() },
    ];
    loading = false;
  }

  onMount(async () => {
    void compute();
    try {
      unlisten = await listen("ai-config-changed", () => { void compute(); });
    } catch { /* ignore */ }
  });
  onDestroy(() => { unlisten?.(); });
</script>

<div class="card-label">AI 機能の状態</div>
<div class="cap-row">
  {#if loading}
    <span class="cap-loading">確認中...</span>
  {:else}
    {#each chips as c}
      <button class="chip chip-{c.status}" title={c.title} onclick={c.run}>
        <span class="dot"></span>
        <span class="label">{c.label}</span>
      </button>
    {/each}
  {/if}
</div>

<style>
  .cap-row {
    display: flex;
    flex-wrap: nowrap;
    gap: 6px;
    margin-bottom: 10px;
    overflow-x: auto;
  }
  .cap-loading { font-size: 11px; color: var(--text-tertiary); padding: 6px 0; }

  .chip {
    flex: 1 1 0;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: 7px;
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    color: var(--text-primary);
    font-size: 11.5px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .chip .label {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .chip:hover { background: var(--bg-hover); border-color: var(--border-strong); }

  .dot {
    flex-shrink: 0;
    width: 7px; height: 7px;
    border-radius: 50%;
  }
  .chip-ok .dot { background: var(--green, #34c759); }
  .chip-warn .dot { background: #f5a623; }
  .chip-warn { color: var(--text-secondary); }
</style>
