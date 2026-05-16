  <script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { isDemoActive } from "../api";
  import { cachedBackendFetch, onCacheUpdate } from "../stores";
  import type { RegistrationData } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import StudentBar from "../StudentBar.svelte";

  let loading = $state(true);
  let error = $state("");
  let data = $state<RegistrationData | null>(null);

  const days = ["月", "火", "水", "木", "金", "土"];

  const statusColor: Record<string, string> = {
    "申請": "var(--accent, #002855)",
    "履修": "#34c759",
    "選択中": "#ff9500",
    "履修済": "#8e8e93",
  };

  function coursesForDay(dayLabel: string) {
    if (!data) return [];
    return data.courses.filter(c => c.day === dayLabel);
  }

  let activeDays = $derived(
    days.filter(d => data?.courses.some(c => c.day === d))
  );

  function scrollToDay(day: string) {
    const el = document.getElementById(`day-${day}`);
    el?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function creditPct(enrolled: string, limit: string): number {
    const e = parseFloat(enrolled);
    const l = parseFloat(limit);
    if (!l || isNaN(l) || isNaN(e)) return 0;
    return Math.min(100, (e / l) * 100);
  }

  async function openRegistration() {
    if (isDemoActive()) return;
    try {
      await invoke("open_registration_window");
    } catch (e: any) {
      console.error("Failed to open registration window:", e);
    }
  }

  const unsubReg = onCacheUpdate<RegistrationData>("registration", (fresh) => {
    data = fresh;
    if (fresh) { error = ""; loading = false; }
  });
  onDestroy(() => unsubReg());

  onMount(async () => {
    try {
      data = await cachedBackendFetch("registration");
    } catch (e: any) {
      error = e?.message || String(e);
      // Auto-retry once after 3s (covers kgc_gate contention / transient errors)
      setTimeout(async () => {
        try {
          data = await cachedBackendFetch("registration");
          error = "";
        } catch { /* keep existing error */ }
      }, 3000);
    } finally {
      loading = false;
    }
  });
</script>

<div class="view">
  <div class="title-row">
    <div class="title-left">
      <h2>履修登録</h2>
      {#if data?.year_semester}
        <span class="year-badge">{data.year_semester}</span>
      {/if}
    </div>
    <button class="open-btn" onclick={openRegistration}>
      履修登録画面を開く
    </button>
  </div>

  <ViewLoader {loading} {error} empty={!data && !loading} emptyMessage="履修情報がありません">
    {#if data}
      <StudentBar student={data.student} />

      {#if data.last_applied}
        <div class="meta-row">
          <span class="meta-label">前回申請</span>
          <span class="meta-value">{data.last_applied}</span>
          {#if data.language_options.length > 0}
            <span class="meta-sep"></span>
            {#each data.language_options as opt}
              <span class="lang-tag">{opt.name}: {opt.value}</span>
            {/each}
          {/if}
        </div>
      {/if}

      {#if data.credit_summary.length > 0}
        <div class="credit-cards">
          {#each data.credit_summary as cs, i}
            <div class="credit-card" style="animation: fade-in 0.3s ease {i * 0.06}s both;">
              <div class="credit-top">
                <span class="credit-label">{cs.semester}</span>
                <span class="credit-nums">
                  <span class="enrolled">{cs.enrolled || "0"}</span>
                  {#if cs.limit !== "－" && cs.semester !== "年間"}
                    <span class="sep">/</span>
                    <span class="limit">{cs.limit}</span>
                  {/if}
                </span>
              </div>
              {#if cs.limit !== "－" && parseFloat(cs.limit) > 0 && cs.semester !== "年間"}
                <div class="credit-bar-bg">
                  <div
                    class="credit-bar-fill"
                    class:near-limit={creditPct(cs.enrolled, cs.limit) > 85}
                    style="width:{creditPct(cs.enrolled, cs.limit)}%"
                  ></div>
                </div>
              {/if}
            </div>
          {/each}
          <div class="credit-card total-card" style="animation: fade-in 0.3s ease 0.18s both;">
            <div class="credit-top">
              <span class="credit-label">合計</span>
              <span class="credit-nums">
                <span class="enrolled">{data.courses.length}</span>
                <span class="sep">科目</span>
              </span>
            </div>
          </div>
        </div>
      {/if}

      {#if data.courses.length > 0}
        <div class="schedule-layout">
          <div class="day-rail">
            {#each activeDays as day}
              <button
                class="day-pill"
                onclick={() => scrollToDay(day)}
              >
                {day}
                <span class="pill-count">{coursesForDay(day).length}</span>
              </button>
            {/each}
          </div>

          <div class="day-list">
            {#each activeDays as day, di}
              <div class="day-section" id="day-{day}">
                <div class="day-header" style="animation: fade-in 0.2s ease {di * 0.06}s both;">
                  {day}曜日
                  <span class="day-header-count">{coursesForDay(day).length}科目</span>
                </div>
                {#each coursesForDay(day) as course, ci}
                  <div
                    class="course-row"
                    style="animation: fade-in 0.2s ease {di * 0.06 + ci * 0.03 + 0.05}s both;"
                  >
                    <div class="course-period">{course.period.replace("時限", "")}</div>
                    <div class="course-main">
                      <div class="course-name">{course.course_name}</div>
                      <div class="course-meta">
                        <span>{course.instructor}</span>
                        {#if course.room}
                          <span class="meta-dot"></span>
                          <span>{course.room}</span>
                        {/if}
                        {#if course.campus}
                          <span class="meta-dot"></span>
                          <span class="campus-text">{course.campus}</span>
                        {/if}
                      </div>
                    </div>
                    <div class="course-right">
                      <span class="course-credits">{course.credits}</span>
                      <span class="course-status" style="color:{statusColor[course.status] || '#8e8e93'}">{course.status}</span>
                    </div>
                  </div>
                {/each}
              </div>
            {/each}
          </div>
        </div>
      {:else}
        <div class="state-msg">登録科目はありません</div>
      {/if}
    {/if}
  </ViewLoader>
</div>

<style>
  .title-row {
    display: flex; align-items: center; justify-content: space-between;
    gap: 12px; margin-bottom: 12px;
  }
  .title-left { display: flex; align-items: center; gap: 10px; }
  .title-left h2 { margin: 0; font-size: 20px; font-weight: 600; letter-spacing: -0.01em; }
  .year-badge {
    font-size: 11px; font-weight: 500; color: var(--accent, #002855);
    background: rgba(0,40,85,0.08); border-radius: 6px; padding: 3px 10px;
  }
  .open-btn {
    flex-shrink: 0; padding: 6px 16px; font-size: 12px; font-weight: 600;
    color: #fff; background: var(--accent, #002855); border: none;
    border-radius: 8px; cursor: pointer; transition: opacity 0.15s;
  }
  .open-btn:hover { opacity: 0.85; }
  .open-btn:active { opacity: 0.7; }

  .meta-row {
    display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
    font-size: 11px; color: var(--text-secondary); margin-bottom: 12px;
  }
  .meta-label { font-weight: 500; }
  .meta-value { font-variant-numeric: tabular-nums; }
  .meta-sep { width: 1px; height: 12px; background: var(--border); }
  .lang-tag {
    background: var(--bg-secondary); border-radius: 4px; padding: 2px 6px;
    font-size: 10px;
  }

  .credit-cards {
    display: flex; gap: 8px; flex-wrap: wrap; margin-bottom: 14px;
  }
  .credit-card {
    flex: 1; min-width: 120px;
    background: var(--bg-secondary); border-radius: 10px;
    padding: 10px 14px;
    box-shadow: 0 0.5px 1px rgba(0,0,0,0.04);
  }
  .credit-top {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
  }
  .credit-label { font-size: 11px; color: var(--text-secondary); }
  .credit-nums { font-size: 16px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .enrolled { color: var(--accent); }
  .sep { color: var(--text-tertiary); margin: 0 1px; font-weight: 300; font-size: 13px; }
  .limit { color: var(--text-secondary); font-weight: 400; font-size: 13px; }
  .total-card { background: rgba(0,40,85,0.04); }

  .credit-bar-bg {
    height: 4px; background: var(--border); border-radius: 2px;
    margin-top: 8px; overflow: hidden;
  }
  .credit-bar-fill {
    height: 100%; border-radius: 2px;
    background: var(--accent, #002855); transition: width 0.4s ease;
  }
  .credit-bar-fill.near-limit { background: var(--green, #34c759); }

  .schedule-layout {
    display: flex; gap: 10px;
  }
  .day-rail {
    display: flex; flex-direction: column; gap: 4px;
    flex-shrink: 0; width: 42px;
    position: sticky; top: 0; align-self: flex-start;
  }
  .day-pill {
    display: flex; flex-direction: column; align-items: center;
    gap: 1px; padding: 6px 0; border: none; background: var(--bg-secondary);
    border-radius: 8px; cursor: pointer; transition: all 0.15s;
    font-size: 12px; font-weight: 600; color: var(--text-secondary);
  }
  .day-pill:hover { background: var(--accent, #002855); color: #fff; }
  .pill-count {
    font-size: 9px; font-weight: 500; line-height: 1; opacity: 0.5;
  }
  .day-pill:hover .pill-count { opacity: 0.85; }

  .day-list { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 14px; }

  .day-section { display: flex; flex-direction: column; gap: 6px; }
  .day-header {
    font-size: 13px; font-weight: 700; color: var(--text-primary);
    display: flex; align-items: center; gap: 6px;
    padding: 0 2px;
  }
  .day-header-count {
    font-size: 11px; font-weight: 400; color: var(--text-tertiary, #999);
  }

  .course-row {
    display: flex; align-items: center; gap: 10px;
    background: var(--bg-secondary); border-radius: 10px;
    padding: 10px 12px;
    transition: background 0.12s;
  }
  .course-row:hover { background: var(--bg-hover, rgba(0,0,0,0.02)); }

  .course-period {
    font-size: 13px; font-weight: 700; color: var(--text-secondary);
    width: 20px; text-align: center; flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .course-main { flex: 1; min-width: 0; }
  .course-name {
    font-size: 13px; font-weight: 600; line-height: 1.3;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .course-meta {
    display: flex; align-items: center; gap: 4px; margin-top: 3px;
    font-size: 11px; color: var(--text-secondary);
    white-space: nowrap; overflow: hidden;
  }
  .meta-dot {
    width: 2px; height: 2px; border-radius: 1px;
    background: var(--text-tertiary, #999); flex-shrink: 0;
  }
  .campus-text { color: var(--text-tertiary, #999); }

  .course-right {
    display: flex; flex-direction: column; align-items: flex-end;
    gap: 2px; flex-shrink: 0;
  }
  .course-credits { font-size: 11px; color: var(--text-secondary); font-weight: 500; }
  .course-status { font-size: 10px; font-weight: 600; }

  .state-msg {
    text-align: center; color: var(--text-secondary);
    font-size: 13px; padding: 40px 0;
  }

  @media (prefers-color-scheme: dark) {
    .year-badge { background: rgba(74,144,217,0.12); }
    .total-card { background: rgba(74,144,217,0.06); }
    .day-pill { background: rgba(255,255,255,0.05); }
    .day-pill:hover { background: var(--accent, #4a90d9); color: #fff; }
  }
</style>
