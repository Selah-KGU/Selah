<script lang="ts">
  import { untrack } from "svelte";
  import { searchSyllabus, fetchSyllabusFavorites, toggleSyllabusBookmark, openSyllabusDetail } from "../api";
  import { authState, syllabusSearchState, cachedFetch, invalidateCache } from "../stores";
  import type { SyllabusSearchParams, SyllabusSearchResult, SyllabusEntry } from "../stores";
  import DataTable from "../DataTable.svelte";

  let loading = $state(false);
  let error = $state("");

  // Restore cached state
  let cachedState = $state($syllabusSearchState);
  let result = $state<SyllabusSearchResult | null>(cachedState.result);
  let searched = $state(cachedState.searched);
  let formCollapsed = $state(cachedState.collapsed);

  // Tabs: "results" | "favorites"
  let activeResultTab = $state<"results" | "favorites">("results");
  let favorites = $state<SyllabusSearchResult | null>(cachedState.favorites);
  let favLoading = $state(false);
  let favError = $state("");
  let togglingSet = $state(new Set<string>());
  let showFavInTimetable = $state(localStorage.getItem("selah-fav-in-timetable") === "1");

  // Form state - restore from cache
  const currentYear = new Date().getFullYear().toString();
  let yearFrom = $state(cachedState.params.year_from || currentYear);
  let yearTo = $state(cachedState.params.year_to || currentYear);
  let term = $state(cachedState.params.term);
  let campus = $state(cachedState.params.campus);
  let department = $state(cachedState.params.department);
  let classCode = $state(cachedState.params.class_code);
  let dayPeriod = $state(cachedState.params.day_period);
  let keyword = $state(cachedState.params.keyword);
  let instructor = $state(cachedState.params.instructor);
  let language = $state(cachedState.params.language);

  const terms = [
    { value: "", label: "未選択" },
    { value: "01", label: "通年" },
    { value: "02", label: "春学期" },
    { value: "03", label: "秋学期" },
    { value: "04", label: "春学期前半" },
    { value: "05", label: "春学期後半" },
    { value: "06", label: "秋学期前半" },
    { value: "07", label: "秋学期後半" },
    { value: "20", label: "通年集中" },
    { value: "21", label: "春学期集中" },
    { value: "22", label: "秋学期集中" },
  ];

  const campuses = [
    { value: "", label: "未選択" },
    { value: "1", label: "西宮上ケ原" },
    { value: "2", label: "神戸三田" },
    { value: "3", label: "大阪梅田" },
    { value: "5", label: "西宮聖和" },
    { value: "6", label: "オンライン" },
    { value: "7", label: "東京丸の内" },
    { value: "8", label: "西宮北口" },
  ];

  const departments = [
    { value: "", label: "未選択" },
    { value: "21", label: "神学部" },
    { value: "22", label: "文学部" },
    { value: "23", label: "社会学部" },
    { value: "24", label: "法学部" },
    { value: "25", label: "経済学部" },
    { value: "26", label: "商学部" },
    { value: "28", label: "理工学部" },
    { value: "29", label: "総合政策学部" },
    { value: "31", label: "人間福祉学部" },
    { value: "32", label: "教育学部" },
    { value: "34", label: "国際学部" },
    { value: "36", label: "理学部" },
    { value: "37", label: "工学部" },
    { value: "38", label: "生命環境学部" },
    { value: "39", label: "建築学部" },
    { value: "42", label: "共通教育センター" },
    { value: "45", label: "言語教育研究センター" },
    { value: "49", label: "国際教育・協力センター" },
  ];

  // Auto-fill department based on student's faculty
  $effect(() => {
    const faculty = $authState.faculty;
    if (faculty) {
      untrack(() => {
        if (!department) {
          const match = departments.find(d =>
            d.value && (d.label.includes(faculty) || faculty.includes(d.label))
          );
          if (match) department = match.value;
        }
      });
    }
  });

  const days = ["月", "火", "水", "木", "金", "土"];
  const periods = ["１", "２", "３", "４", "５", "６", "７"];
  const dayPeriodCodes = ["A", "B", "C", "D", "E", "F"];

  // Build day-period options
  const dayPeriodOptions: { value: string; label: string }[] = [
    { value: "", label: "未選択" },
    ...days.flatMap((day, di) =>
      periods.map((p, pi) => ({
        value: `${dayPeriodCodes[di]}${pi + 1}`,
        label: `${day}曜${p}時限`,
      }))
    ),
    { value: "Z9", label: "集中・その他" },
  ];

  const languages = [
    { value: "", label: "未選択" },
    { value: "001", label: "日本語" },
    { value: "002", label: "英語" },
    { value: "003", label: "フランス語" },
    { value: "004", label: "ドイツ語" },
    { value: "005", label: "中国語" },
    { value: "006", label: "朝鮮語" },
    { value: "007", label: "スペイン語" },
  ];

  const columns = [
    { key: "bookmark", label: "☆", class: "col-bookmark", align: "center" as const },
    { key: "academic_year", label: "年度", class: "col-year" },
    { key: "department", label: "学部", class: "col-dept" },
    { key: "class_code", label: "授業コード", class: "col-code" },
    { key: "course_title", label: "授業名称", class: "col-title" },
    { key: "instructor", label: "教員", class: "col-instructor" },
    { key: "term", label: "履修期", class: "col-term" },
    { key: "day_period", label: "曜時", class: "col-dayperiod" },
    { key: "campus", label: "キャンパス", class: "col-campus" },
  ];

  const favColumns = [
    { key: "remove", label: "☆", class: "col-bookmark", align: "center" as const },
    { key: "academic_year", label: "年度", class: "col-year" },
    { key: "department", label: "学部", class: "col-dept" },
    { key: "class_code", label: "授業コード", class: "col-code" },
    { key: "course_title", label: "授業名称", class: "col-title" },
    { key: "instructor", label: "教員", class: "col-instructor" },
    { key: "term", label: "履修期", class: "col-term" },
    { key: "day_period", label: "曜時", class: "col-dayperiod" },
    { key: "campus", label: "キャンパス", class: "col-campus" },
  ];

  async function doSearch() {
    // Validate: at least one of term/campus/department/dayPeriod required
    if (!term && !campus && !department && !dayPeriod) {
      error = "履修期・キャンパス・学部・曜時のいずれか１つを指定してください。";
      return;
    }

    loading = true;
    error = "";
    result = null;
    searched = true;

    try {
      const params: SyllabusSearchParams = {
        year_from: yearFrom,
        year_to: yearTo,
        term,
        campus,
        department,
        class_code: classCode,
        day_period: dayPeriod,
        keyword,
        instructor,
        language,
      };
      result = await searchSyllabus(params);
      // Collapse form and cache state after successful search
      if (result && result.entries.length > 0) {
        formCollapsed = true;
      }
      saveState();
      // Auto-load favorites in background
      loadFavorites();
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  function saveState() {
    syllabusSearchState.set({
      params: {
        year_from: yearFrom,
        year_to: yearTo,
        term,
        campus,
        department,
        class_code: classCode,
        day_period: dayPeriod,
        keyword,
        instructor,
        language,
      },
      result,
      favorites,
      searched,
      collapsed: formCollapsed,
    });
  }

  function resetForm() {
    yearFrom = currentYear;
    yearTo = currentYear;
    term = "";
    campus = "";
    department = "";
    classCode = "";
    dayPeriod = "";
    keyword = "";
    instructor = "";
    language = "";
    error = "";
    result = null;
    searched = false;
    formCollapsed = false;
    activeResultTab = "results";
    favorites = null;
    saveState();
  }

  function toggleForm() {
    formCollapsed = !formCollapsed;
    saveState();
  }

  // Build search summary for collapsed display
  let searchSummary = $derived.by(() => {
    const parts: string[] = [];
    if (yearFrom || yearTo) parts.push(`${yearFrom}〜${yearTo}`);
    const termLabel = terms.find(t => t.value === term)?.label;
    if (termLabel && term) parts.push(termLabel);
    const campusLabel = campuses.find(c => c.value === campus)?.label;
    if (campusLabel && campus) parts.push(campusLabel);
    const deptLabel = departments.find(d => d.value === department)?.label;
    if (deptLabel && department) parts.push(deptLabel);
    const dpLabel = dayPeriodOptions.find(d => d.value === dayPeriod)?.label;
    if (dpLabel && dayPeriod) parts.push(dpLabel);
    if (keyword) parts.push(`「${keyword}」`);
    if (instructor) parts.push(instructor);
    if (classCode) parts.push(classCode);
    return parts.join(" / ");
  });

  async function loadFavorites() {
    favLoading = true;
    favError = "";
    try {
      favorites = await cachedFetch("favorites", fetchSyllabusFavorites);
      saveState();
    } catch (e: any) {
      favError = e?.message || String(e);
    } finally {
      favLoading = false;
    }
  }

  async function handleToggleBookmark(entry: SyllabusEntry & { _original?: SyllabusEntry }, e: MouseEvent) {
    e.stopPropagation();
    const original = entry._original ?? entry;
    const classCode = original.class_code;
    if (!classCode || togglingSet.has(classCode)) return;
    togglingSet = new Set([...togglingSet, classCode]);
    try {
      await toggleSyllabusBookmark(classCode);
      // Toggle local state
      if (result) {
        result = {
          ...result,
          entries: result.entries.map(e =>
            e.class_code === classCode ? { ...e, bookmarked: !e.bookmarked } : e
          ),
        };
        saveState();
      }
      // Refresh favorites in background
      invalidateCache("favorites");
      loadFavorites();
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      const next = new Set(togglingSet);
      next.delete(classCode);
      togglingSet = next;
    }
  }

  async function handleRemoveFavorite(entry: SyllabusEntry & { _original?: SyllabusEntry }, e: MouseEvent) {
    e.stopPropagation();
    const original = entry._original ?? entry;
    const classCode = original.class_code;
    if (!classCode || togglingSet.has(classCode)) return;
    togglingSet = new Set([...togglingSet, classCode]);
    try {
      await toggleSyllabusBookmark(classCode);
      invalidateCache("favorites");
      // Remove from favorites list locally
      if (favorites) {
        favorites = {
          ...favorites,
          entries: favorites.entries.filter(e => e.class_code !== classCode),
        };
        saveState();
      }
      // Also update search results bookmark state
      if (result) {
        result = {
          ...result,
          entries: result.entries.map(e =>
            e.class_code === classCode ? { ...e, bookmarked: false } : e
          ),
        };
        saveState();
      }
    } catch (e: any) {
      error = e?.message || String(e);
    } finally {
      const next = new Set(togglingSet);
      next.delete(classCode);
      togglingSet = next;
    }
  }

  function handleRowClick(entry: SyllabusEntry & { _original?: SyllabusEntry }) {
    const original = entry._original ?? entry;
    openSyllabusDetail(original.class_code, original.course_title).catch(e => console.error("Failed to open syllabus:", e));
  }

  // Strip bilingual suffixes for compact display
  function shortText(text: string): string {
    return text.replace(/／.+$/, "").replace(/\/.+$/, "");
  }

  // Transform entries for display, preserving original for actions
  let displayEntries = $derived(
    result?.entries.map((e: SyllabusEntry) => ({
      ...e,
      _original: e,
      academic_year: shortText(e.academic_year),
      department: shortText(e.department),
      course_title: shortText(e.course_title),
      term: shortText(e.term),
      day_period: shortText(e.day_period),
      campus: shortText(e.campus),
    })) ?? []
  );

  let displayFavorites = $derived(
    favorites?.entries.map((e: SyllabusEntry) => ({
      ...e,
      _original: e,
      academic_year: shortText(e.academic_year),
      department: shortText(e.department),
      course_title: shortText(e.course_title),
      term: shortText(e.term),
      day_period: shortText(e.day_period),
      campus: shortText(e.campus),
    })) ?? []
  );
</script>

<div class="view">
  <h2>シラバス検索</h2>

  <div class="search-form" class:collapsed={formCollapsed}>
    <button class="form-header" onclick={toggleForm}>
      <span class="form-header-title">
        <svg class="chevron" class:rotated={!formCollapsed} width="12" height="12" viewBox="0 0 12 12">
          <path d="M4.5 2.5L8 6L4.5 9.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>
        </svg>
        検索条件
      </span>
      {#if formCollapsed && searchSummary}
        <span class="search-summary">{searchSummary}</span>
      {/if}
    </button>
    <div class="form-body" class:hidden={formCollapsed}>
    <div class="form-grid">
      <label class="field">
        <span class="field-label">開講年度</span>
        <div class="year-range">
          <input type="text" bind:value={yearFrom} maxlength="4" class="input-year" />
          <span class="range-sep">〜</span>
          <input type="text" bind:value={yearTo} maxlength="4" class="input-year" />
        </div>
      </label>

      <label class="field">
        <span class="field-label">履修期</span>
        <select bind:value={term}>
          {#each terms as t}
            <option value={t.value}>{t.label}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        <span class="field-label">キャンパス</span>
        <select bind:value={campus}>
          {#each campuses as c}
            <option value={c.value}>{c.label}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        <span class="field-label">学部・研究科</span>
        <select bind:value={department}>
          {#each departments as d}
            <option value={d.value}>{d.label}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        <span class="field-label">曜時</span>
        <select bind:value={dayPeriod}>
          {#each dayPeriodOptions as dp}
            <option value={dp.value}>{dp.label}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        <span class="field-label">授業コード</span>
        <input type="text" bind:value={classCode} maxlength="10" placeholder="例: 28550600" />
      </label>

      <label class="field span-2">
        <span class="field-label">フリーワード検索</span>
        <input type="text" bind:value={keyword} maxlength="100" placeholder="授業名称等" />
      </label>

      <label class="field span-2">
        <span class="field-label">教員名</span>
        <input type="text" bind:value={instructor} maxlength="50" placeholder="漢字名称" />
      </label>

      <label class="field">
        <span class="field-label">教授言語</span>
        <select bind:value={language}>
          {#each languages as l}
            <option value={l.value}>{l.label}</option>
          {/each}
        </select>
      </label>
    </div>

    <div class="form-actions">
      <button class="btn-primary" onclick={doSearch} disabled={loading}>
        {#if loading}
          <span class="spinner"></span>
          検索
        {:else}
          検索
        {/if}
      </button>
      <button class="btn-secondary" onclick={resetForm} disabled={loading}>リセット</button>
    </div>
    </div>
  </div>

  {#if error}
    <div class="error-message">{error}</div>
  {/if}

  {#if searched && !loading}
    {#if result && result.entries.length > 0}
      <div class="segmented-control" role="tablist">
        <button
          class="segment"
          class:active={activeResultTab === "results"}
          role="tab"
          aria-selected={activeResultTab === "results"}
          onclick={() => activeResultTab = "results"}
        >
          検索結果 ({result.total_count || result.entries.length})
        </button>
        <button
          class="segment"
          class:active={activeResultTab === "favorites"}
          role="tab"
          aria-selected={activeResultTab === "favorites"}
          onclick={() => { activeResultTab = "favorites"; if (!favorites && !favLoading) loadFavorites(); }}
        >
          お気に入り{favorites ? ` (${favorites.entries.length})` : ""}
        </button>
      </div>

      {#if activeResultTab === "results"}
        <div class="result-info">
          {result.total_count || result.entries.length}件の結果{#if result.total_pages > 1}（{result.current_page}/{result.total_pages}ページ）{/if}
        </div>
        {#snippet cellSnippet({ row, col, value }: { row: any; col: any; value: any })}
          {#if col.key === "bookmark"}
            <button
              class="bookmark-btn"
              class:bookmarked={row._original?.bookmarked ?? row.bookmarked}
              class:toggling={togglingSet.has(row._original?.register_index ?? row.register_index)}
              onclick={(e: MouseEvent) => handleToggleBookmark(row, e)}
              title={row._original?.bookmarked ?? row.bookmarked ? "お気に入り解除" : "お気に入りに追加"}
            >
              {#if togglingSet.has(row._original?.register_index ?? row.register_index)}
                <span class="bookmark-spinner"></span>
              {:else}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                  <path d="M4 2h8a1 1 0 0 1 1 1v11.5l-5-3-5 3V3a1 1 0 0 1 1-1z"
                    fill={(row._original?.bookmarked ?? row.bookmarked) ? "var(--accent)" : "none"}
                    stroke={(row._original?.bookmarked ?? row.bookmarked) ? "var(--accent)" : "var(--text-tertiary)"}
                    stroke-width="1.2"
                  />
                </svg>
              {/if}
            </button>
          {:else}
            {value ?? ""}
          {/if}
        {/snippet}
        <DataTable data={displayEntries} {columns} {cellSnippet} onrowclick={(row: any) => handleRowClick(row)} />
      {:else}
        {#if favLoading}
          <div class="loading-message"><span class="spinner"></span> お気に入りを読み込み中...</div>
        {:else if favError}
          <div class="error-message">{favError}</div>
        {:else if favorites && favorites.entries.length > 0}
          <div class="result-info fav-header">
            <span>{favorites.entries.length}件のお気に入り</span>
            <label class="fav-timetable-toggle" title="時間割にお気に入り科目を表示">
              <input type="checkbox" bind:checked={showFavInTimetable} onchange={() => { localStorage.setItem("selah-fav-in-timetable", showFavInTimetable ? "1" : "0"); window.dispatchEvent(new CustomEvent("selah-fav-toggle", { detail: showFavInTimetable })); }} />
              <span class="toggle-label">時間割に表示</span>
            </label>
            <button class="btn-refresh" onclick={() => loadFavorites()} title="更新">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path d="M13.65 2.35A7.96 7.96 0 0 0 8 0C3.58 0 0 3.58 0 8s3.58 8 8 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 8 14 6 6 0 1 1 8 2c1.66 0 3.14.69 4.22 1.78L9 7h7V0l-2.35 2.35z" fill="currentColor"/>
              </svg>
            </button>
          </div>
          {#snippet favCellSnippet({ row, col, value }: { row: any; col: any; value: any })}
            {#if col.key === "remove"}
              <button
                class="bookmark-btn bookmarked"
                class:toggling={togglingSet.has(row._original?.register_index ?? row.register_index)}
                onclick={(e: MouseEvent) => handleRemoveFavorite(row, e)}
                title="お気に入り解除"
              >
                {#if togglingSet.has(row._original?.register_index ?? row.register_index)}
                  <span class="bookmark-spinner"></span>
                {:else}
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                    <path d="M4 2h8a1 1 0 0 1 1 1v11.5l-5-3-5 3V3a1 1 0 0 1 1-1z"
                      fill="var(--accent)" stroke="var(--accent)" stroke-width="1.2"
                    />
                  </svg>
                {/if}
              </button>
            {:else}
              {value ?? ""}
            {/if}
          {/snippet}
          <DataTable data={displayFavorites} columns={favColumns} cellSnippet={favCellSnippet} onrowclick={(row: any) => handleRowClick(row)} />
        {:else if favorites}
          <div class="empty-message">お気に入りはありません。</div>
        {/if}
      {/if}
    {:else if result}
      <div class="empty-message">該当する授業が見つかりませんでした。</div>
    {/if}
  {/if}
</div>

<style>
  .view {
    padding: 0;
  }

  h2 {
    font-size: 20px;
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--text-primary);
  }

  .search-form {
    background: var(--bg-secondary);
    border: 0.5px solid var(--border);
    border-radius: 10px;
    padding: 0;
    margin-bottom: 16px;
    overflow: hidden;
  }

  .form-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 12px 16px;
    border: none;
    background: none;
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    color: var(--text-primary);
    text-align: left;
  }

  .form-header:hover {
    background: var(--bg-hover);
  }

  .form-header-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    font-size: 12px;
    color: var(--text-secondary);
    flex-shrink: 0;
  }

  .chevron {
    transition: transform 0.2s ease;
    color: var(--text-tertiary);
  }

  .chevron.rotated {
    transform: rotate(90deg);
  }

  .search-summary {
    font-size: 12px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .form-body {
    padding: 0 16px 16px;
  }

  .form-body.hidden {
    display: none;
  }

  .form-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .span-2 {
    grid-column: span 2;
  }

  .field-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    letter-spacing: 0.02em;
  }

  .year-range {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .input-year {
    width: 64px;
  }

  .range-sep {
    color: var(--text-secondary);
    font-size: 13px;
  }

  select, input[type="text"] {
    height: 28px;
    padding: 0 8px;
    border: 0.5px solid var(--border-strong);
    border-radius: 6px;
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color 0.15s;
  }

  select:focus, input[type="text"]:focus {
    border-color: var(--accent);
  }

  select {
    cursor: pointer;
  }

  .form-actions {
    display: flex;
    gap: 8px;
    margin-top: 16px;
    padding-top: 12px;
    border-top: 0.5px solid var(--border);
  }

  .btn-primary, .btn-secondary {
    height: 30px;
    padding: 0 16px;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    transition: opacity 0.15s;
    min-width: 64px;
  }

  .btn-primary {
    background: var(--accent);
    color: #fff;
  }

  .btn-primary:hover:not(:disabled) {
    opacity: 0.85;
  }

  .btn-secondary {
    background: var(--bg-tertiary);
    color: var(--text-primary);
    border: 0.5px solid var(--border-strong);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .btn-primary:disabled, .btn-secondary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255,255,255,0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .error-message {
    background: rgba(255, 59, 48, 0.08);
    color: var(--destructive);
    padding: 10px 14px;
    border-radius: 8px;
    font-size: 13px;
    margin-bottom: 16px;
    white-space: pre-line;
  }

  .result-info {
    font-size: 12px;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .fav-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .fav-timetable-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-left: auto;
    cursor: pointer;
    font-size: 12px;
    color: var(--text-secondary);
    user-select: none;
  }
  .fav-timetable-toggle input {
    accent-color: var(--accent);
  }
  .toggle-label {
    white-space: nowrap;
  }

  .btn-refresh {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--text-tertiary);
    border-radius: 4px;
    transition: all 0.15s;
  }

  .btn-refresh:hover {
    color: var(--accent);
    background: var(--bg-hover);
  }

  .empty-message {
    text-align: center;
    padding: 40px 20px;
    color: var(--text-tertiary);
    font-size: 14px;
  }

  .segmented-control {
    display: inline-flex;
    background: var(--bg-tertiary);
    border-radius: 8px;
    padding: 2px;
    margin-bottom: 12px;
    border: 0.5px solid var(--border);
  }

  .segment {
    padding: 5px 14px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    color: var(--text-secondary);
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .segment:hover:not(.active) {
    color: var(--text-primary);
  }

  .segment.active {
    background: var(--bg-primary);
    color: var(--text-primary);
    box-shadow: 0 0.5px 2px rgba(0, 0, 0, 0.08), 0 0 0 0.5px rgba(0, 0, 0, 0.04);
  }

  .bookmark-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
    border-radius: 5px;
    transition: all 0.15s ease;
    opacity: 0.6;
  }

  .bookmark-btn:hover {
    opacity: 1;
    background: var(--bg-hover);
  }

  .bookmark-btn.bookmarked {
    opacity: 1;
  }

  .bookmark-btn.toggling {
    opacity: 0.35;
    pointer-events: none;
  }

  .bookmark-spinner {
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .loading-message {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    color: var(--text-secondary);
    font-size: 14px;
  }

  :global(.col-bookmark) { width: 36px; }
  :global(.col-year) { width: 60px; }
  :global(.col-dept) { width: 100px; }
  :global(.col-code) { width: 80px; }
  :global(.col-title) { min-width: 160px; }
  :global(.col-instructor) { width: 120px; }
  :global(.col-term) { width: 70px; }
  :global(.col-dayperiod) { width: 90px; }
  :global(.col-campus) { width: 80px; }
</style>
