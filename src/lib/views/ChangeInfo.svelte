  <script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { authState, cachedBackendFetch, onCacheUpdate, splitByFaculty } from "../stores";
  import type { CancellationsData, MakeupData, RoomChangesData } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import DataTable from "../DataTable.svelte";
  import Icon from "../Icon.svelte";

  let activeTab = $state<"cancellations" | "makeup" | "rooms">("cancellations");
  let loading = $state(true);
  let error = $state("");

  let cancellations = $state<CancellationsData | null>(null);
  let makeup = $state<MakeupData | null>(null);
  let rooms = $state<RoomChangesData | null>(null);

  const cancelColumns = [
    { key: "date", label: "休講日付", width: "100px" },
    { key: "period", label: "休講時限", align: "center" as const, width: "70px" },
    { key: "campus", label: "キャンパス", width: "90px" },
    { key: "department", label: "授業管理部署", width: "100px" },
    { key: "course_code", label: "授業コード", width: "95px" },
    { key: "year", label: "開講年度", width: "75px" },
    { key: "course_name", label: "授業名称" },
    { key: "instructor", label: "教員氏名", width: "100px" },
    { key: "room", label: "教室名称", width: "100px" },
    { key: "comment", label: "コメント" },
  ];

  const makeupColumns = [
    { key: "date", label: "補講日付", width: "100px" },
    { key: "period", label: "時限", align: "center" as const, width: "70px" },
    { key: "campus", label: "キャンパス", width: "90px" },
    { key: "department", label: "授業管理部署", width: "100px" },
    { key: "course_code", label: "授業コード", width: "95px" },
    { key: "year", label: "開講年度", width: "75px", align: "center" as const },
    { key: "course_name", label: "授業名称" },
    { key: "instructor", label: "教員氏名", width: "100px" },
    { key: "room", label: "教室名称", width: "100px" },
    { key: "comment", label: "コメント" },
  ];

  const roomColumns = [
    { key: "date", label: "変更日付", width: "110px" },
    { key: "department", label: "授業管理部署", width: "100px" },
    { key: "course_code", label: "授業コード", width: "95px" },
    { key: "year", label: "開講年度", width: "75px", align: "center" as const },
    { key: "course_name", label: "授業名称" },
    { key: "room", label: "教室名称(変更前／変更後)" },
    { key: "instructor", label: "教員氏名(変更前／変更後)", width: "120px" },
    { key: "schedule", label: "曜時(変更前／変更後)", width: "120px" },
    { key: "comment", label: "コメント" },
  ];

  let cancelSplit = $derived(splitByFaculty(cancellations?.entries, $authState.faculty));
  let makeupSplit = $derived(splitByFaculty(makeup?.entries, $authState.faculty));
  let roomSplit = $derived(splitByFaculty(rooms?.entries, $authState.faculty));

  // SWR: update UI when background polling brings fresh data
  const unsubCancel = onCacheUpdate<CancellationsData>("cancellations", (fresh) => { cancellations = fresh; });
  const unsubMakeup = onCacheUpdate<MakeupData>("makeup", (fresh) => { makeup = fresh; });
  const unsubRooms = onCacheUpdate<RoomChangesData>("rooms", (fresh) => { rooms = fresh; });
  onDestroy(() => { unsubCancel(); unsubMakeup(); unsubRooms(); });

  onMount(async () => {
    try {
      const [c, m, r] = await Promise.all([
        cachedBackendFetch<CancellationsData>("cancellations"),
        cachedBackendFetch<MakeupData>("makeup"),
        cachedBackendFetch<RoomChangesData>("rooms"),
      ]);
      cancellations = c;
      makeup = m;
      rooms = r;
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  });

  function currentData() {
    if (activeTab === "cancellations") return cancellations?.entries;
    if (activeTab === "makeup") return makeup?.entries;
    return rooms?.entries;
  }

  let cancelCount = $derived(cancellations?.entries?.length ?? 0);
  let makeupCount = $derived(makeup?.entries?.length ?? 0);
  let roomCount = $derived(rooms?.entries?.length ?? 0);
</script>

<div class="view">
  <h2>変更情報</h2>

  <div class="segmented-control" role="tablist">
    <button class="segment" class:active={activeTab === "cancellations"} role="tab" aria-selected={activeTab === "cancellations"} onclick={() => activeTab = "cancellations"}>
      休講
      {#if cancelCount > 0}<span class="count-badge">{cancelCount}</span>{/if}
    </button>
    <button class="segment" class:active={activeTab === "makeup"} role="tab" aria-selected={activeTab === "makeup"} onclick={() => activeTab = "makeup"}>
      補講
      {#if makeupCount > 0}<span class="count-badge">{makeupCount}</span>{/if}
    </button>
    <button class="segment" class:active={activeTab === "rooms"} role="tab" aria-selected={activeTab === "rooms"} onclick={() => activeTab = "rooms"}>
      教室変更
      {#if roomCount > 0}<span class="count-badge">{roomCount}</span>{/if}
    </button>
  </div>

  <ViewLoader {loading} {error} empty={!loading && (currentData()?.length ?? 0) === 0} emptyMessage="情報はありません">
    {#if activeTab === "cancellations" && cancellations}
      {@const { related, others } = cancelSplit}
      {#if related.length > 0}
        <div class="section-group related">
          <h3 class="section-label"><Icon name="pin" size={14} />{$authState.faculty}の休講情報</h3>
          <DataTable data={related} columns={cancelColumns} />
        </div>
      {/if}
      {#if others.length > 0}
        <div class="section-group">
          {#if related.length > 0}
            <h3 class="section-label other-label">その他の休講情報</h3>
          {/if}
          <DataTable data={others} columns={cancelColumns} />
        </div>
      {/if}

    {:else if activeTab === "makeup" && makeup}
      {@const { related, others } = makeupSplit}
      {#if related.length > 0}
        <div class="section-group related">
          <h3 class="section-label"><Icon name="pin" size={14} />{$authState.faculty}の補講情報</h3>
          <DataTable data={related} columns={makeupColumns} />
        </div>
      {/if}
      {#if others.length > 0}
        <div class="section-group">
          {#if related.length > 0}
            <h3 class="section-label other-label">その他の補講情報</h3>
          {/if}
          <DataTable data={others} columns={makeupColumns} />
        </div>
      {/if}

    {:else if activeTab === "rooms" && rooms}
      {@const { related, others } = roomSplit}
      {#if related.length > 0}
        <div class="section-group related">
          <h3 class="section-label"><Icon name="pin" size={14} />{$authState.faculty}の教室変更</h3>
          <DataTable data={related} columns={roomColumns} />
        </div>
      {/if}
      {#if others.length > 0}
        <div class="section-group">
          {#if related.length > 0}
            <h3 class="section-label other-label">その他の教室変更</h3>
          {/if}
          <DataTable data={others} columns={roomColumns} />
        </div>
      {/if}
    {/if}
  </ViewLoader>
</div>

<style>
  .view h2 {
    margin-bottom: 12px;
  }
  .segmented-control {
    display: flex;
    background: var(--bg-tertiary);
    border-radius: 8px;
    padding: 2px;
    margin-bottom: 12px;
    gap: 2px;
  }
  .segment {
    flex: 1;
    padding: 6px 10px;
    border: none;
    background: none;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
  }
  .segment:hover { color: var(--text-primary); }
  .segment.active {
    background: var(--bg-card);
    color: var(--text-primary);
    font-weight: 600;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08);
  }
  .count-badge {
    font-size: 10px;
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 9px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    text-align: center;
  }
  .section-group {
    margin-bottom: 16px;
  }
  .section-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 8px 4px;
  }
  .other-label {
    color: var(--text-secondary);
    margin-top: 8px;
  }
  .section-label :global(.icon) {
    margin-right: 4px;
    vertical-align: -1px;
    color: var(--accent, #007aff);
  }
  .related :global(.table-wrap) {
    box-shadow: 0 0 0 1.5px var(--accent, #007aff), var(--shadow-md);
  }
</style>
