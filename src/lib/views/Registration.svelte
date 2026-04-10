<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { fetchRegistration } from "../api";
  import { cachedFetch, onCacheUpdate } from "../stores";
  import type { RegistrationData } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import StudentBar from "../StudentBar.svelte";
  import DataTable from "../DataTable.svelte";

  let loading = $state(true);
  let error = $state("");
  let data = $state<RegistrationData | null>(null);

  const courseColumns = [
    { key: "day", label: "曜日", width: "50px", align: "center" as const },
    { key: "period", label: "時限", width: "60px", align: "center" as const },
    { key: "semester", label: "学期", width: "70px" },
    { key: "course_name", label: "授業名称" },
    { key: "instructor", label: "教員", width: "100px" },
    { key: "credits", label: "単位", align: "center" as const, width: "60px" },
    { key: "status", label: "状態", width: "60px", align: "center" as const },
  ];

  async function openRegistration() {
    try {
      await invoke("open_registration_window");
    } catch (e: any) {
      console.error("Failed to open registration window:", e);
    }
  }

  const unsubReg = onCacheUpdate<RegistrationData>("registration", (fresh) => { data = fresh; });
  onDestroy(() => unsubReg());

  onMount(async () => {
    try {
      data = await cachedFetch("registration", fetchRegistration);
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  });
</script>

<div class="view">
  <div class="title-row">
    <h2>履修登録</h2>
    <button class="open-btn" onclick={openRegistration}>
      履修登録画面を開く
    </button>
  </div>
  <ViewLoader {loading} {error}>
    {#if data}
      <StudentBar student={data.student} />

      {#if data.credit_summary.length > 0}
        <div class="section-label">単位数</div>
        <div class="credit-cards">
          {#each data.credit_summary as cs, i}
            <div class="credit-card" style="animation: fade-in 0.3s ease {i * 0.06}s both;">
              <div class="credit-label">{cs.semester}</div>
              <div class="credit-values">
                <span class="enrolled">{cs.enrolled}</span>
                <span class="sep">/</span>
                <span class="limit">{cs.limit}</span>
              </div>
            </div>
          {/each}
        </div>
      {/if}

      {#if data.courses.length > 0}
        <div class="section-label">登録科目</div>
        <DataTable data={data.courses} columns={courseColumns} />
      {:else}
        <div class="state-msg">登録科目はありません</div>
      {/if}
    {/if}
  </ViewLoader>
</div>

<style>
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 12px;
  }
  .title-row h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .open-btn {
    flex-shrink: 0;
    padding: 6px 16px;
    font-size: 12px;
    font-weight: 600;
    color: #fff;
    background: var(--accent, #002855);
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: opacity 0.15s;
  }
  .open-btn:hover { opacity: 0.85; }
  .open-btn:active { opacity: 0.7; }
  .section-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 20px 0 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .credit-cards {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    margin-bottom: 4px;
  }
  .credit-card {
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    border-radius: 12px;
    padding: 14px 22px;
    text-align: center;
    box-shadow: var(--shadow-sm);
    transition: transform 0.2s ease, box-shadow 0.2s ease;
  }
  .credit-card:hover {
    transform: translateY(-1px);
    box-shadow: var(--shadow-md);
  }
  .credit-label {
    font-size: 11px;
    color: var(--text-secondary);
    margin-bottom: 4px;
  }
  .credit-values {
    font-size: 20px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .enrolled { color: var(--accent); }
  .sep { color: var(--text-tertiary); margin: 0 2px; font-weight: 300; }
  .limit { color: var(--text-secondary); font-weight: 400; }
</style>
