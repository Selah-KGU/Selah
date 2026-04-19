<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let permState = $state<"loading" | "ok" | "ng">("loading");
  let permLabel = $state("確認中...");

  let notifImportant = $state("true");
  let notifFaculty = $state("true");
  let notifClass = $state("true");
  let notifOther = $state("true");
  let notifMail = $state("true");

  let saveBusy = $state(false);

  async function checkPerm() {
    try {
      const granted = await invoke<boolean>("plugin:notification|is_permission_granted");
      permState = granted ? "ok" : "ng";
      permLabel = granted ? "許可済" : "未許可";
    } catch {
      permState = "ng";
      permLabel = "エラー";
    }
  }

  async function loadConfig() {
    try {
      const cfg = await invoke<{ notify_important?: boolean; notify_faculty?: boolean; notify_class?: boolean; notify_other?: boolean; notify_mail?: boolean }>("get_notification_config");
      notifImportant = cfg.notify_important ? "true" : "false";
      notifFaculty = cfg.notify_faculty ? "true" : "false";
      notifClass = cfg.notify_class ? "true" : "false";
      notifOther = cfg.notify_other ? "true" : "false";
      notifMail = cfg.notify_mail !== false ? "true" : "false";
    } catch (e) {
      console.error("Failed to load notification config:", e);
    }
  }

  export async function save() {
    saveBusy = true;
    try {
      await invoke("save_notification_config", {
        config: {
          notify_important: notifImportant === "true",
          notify_faculty: notifFaculty === "true",
          notify_class: notifClass === "true",
          notify_other: notifOther === "true",
          notify_mail: notifMail === "true",
        },
      });
    } catch (e) {
      throw e;
    } finally {
      saveBusy = false;
    }
  }

  onMount(() => {
    void checkPerm();
    void loadConfig();
  });
</script>

<div class="hero-card">
  <div class="hero-icon" style="background:linear-gradient(135deg,rgba(255,150,0,0.15),rgba(255,59,48,0.12));">
    <svg viewBox="0 0 20 20" fill="none" stroke="#cc7a00" stroke-width="1.3">
      <path d="M10 2.5a5 5 0 015 5v3l1.5 2H3.5l1.5-2v-3a5 5 0 015-5z" stroke-linejoin="round"/>
      <path d="M8 14.5a2 2 0 004 0" stroke-linecap="round"/>
    </svg>
  </div>
  <div class="hero-text">
    <h2 class="panel-title">通知</h2>
    <p class="panel-desc">通知の受信設定を管理します。カテゴリごとに通知のオン・オフを切り替えできます。</p>
  </div>
</div>

<div class="card-label">通知設定</div>
<div class="card">
  <div class="row">
    <span class="row-label">通知権限</span>
    <div class="row-input">
      <div class="session-indicator">
        {#if permState === "loading"}<span class="spinner-sm"></span>
        {:else if permState === "ok"}<span class="session-dot ok"></span>
        {:else}<span class="session-dot ng"></span>{/if}
        {permLabel}
      </div>
    </div>
  </div>
</div>

<div class="card-label">受信カテゴリ</div>
<div class="card">
  <div class="row">
    <span class="row-label">呼出し・重要なお知らせ</span>
    <div class="row-input">
      <select bind:value={notifImportant}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">学部・研究科からのお知らせ</span>
    <div class="row-input">
      <select bind:value={notifFaculty}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">授業のお知らせ</span>
    <div class="row-input">
      <select bind:value={notifClass}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">その他</span>
    <div class="row-input">
      <select bind:value={notifOther}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">メール</span>
    <div class="row-input">
      <select bind:value={notifMail}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
</div>

