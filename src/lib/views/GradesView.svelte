<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { fetchGrades } from "../api";
  import { cachedFetch, onCacheUpdate } from "../stores";
  import type { GradesData } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import StudentBar from "../StudentBar.svelte";
  import DataTable from "../DataTable.svelte";

  let loading = $state(true);
  let error = $state("");
  let data = $state<GradesData | null>(null);

  const columns = [
    { key: "category", label: "系列" },
    { key: "required_credits", label: "必要単位", align: "center" as const, width: "110px" },
    { key: "enrolled_credits", label: "履修単位", align: "center" as const, width: "110px" },
    { key: "earned_credits", label: "修得単位", align: "center" as const, width: "110px" },
  ];

  const unsubGrades = onCacheUpdate<GradesData>("grades", (fresh) => { data = fresh; });
  onDestroy(() => unsubGrades());

  onMount(async () => {
    try {
      data = await cachedFetch("grades", fetchGrades);
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  });
</script>

<div class="view">
  <h2>成績照会</h2>
  <ViewLoader {loading} {error} empty={data?.curriculum.length === 0} emptyMessage="成績データがありません">
    {#if data}
      <StudentBar student={data.student} />
      <DataTable data={data.curriculum} {columns} />
    {/if}
  </ViewLoader>
</div>

<style>
  .view h2 {
    margin-bottom: 12px;
  }
</style>
