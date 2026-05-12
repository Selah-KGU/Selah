<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { isDemoActive } from "../../api";

  const DEMO_DOWNLOAD_CONFIG_KEY = "selah-demo-download-config";

  function readDemoDownloadConfig() {
    try {
      const raw = localStorage.getItem(DEMO_DOWNLOAD_CONFIG_KEY);
      return raw ? JSON.parse(raw) : {};
    } catch {
      return {};
    }
  }

  function writeDemoDownloadConfig(config: any) {
    try { localStorage.setItem(DEMO_DOWNLOAD_CONFIG_KEY, JSON.stringify(config)); } catch { /* ignore */ }
  }

  let downloadDir = $state("");
  let classifyByCourse = $state("true");
  let saveBusy = $state(false);
  let statusMsg = $state("");
  let statusColor = $state("");

  async function loadConfig() {
    if (isDemoActive()) {
      const cfg = readDemoDownloadConfig();
      downloadDir = cfg.download_dir || "";
      classifyByCourse = cfg.classify_by_course !== false ? "true" : "false";
      return;
    }
    try {
      const cfg = await invoke<{ download_dir?: string; classify_by_course?: boolean }>("get_download_config");
      downloadDir = cfg.download_dir || "";
      classifyByCourse = cfg.classify_by_course ? "true" : "false";
    } catch (e) {
      console.error("Failed to load download config:", e);
    }
  }

  async function browseDir() {
    if (isDemoActive()) {
      downloadDir = "/Users/demo/Documents/Selah";
      statusColor = "var(--green)";
      statusMsg = "デモ用の保存先を選択しました";
      setTimeout(() => { statusMsg = ""; }, 4000);
      return;
    }
    try {
      const dir = await invoke<string>("select_download_dir");
      downloadDir = dir;
    } catch (e) {
      if (e !== "cancelled") {
        statusColor = "var(--red)";
        statusMsg = "フォルダ選択失敗: " + String(e);
        setTimeout(() => { statusMsg = ""; }, 4000);
      }
    }
  }

  async function resetToDefault() {
    downloadDir = "";
    classifyByCourse = "true";
    try {
      if (isDemoActive()) {
        writeDemoDownloadConfig({ download_dir: "", classify_by_course: true });
      } else {
        await invoke("save_download_config", { config: { download_dir: "", classify_by_course: true } });
      }
      statusColor = "var(--green)";
      statusMsg = "デフォルトに戻しました";
    } catch (e) {
      statusColor = "var(--red)";
      statusMsg = "リセット失敗: " + String(e);
    }
    setTimeout(() => { statusMsg = ""; }, 4000);
  }

  export async function save() {
    saveBusy = true;
    try {
      const config = {
        download_dir: downloadDir,
        classify_by_course: classifyByCourse === "true",
      };
      if (isDemoActive()) writeDemoDownloadConfig(config);
      else await invoke("save_download_config", { config });
    } catch (e) {
      throw e;
    } finally {
      saveBusy = false;
    }
  }

  onMount(() => { void loadConfig(); });
</script>

<div class="hero-card">
  <div class="hero-icon" style="background:linear-gradient(135deg,rgba(0,122,255,0.15),rgba(52,199,89,0.15));">
    <svg viewBox="0 0 20 20" fill="none" stroke="#0055b3" stroke-width="1.3">
      <path d="M10 3v10" stroke-linecap="round"/>
      <polyline points="6 9 10 13 14 9" stroke-linecap="round" stroke-linejoin="round"/>
      <path d="M3 14v2a1 1 0 001 1h12a1 1 0 001-1v-2" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">ダウンロード</h2>
    <p class="panel-desc">ファイルの保存先と自動分類の設定を管理します。コース名でフォルダを自動作成し、教材を整理できます。</p>
  </div>
</div>

<div class="card-label">保存先</div>
<div class="card">
  <div class="row">
    <span class="row-label">フォルダ</span>
    <div class="row-input">
      <div class="key-row">
        <input
          type="text"
          bind:value={downloadDir}
          placeholder="(ドキュメント/Selah)"
          spellcheck="false"
          style="font-size:11px;"
          readonly
        />
        <button class="btn-test" onclick={browseDir}>選択</button>
      </div>
      <div class="hint">空欄の場合は <code>ドキュメント/Selah</code> に自動保存されます。</div>
    </div>
  </div>
</div>

<div class="card-label">自動分類</div>
<div class="card">
  <div class="row">
    <span class="row-label">コース別分類</span>
    <div class="row-input">
      <select bind:value={classifyByCourse}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
      <div class="hint">有効にすると、ダウンロードされた教材をコース名のフォルダに自動分類します。コース不明のファイルは <code>その他/</code> にまとめられます。</div>
    </div>
  </div>
</div>

<div class="card-label">フォルダ構成例</div>
<div class="card preview">
  <div>ドキュメント/Selah/</div>
  <div style="padding-left:16px;">基礎数学 I/</div>
  <div style="padding-left:32px;">lecture01.pdf</div>
  <div style="padding-left:32px;">exercise05.pdf</div>
  <div style="padding-left:16px;">英語 IIA/</div>
  <div style="padding-left:32px;">handout.docx</div>
  <div style="padding-left:16px;">その他/</div>
  <div style="padding-left:32px;">attachment.pdf</div>
</div>

<div class="action-bar">
  <button class="btn-test" onclick={resetToDefault}>デフォルトに戻す</button>
  {#if statusMsg}
    <span class="hint" style="color:{statusColor};margin-left:4px;">{statusMsg}</span>
  {/if}
</div>

<style>
  .key-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .action-bar {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 8px;
  }
  .preview {
    padding: 10px 14px;
    font-size: 11px;
    color: var(--text-secondary);
    line-height: 1.8;
    font-family: monospace;
  }
  code {
    font-size: 10px;
    background: var(--bg-hover);
    padding: 1px 4px;
    border-radius: 3px;
  }
</style>
