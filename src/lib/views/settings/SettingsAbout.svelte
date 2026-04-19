<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { devModeActive } from "../../stores";
  import { get } from "svelte/store";
  import logoUrl from "../../../assets/logo.png";

  let version = $state("...");
  let deleteConfirmOpen = $state(false);
  let deleteErr = $state("");

  let tapCount = 0;
  let tapTimer: ReturnType<typeof setTimeout> | null = null;
  let shakeKey = $state(0);

  async function loadVersion() {
    try {
      const info = await invoke<{ app_version?: string }>("debug_info");
      if (info.app_version) version = info.app_version;
    } catch (e) {
      console.error("version fetch failed:", e);
    }
  }

  function handleVersionTap() {
    if (get(devModeActive)) return;
    tapCount++;
    if (tapTimer) clearTimeout(tapTimer);
    const remaining = 7 - tapCount;
    if (remaining > 0 && remaining <= 3) {
      shakeKey++;
    }
    if (tapCount >= 7) {
      devModeActive.set(true);
      tapCount = 0;
    }
    tapTimer = setTimeout(() => { tapCount = 0; }, 1500);
  }

  async function openUrl(url: string) {
    try { await invoke("open_external_url", { url }); } catch (e) { console.error(e); }
  }

  async function logoutAndReturn() {
    try {
      await invoke("logout");
      location.reload();
    } catch (e) {
      console.error("logout failed:", e);
    }
  }

  function startDelete() {
    deleteErr = "";
    deleteConfirmOpen = true;
  }

  async function confirmDelete() {
    try {
      await invoke("delete_all_local_data");
      try { localStorage.clear(); } catch { /* noop */ }
      location.reload();
    } catch (e) {
      deleteErr = "データの削除に失敗しました: " + String(e);
    }
  }

  onMount(() => { void loadVersion(); });
</script>

<div class="card">
  <div class="about-hero">
    <img
      class="about-icon"
      src={logoUrl}
      alt="Selah"
      onclick={handleVersionTap}
      draggable="false"
    />
    <div class="about-name">Selah</div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    {#key shakeKey}
      <div
        class={`about-version ${shakeKey > 0 ? `shaking-${shakeKey % 2}` : ""}`}
        onclick={handleVersionTap}
      >バージョン {version}</div>
    {/key}
    <div class="about-tagline">新月の下で、知性を繋ぐ。すべての関学生に。</div>
    <div class="about-sep"></div>
  </div>
  <div>
    <div class="about-row"><span class="al">開発者</span><span class="ar">mirai-mamori</span></div>
    <div class="about-row"><span class="al">対応大学</span><span class="ar">関西学院大学</span></div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="about-row clickable" onclick={() => openUrl("https://github.com/mirai-mamori/Selah")}>
      <span class="al">ソースコード</span>
      <span class="ar" style="color:var(--accent)">GitHub<span style="color:var(--text-tertiary);font-weight:400;margin-left:4px">›</span></span>
    </div>
    <div class="about-row"><span class="al">ライセンス</span><span class="ar">MIT License</span></div>
  </div>
  <div class="about-copy">
    © 2026 mirai-mamori. All rights reserved.<br />
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="link" onclick={() => openUrl("https://selah.jp/terms.html")}>利用規約</span>
    <span style="margin:0 4px;color:var(--text-tertiary)">|</span>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="link" onclick={() => openUrl("https://selah.jp/privacy.html")}>プライバシーポリシー</span>
  </div>
</div>

<div class="card" style="margin-top:10px;padding:0;">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="about-row clickable" style="justify-content:center;" onclick={logoutAndReturn}>
    <span style="color:var(--red);font-weight:500;">ログアウトしてログイン画面へ戻る</span>
  </div>
</div>

{#if !deleteConfirmOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="card" style="margin-top:10px;padding:0;cursor:pointer;" onclick={startDelete}>
    <div class="about-row" style="justify-content:center;pointer-events:none;">
      <span style="color:var(--red);font-weight:500;">すべてのローカルデータを削除</span>
    </div>
    <div class="delete-warn">
      データベース、設定、Cookie、キーチェーンの秘密情報など、アプリが保存したすべてのデータを完全に削除します。この操作は取り消せません。
    </div>
  </div>
{:else}
  <div class="card delete-confirm" style="margin-top:10px;padding:12px;">
    <div style="font-size:12px;font-weight:600;color:var(--red);margin-bottom:6px;">本当に削除しますか？</div>
    <div style="font-size:11px;color:var(--text-secondary);margin-bottom:10px;line-height:1.4;">
      すべてのローカルデータ（データベース、設定ファイル、Cookie、キーチェーンの秘密情報など）が完全に削除されます。この操作は取り消せません。
    </div>
    {#if deleteErr}
      <div style="font-size:11px;color:var(--red);margin-bottom:8px;">{deleteErr}</div>
    {/if}
    <div style="display:flex;gap:8px;justify-content:flex-end;">
      <button class="btn-test" onclick={() => { deleteConfirmOpen = false; }}>キャンセル</button>
      <button class="btn-test danger-solid" onclick={confirmDelete}>削除する</button>
    </div>
  </div>
{/if}

<style>
  :global(.settings-main .about-hero) {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: 20px 0 0;
  }
  :global(.settings-main .about-icon) {
    width: 72px;
    height: 72px;
    margin-bottom: 10px;
    cursor: default;
    -webkit-user-select: none;
    user-select: none;
  }
  :global(.settings-main .about-name) {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  :global(.settings-main .about-version) {
    font-size: 11px;
    color: var(--text-secondary);
    margin-top: 1px;
    cursor: default;
    -webkit-user-select: none;
    user-select: none;
  }
  :global(.settings-main .about-version.shaking-0) {
    animation: shake-0 0.3s ease;
  }
  :global(.settings-main .about-version.shaking-1) {
    animation: shake-1 0.3s ease;
  }
  :global(.settings-main .about-tagline) {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 6px;
    font-style: italic;
  }
  :global(.settings-main .about-sep) {
    width: 36px;
    height: 0.5px;
    background: var(--border-strong);
    margin: 14px auto;
  }
  :global(.settings-main .about-row) {
    display: flex;
    justify-content: space-between;
    padding: 7px 14px;
    border-top: 0.5px solid var(--border);
    font-size: 12px;
  }
  :global(.settings-main .about-row:first-child) {
    border-top: none;
  }
  :global(.settings-main .about-row .al) {
    color: var(--text-secondary);
  }
  :global(.settings-main .about-row .ar) {
    color: var(--text-primary);
    font-weight: 500;
  }
  :global(.settings-main .about-row.clickable) {
    cursor: pointer;
  }
  :global(.settings-main .about-copy) {
    font-size: 10px;
    color: var(--text-tertiary);
    text-align: center;
    padding: 12px 14px 10px;
  }
  :global(.settings-main .about-copy .link) {
    cursor: pointer;
    color: var(--accent);
  }
  :global(.settings-main .delete-warn) {
    padding: 0 12px 8px;
    font-size: 10px;
    color: var(--text-secondary);
    line-height: 1.35;
    pointer-events: none;
  }
  :global(.settings-main .delete-confirm) {
    border: 1px solid var(--red);
  }
  :global(.settings-main .btn-test.danger-solid) {
    background: var(--red);
    color: #fff;
    border-color: var(--red);
  }

  @keyframes shake-0 {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(-3px); }
    75% { transform: translateX(3px); }
  }
  @keyframes shake-1 {
    0%, 100% { transform: translateX(0); }
    25% { transform: translateX(3px); }
    75% { transform: translateX(-3px); }
  }
</style>
