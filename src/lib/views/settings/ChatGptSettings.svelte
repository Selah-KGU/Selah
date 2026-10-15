<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { getChatGptAccounts, type ChatGptAccount, type ChatGptModel } from "../../chatgptApi";
  import { isDemoActive, updateAiReadiness } from "../../api";
  import { externalLinkDelegate } from "../../externalLinkDelegate";

  let { model = $bindable(""), testConnection, testBusy = false }:
    { model: string; testConnection: () => Promise<void>; testBusy?: boolean } = $props();
  let accounts = $state<ChatGptAccount[]>([]);
  let models = $state<ChatGptModel[]>([]);
  let selected = $state("");
  let busy = $state(false);
  let signingIn = $state(false);
  let loading = $state(true);
  let error = $state("");
  let notice = $state("");
  let active = $derived(accounts.find((account) => account.active));
  let ready = $derived(!!active?.connected && !!active?.plan_enabled);
  let alive = true;
  let modelRequest = 0;

  async function refreshModels() {
    const request = ++modelRequest;
    models = [];
    if (!accounts.some((account) => account.active && account.connected && account.plan_enabled)) return;
    const next = await invoke<ChatGptModel[]>("chatgpt_list_models");
    if (!alive || request !== modelRequest) return;
    models = next;
    if (!next.some((item) => item.slug === model)) model = next[0]?.slug || "";
    if (!next.length) throw new Error("このアカウントで利用できるモデルがありません。ChatGPT の権限を確認してください。");
  }

  async function reload() {
    if (isDemoActive()) {
      notice = "デモでは接続できません。デスクトップ版で ChatGPT にログインしてください。";
      return;
    }
    const next = await getChatGptAccounts();
    if (!alive) return;
    accounts = next;
    selected = next.find((account) => account.active)?.client_id || next[0]?.client_id || "";
    await updateAiReadiness();
    await refreshModels();
  }

  async function run(action: () => Promise<void>) {
    busy = true;
    error = "";
    notice = "";
    try { await action(); }
    catch (e) { if (alive) error = String(e); }
    finally { if (alive) { busy = false; signingIn = false; loading = false; } }
  }

  function signIn(clientId: string | null) {
    void run(async () => {
      signingIn = true;
      try {
        await invoke("chatgpt_sign_in", { clientId });
      } finally {
        // An identity-only grant or failed first exchange still creates a saved
        // registration. Show it so the next attempt can reuse its issued ID.
        await reload();
      }
    });
  }

  async function selectAccount() {
    const account = accounts.find((item) => item.client_id === selected);
    if (!account) return;
    if (!account.connected || !account.plan_enabled) { signIn(account.client_id); return; }
    await run(async () => {
      await invoke("chatgpt_select_account", { clientId: selected });
      await reload();
    });
  }

  function disconnect() {
    void run(async () => {
      const revoked = await invoke<boolean>("chatgpt_disconnect", { clientId: selected });
      await reload();
      notice = revoked ? "ログアウトしました。" : "この端末からログアウトしました。サーバー側の解除を確認できないため、ChatGPT の設定で Selah の接続を解除してください。";
    });
  }

  onMount(() => { void run(reload); });
  onDestroy(() => {
    alive = false;
    ++modelRequest;
    if (signingIn) void invoke("chatgpt_cancel_login").catch(() => {});
  });
</script>

<div class="chatgpt-card" use:externalLinkDelegate>
  <div class="heading">ChatGPT プランを接続</div>
  <p>対象の ChatGPT Plus / Pro アカウントでログインし、Selah にプランの利用を許可してください。利用量は ChatGPT の上限に含まれ、許可したクレジットが使われる場合があります。</p>
  {#if accounts.length}
    <label for="chatgpt-account">アカウント</label>
    <select id="chatgpt-account" bind:value={selected} onchange={selectAccount} disabled={busy || testBusy}>
      {#each accounts as account}
        <option value={account.client_id}>{account.email || "ChatGPT アカウント"} · {account.client_id.slice(-8)}{account.active ? "（選択中）" : ""}{!account.connected ? "（未接続）" : ""}</option>
      {/each}
    </select>
  {/if}
  {#if active}
    <p class="account-status">{active.email || "ChatGPT アカウント"} — {ready ? "接続済み · プラン利用が有効" : "再ログインまたはプランの利用許可が必要です"}</p>
  {/if}
  <div class="actions">
    <button class="sign-in" onclick={() => signIn(selected || null)} disabled={busy || testBusy || isDemoActive()}>Continue with ChatGPT</button>
    {#if signingIn}
      <button onclick={() => invoke("chatgpt_cancel_login").catch((e) => { error = String(e); })}>キャンセル</button>
    {/if}
    {#if accounts.length && !signingIn}
      <button onclick={() => signIn(null)} disabled={busy || testBusy}>別のアカウントを追加</button>
      <button onclick={disconnect} disabled={busy || testBusy || !selected}>ログアウト</button>
    {/if}
    <a href="https://chatgpt.com/settings/usage" target="_blank" rel="noopener noreferrer">使用状況を管理</a>
  </div>
  {#if signingIn}<p role="status">ブラウザで認証とプラン利用の許可を完了してください。</p>{/if}
  {#if loading}<p role="status">接続状態を確認中...</p>{/if}
  {#if ready}
    <label for="chatgpt-model">モデル</label>
    <div class="model-row">
      <select id="chatgpt-model" bind:value={model} disabled={busy || testBusy || !models.length}>
        {#each models as item}<option value={item.slug}>{item.display_name}</option>{/each}
      </select>
      <button onclick={() => run(refreshModels)} disabled={busy || testBusy}>更新</button>
      <button onclick={testConnection} disabled={busy || testBusy || !models.some((item) => item.slug === model)}>{testBusy ? "テスト中..." : "接続テスト"}</button>
    </div>
    <p>接続テストで設定を保存し、選択したモデルの応答を確認できます。モデルの利用可否はアカウントとプランの権限に従います。</p>
  {/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .chatgpt-card { padding: 22px 24px; border-radius: 14px; border: 1px solid var(--border); background: var(--bg-card); margin-bottom: 18px; }
  .heading { font-size: 15px; font-weight: 650; }
  p { font-size: 12px; color: var(--text-secondary); line-height: 1.7; margin: 10px 0; }
  label { display: block; font-size: 12px; font-weight: 600; margin: 14px 0 8px; }
  select { min-width: 0; max-width: 100%; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-input, var(--bg-card)); color: var(--text-primary); }
  button { padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-card); color: var(--text-primary); cursor: pointer; font-size: 12px; }
  button:disabled { opacity: .5; cursor: default; }
  .sign-in { background: #111; color: #fff; border-color: #111; min-height: 40px; padding-inline: 16px; font-weight: 600; }
  .actions, .model-row { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; margin: 12px 0; }
  .model-row select { flex: 1; }
  a { font-size: 12px; color: var(--accent); }
  .error { color: var(--danger, #c23c3c); }
  .account-status { color: var(--text-primary); }
</style>
