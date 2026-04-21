<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let permState = $state<"loading" | "ok" | "ng">("loading");
  let permLabel = $state("確認中...");

  let notifImportant = $state("true");
  let notifFaculty = $state("true");
  let notifClass = $state("true");
  let notifClassGeneral = $state("true");
  let notifClassAnnouncement = $state("true");
  let notifClassAssignment = $state("true");
  let notifClassExam = $state("true");
  let notifClassDiscussion = $state("true");
  let notifClassSurvey = $state("true");
  let notifClassAttendance = $state("true");
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
      const cfg = await invoke<{
        notify_important?: boolean;
        notify_faculty?: boolean;
        notify_class?: boolean;
        notify_class_general?: boolean;
        notify_class_announcement?: boolean;
        notify_class_assignment?: boolean;
        notify_class_exam?: boolean;
        notify_class_discussion?: boolean;
        notify_class_survey?: boolean;
        notify_class_attendance?: boolean;
        notify_other?: boolean;
        notify_mail?: boolean;
      }>("get_notification_config");
      notifImportant = cfg.notify_important !== false ? "true" : "false";
      notifFaculty = cfg.notify_faculty !== false ? "true" : "false";
      notifClass = cfg.notify_class !== false ? "true" : "false";
      notifClassGeneral = cfg.notify_class_general !== false ? "true" : "false";
      notifClassAnnouncement = cfg.notify_class_announcement !== false ? "true" : "false";
      notifClassAssignment = cfg.notify_class_assignment !== false ? "true" : "false";
      notifClassExam = cfg.notify_class_exam !== false ? "true" : "false";
      notifClassDiscussion = cfg.notify_class_discussion !== false ? "true" : "false";
      notifClassSurvey = cfg.notify_class_survey !== false ? "true" : "false";
      notifClassAttendance = cfg.notify_class_attendance !== false ? "true" : "false";
      notifOther = cfg.notify_other !== false ? "true" : "false";
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
          notify_class_general: notifClassGeneral === "true",
          notify_class_announcement: notifClassAnnouncement === "true",
          notify_class_assignment: notifClassAssignment === "true",
          notify_class_exam: notifClassExam === "true",
          notify_class_discussion: notifClassDiscussion === "true",
          notify_class_survey: notifClassSurvey === "true",
          notify_class_attendance: notifClassAttendance === "true",
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
    <p class="panel-desc">通知の受信設定を管理します。カテゴリごとの切り替えに加えて、授業通知は種類別に細かく制御できます。</p>
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

<div class="card-label">授業通知の詳細</div>
<div class="card" style:opacity={notifClass === "true" ? 1 : 0.6}>
  <div class="hint" style="padding:6px 14px 8px;">
    「授業のお知らせ」が有効な場合のみ適用されます。掲示板の返信やコメント通知は「Luna 掲示板・コメント」で切り替えます。
  </div>
  <div class="row">
    <span class="row-label">一般の授業通知</span>
    <div class="row-input">
      <select bind:value={notifClassGeneral} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
      <div class="hint">KG-Course、KWIC の授業通知、Luna の未分類通知に適用されます。</div>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna お知らせ・資料</span>
    <div class="row-input">
      <select bind:value={notifClassAnnouncement} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna 課題・レポート</span>
    <div class="row-input">
      <select bind:value={notifClassAssignment} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna テスト・小テスト</span>
    <div class="row-input">
      <select bind:value={notifClassExam} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna 掲示板・コメント</span>
    <div class="row-input">
      <select bind:value={notifClassDiscussion} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna アンケート</span>
    <div class="row-input">
      <select bind:value={notifClassSurvey} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
  <div class="row">
    <span class="row-label">Luna 出席</span>
    <div class="row-input">
      <select bind:value={notifClassAttendance} disabled={notifClass !== "true"}>
        <option value="true">有効</option>
        <option value="false">無効</option>
      </select>
    </div>
  </div>
</div>
