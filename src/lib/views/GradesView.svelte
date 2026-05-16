  <script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { cachedBackendFetch, onCacheUpdate } from "../stores";
  import type { GradesData, CurriculumRow } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import StudentBar from "../StudentBar.svelte";

  let loading = $state(true);
  let error = $state("");
  let data = $state<GradesData | null>(null);

  const unsubGrades = onCacheUpdate<GradesData>("grades", (fresh) => { data = fresh; });
  onDestroy(() => unsubGrades());

  onMount(async () => {
    try {
      data = await cachedBackendFetch("grades");
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  });

  // Summary stats
  let totalRequired = $derived(
    data?.curriculum.find(r => r.level === 1 && r.required_credits)?.required_credits || ""
  );
  let totalEarned = $derived(
    data?.curriculum.find(r => r.level === 1 && r.required_credits)?.earned_credits || ""
  );
  let totalEnrolled = $derived(
    data?.curriculum.find(r => r.level === 1 && r.required_credits)?.enrolled_credits || ""
  );

  function progressPct(row: CurriculumRow): number {
    const req = parseFloat(row.required_credits);
    const acq = parseFloat(row.earned_credits);
    if (!req || isNaN(req) || isNaN(acq)) return 0;
    return Math.min(100, (acq / req) * 100);
  }

  function enrolledPct(row: CurriculumRow): number {
    const req = parseFloat(row.required_credits);
    const enr = parseFloat(row.enrolled_credits);
    if (!req || isNaN(req) || isNaN(enr)) return 0;
    return Math.min(100, (enr / req) * 100);
  }
</script>

<div class="view">
  <h2>成績照会</h2>
  <ViewLoader {loading} {error} empty={data?.curriculum.length === 0} emptyMessage="成績データがありません">
    {#if data}
      <StudentBar student={data.student} />

      {#if totalRequired}
        <div class="summary-card">
          <div class="summary-item">
            <span class="summary-label">卒業必要単位</span>
            <span class="summary-value">{totalRequired}</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">履修中</span>
            <span class="summary-value accent">{totalEnrolled}</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">修得済</span>
            <span class="summary-value green">{totalEarned}</span>
          </div>
          <div class="summary-bar-wrap">
            <div class="summary-bar">
              <div class="bar-enrolled" style="width:{enrolledPct(data.curriculum.find(r => r.level === 1 && r.required_credits)!)}%"></div>
              <div class="bar-earned" style="width:{progressPct(data.curriculum.find(r => r.level === 1 && r.required_credits)!)}%"></div>
            </div>
          </div>
        </div>
      {/if}

      <div class="credits-table">
        <div class="table-header">
          <span class="col-name">系列</span>
          <span class="col-num">必要</span>
          <span class="col-num">履修</span>
          <span class="col-num">修得</span>
          <span class="col-bar">進捗</span>
        </div>
        {#each data.curriculum as row, i}
          <div
            class="table-row"
            class:level1={row.level === 1}
            class:level2={row.level === 2}
            class:level3={row.level === 3}
            class:level4={row.level === 4}
            class:deficit={row.is_deficit && parseFloat(row.required_credits) > 0}
            style={`animation: fade-in 0.2s ease ${Math.min(i * 0.02, 0.5)}s both;`}
          >
            <span class="col-name" style="padding-left:{(row.level - 1) * 14}px;">
              {row.category}
            </span>
            <span class="col-num">{row.required_credits || "-"}</span>
            <span class="col-num">{row.enrolled_credits}</span>
            <span class="col-num" class:earned-ok={!row.is_deficit && parseFloat(row.earned_credits) > 0}>
              {row.earned_credits}
            </span>
            <span class="col-bar">
              {#if parseFloat(row.required_credits) > 0}
                <div class="mini-bar">
                  <div class="mini-enrolled" style="width:{enrolledPct(row)}%"></div>
                  <div class="mini-earned" style="width:{progressPct(row)}%"></div>
                </div>
              {/if}
            </span>
          </div>
        {/each}
      </div>
    {/if}
  </ViewLoader>
</div>

<style>
  .view h2 { margin-bottom: 12px; }

  .summary-card {
    display: flex; flex-wrap: wrap; align-items: center; gap: 16px;
    background: var(--bg-secondary); border-radius: 10px; padding: 12px 16px;
    margin-bottom: 14px; box-shadow: 0 0.5px 1px rgba(0,0,0,0.04);
  }
  .summary-item { display: flex; flex-direction: column; gap: 1px; }
  .summary-label { font-size: 10px; color: var(--text-secondary); }
  .summary-value { font-size: 18px; font-weight: 700; letter-spacing: -0.02em; }
  .summary-value.accent { color: var(--accent); }
  .summary-value.green { color: #34c759; }
  .summary-bar-wrap { flex: 1; min-width: 100px; }
  .summary-bar {
    height: 6px; border-radius: 3px; background: var(--border);
    position: relative; overflow: hidden;
  }
  .bar-enrolled {
    position: absolute; top: 0; left: 0; height: 100%;
    background: rgba(0,40,85,0.15); border-radius: 3px;
    transition: width 0.4s ease;
  }
  .bar-earned {
    position: absolute; top: 0; left: 0; height: 100%;
    background: #34c759; border-radius: 3px;
    transition: width 0.4s ease;
  }

  .credits-table {
    background: var(--bg-secondary); border-radius: 10px; overflow: hidden;
    box-shadow: 0 0.5px 1px rgba(0,0,0,0.04);
  }
  .table-header {
    display: flex; align-items: center; padding: 8px 12px;
    font-size: 10px; font-weight: 600; color: var(--text-secondary);
    border-bottom: 0.5px solid var(--border);
    text-transform: uppercase; letter-spacing: 0.02em;
  }
  .table-row {
    display: flex; align-items: center; padding: 7px 12px;
    font-size: 12px; border-top: 0.5px solid var(--border);
    transition: background 0.12s;
  }
  .table-row:first-of-type { border-top: none; }
  .table-row:hover { background: var(--bg-hover); }

  .table-row.level1 { font-weight: 600; background: rgba(0,40,85,0.04); }
  .table-row.level2 { font-weight: 500; }
  .table-row.level3 { color: var(--text-primary); }
  .table-row.level4 { color: var(--text-secondary); font-size: 11px; }

  .table-row.deficit .col-num { color: #ff3b30; }

  .col-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .col-num { width: 52px; text-align: right; flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .col-bar { width: 72px; flex-shrink: 0; padding-left: 8px; }

  .earned-ok { color: #34c759; font-weight: 500; }

  .mini-bar {
    height: 4px; border-radius: 2px; background: var(--border);
    position: relative; overflow: hidden;
  }
  .mini-enrolled {
    position: absolute; top: 0; left: 0; height: 100%;
    background: rgba(0,40,85,0.12); border-radius: 2px;
  }
  .mini-earned {
    position: absolute; top: 0; left: 0; height: 100%;
    background: #34c759; border-radius: 2px;
  }

  @media (prefers-color-scheme: dark) {
    .table-row.level1 { background: rgba(74,144,217,0.06); }
    .bar-enrolled, .mini-enrolled { background: rgba(74,144,217,0.2); }
  }
</style>
