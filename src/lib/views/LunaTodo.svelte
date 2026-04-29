<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { lunaInvoke, aiAnalyzeTodo } from "../api";
  import { cachedBackendFetch, refreshBackendManagedCache, onCacheUpdate, lunaAuthState, aiTodoStore, aiReady } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import AiTodoPage from "./AiTodoPage.svelte";
  import type { LunaTodoItem, AiTodoAnalysis } from "../types";

  let loading = $state(true);
  let error = $state("");
  let todoItems = $state<LunaTodoItem[]>([]);
  let selectedCourse = $state("all");
  let hideOverdue = $state(true);

  // AI state
  let aiResult = $state<AiTodoAnalysis | null>(null);
  let aiLoading = $state(false);
  let showAiPage = $state(false);

  // Drop tasks that are more than 7 days overdue — at that point Luna almost
  // always disallows submission, so they only clutter the list and counts.
  const STALE_OVERDUE_MS = 7 * 86400_000;
  let pending = $derived(todoItems.filter(t => {
    if (t.status.includes("提出済")) return false;
    if (!t.deadline) return true;
    const overdueBy = Date.now() - parseDeadline(t.deadline);
    return overdueBy < STALE_OVERDUE_MS;
  }));
  let hasOverdue = $derived(pending.some(t => urgency(t.deadline) === "overdue"));
  let overdueCount = $derived(pending.filter(t => urgency(t.deadline) === "overdue").length);
  let displayCount = $derived(hideOverdue ? pending.length - overdueCount : pending.length);

  let courses = $derived(
    [...new Set(pending.map(t => t.course_name))].filter(Boolean).sort()
  );

  let courseCounts = $derived(
    pending.reduce((m, t) => {
      if (t.course_name) m.set(t.course_name, (m.get(t.course_name) || 0) + 1);
      return m;
    }, new Map<string, number>())
  );

  let filtered = $derived.by(() => {
    let items = pending;
    if (hideOverdue) items = items.filter(t => urgency(t.deadline) !== "overdue");
    if (selectedCourse !== "all") items = items.filter(t => t.course_name === selectedCourse);
    return items.slice().sort((a, b) => parseDeadline(a.deadline) - parseDeadline(b.deadline));
  });

  async function refresh() {
    loading = true;
    error = "";
    try {
      todoItems = await refreshBackendManagedCache("luna_todo");
    } catch (e: any) {
      error = String(e);
    }
    loading = false;
  }

  function parseDeadline(d: string): number {
    if (!d) return Infinity;
    return new Date(d.replace(/\//g, "-")).getTime();
  }

  function urgency(deadline: string): "overdue" | "critical" | "soon" | "normal" {
    if (!deadline) return "normal";
    const diff = parseDeadline(deadline) - Date.now();
    if (diff <= 0) return "overdue";
    if (diff < 1 * 86400_000) return "critical";
    if (diff < 2 * 86400_000) return "soon";
    if (diff <= 4 * 86400_000) return "soon";
    return "normal";
  }

  function urgencyPct(deadline: string): number {
    if (!deadline) return 0;
    const diff = parseDeadline(deadline) - Date.now();
    if (diff <= 0) return 1;
    const horizon = 7 * 86400_000;
    if (diff >= horizon) return 0;
    return 1 - diff / horizon;
  }

  function remainingLabel(deadline: string): string {
    if (!deadline) return "";
    const diff = parseDeadline(deadline) - Date.now();
    if (diff <= 0) {
      const elapsed = -diff;
      if (elapsed < 3600_000) return `${Math.floor(elapsed / 60_000)}分超過`;
      if (elapsed < 86400_000) return `${Math.floor(elapsed / 3600_000)}時間超過`;
      return `${Math.floor(elapsed / 86400_000)}日超過`;
    }
    if (diff < 3600_000) return `残り${Math.ceil(diff / 60_000)}分`;
    if (diff < 86400_000) {
      const h = Math.floor(diff / 3600_000);
      return `残り${h}時間`;
    }
    return `残り${Math.floor(diff / 86400_000)}日`;
  }

  async function openDetail(path: string, title: string) {
    if (!path) return;
    try {
      // Detect mode from URL path and extract params for proper window setup
      const params: Record<string, any> = { path, title };
      const urlParts = new URLSearchParams(path.split('?')[1] || '');
      const idnumber = urlParts.get('idnumber') || undefined;

      if (path.includes('/report/submission')) {
        params.mode = 'report';
        params.idnumber = idnumber;
        params.infoId = urlParts.get('reportId') || undefined;
      } else if (path.includes('/forums/themetop')) {
        params.mode = 'discussion';
      } else if (path.includes('/forums/thread')) {
        params.mode = 'thread';
      } else if (path.includes('/surveys/take') || path.includes('/course/surveys')) {
        params.mode = 'survey';
      }

      await lunaInvoke("university_open_detail_window", params);
    } catch (e: any) {
      console.error("Failed to open detail window:", e);
    }
  }

  async function enterAiMode() {
    showAiPage = true;
    // Pre-load cached result if not already loaded
    if (!aiResult && !aiLoading) {
      aiLoading = true;
      try {
        aiResult = await aiAnalyzeTodo(false);
      } catch { /* AI page handles errors itself */ }
      aiLoading = false;
    }
  }

  const unsubTodo = onCacheUpdate<LunaTodoItem[]>("luna_todo", (fresh) => { todoItems = fresh; });
  // Subscribe to AI scheduler updates
  const unsubAiTodo = aiTodoStore.subscribe((val) => {
    if (val?.result) aiResult = val.result;
  });
  onDestroy(() => { unsubTodo(); unsubAiTodo(); });

  onMount(async () => {
    loading = true;
    error = "";
    try {
      todoItems = await cachedBackendFetch("luna_todo");
    } catch (e: any) {
      error = String(e);
    }
    loading = false;
    // Pre-fetch cached AI result (non-blocking)
    if (pending.length > 0) {
      aiLoading = true;
      aiAnalyzeTodo(false).then(r => { aiResult = r; }).catch(() => {}).finally(() => { aiLoading = false; });
    }
  });
</script>

{#if showAiPage}
  <div class="view">
    <AiTodoPage initial={aiResult} onBack={() => showAiPage = false} />
  </div>
{:else}
<div class="view">
  <div class="title-row">
    <div class="title-left">
      <h2>TODO</h2>
      {#if pending.length > 0}
        {#if hasOverdue}
          <button class="count-btn" class:count-warn={displayCount >= 10} class:hiding={hideOverdue} onclick={() => hideOverdue = !hideOverdue}>
            {displayCount}
            <svg class="count-eye" width="11" height="11" viewBox="0 0 16 16" fill="none">
              {#if hideOverdue}
                <path d="M3 8c1-2.5 3-4 5-4s4 1.5 5 4c-1 2.5-3 4-5 4s-4-1.5-5-4z" stroke="currentColor" stroke-width="1.4" fill="none"/>
                <line x1="2" y1="14" x2="14" y2="2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>
              {:else}
                <path d="M3 8c1-2.5 3-4 5-4s4 1.5 5 4c-1 2.5-3 4-5 4s-4-1.5-5-4z" stroke="currentColor" stroke-width="1.4" fill="none"/>
                <circle cx="8" cy="8" r="1.8" stroke="currentColor" stroke-width="1.4" fill="none"/>
              {/if}
            </svg>
          </button>
        {:else}
          <span class="count" class:count-warn={pending.length >= 10}>{pending.length}</span>
        {/if}
      {/if}
    </div>
    <div class="title-actions">
      {#if pending.length > 0}
        <button class="ai-pill" onclick={enterAiMode} disabled={!$aiReady || (aiLoading && !aiResult)}
          title={!$aiReady ? 'AI 利用不可（設定で有効化）' : 'AI 輔助モード'}>
          <svg class="ai-pill-icon" width="12" height="12" viewBox="0 0 20 20" fill="none" class:spin={aiLoading && !aiResult}>
            {#if aiLoading && !aiResult}
              <circle cx="10" cy="10" r="7.5" stroke="currentColor" stroke-width="1.5" fill="none" stroke-dasharray="35 12" stroke-linecap="round"/>
            {:else}
              <path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none"/><path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke="currentColor" stroke-width="1" stroke-linejoin="round" fill="none"/>
            {/if}
          </svg>
          <span class="ai-pill-label">AI 辅助モード</span>
          <svg class="ai-pill-arrow" width="6" height="10" viewBox="0 0 6 10" fill="none">
            <path d="M1 1l4 4-4 4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </button>
      {/if}
      <button class="refresh-btn" onclick={refresh} disabled={loading} aria-label="更新" title="更新">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" class:spin={loading}>
          <path d="M14 8A6 6 0 1 1 8 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
          <path d="M14 2v4h-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>
  </div>

  {#if pending.length > 1 && courses.length > 1}
    <div class="filters">
      <button class="chip" class:active={selectedCourse === "all"} onclick={() => selectedCourse = "all"}>
        すべて
      </button>
      {#each courses as course}
        <button class="chip" class:active={selectedCourse === course} onclick={() => selectedCourse = course}>
          {course} <span class="chip-count">{courseCounts.get(course)}</span>
        </button>
      {/each}
    </div>
  {/if}

  <ViewLoader {loading} {error} empty={pending.length === 0 && !loading} emptyMessage="すべて完了しました">
    {#if !$lunaAuthState.authenticated && todoItems.length === 0 && !loading}
      <div class="empty-msg">Luna LMSに接続されていません</div>
    {:else}
      {#if filtered.length === 0}
        <div class="empty-msg">該当するTODOはありません</div>
      {:else}
        <div class="task-list">
          {#each filtered as item, i}
            {@const urg = urgency(item.deadline)}
            {@const pct = urgencyPct(item.deadline)}
            {@const remaining = remainingLabel(item.deadline)}
            <button
              class="task"
              class:overdue={urg === "overdue"}
              class:critical={urg === "critical"}
              class:soon={urg === "soon"}
              style="--delay: {Math.min(i * 0.05, 0.5)}s"
              onclick={() => openDetail(item.url, item.content_name || item.content_type)}
            >
              <div class="urgency-bar" class:overdue={urg === "overdue"} class:critical={urg === "critical"} class:soon={urg === "soon"}>
                <div class="urgency-fill" style="height: {Math.max(pct * 100, 6)}%"></div>
              </div>
              <div class="task-body">
                <div class="task-name">{item.content_name}</div>
                <div class="task-sub">
                  <span class="task-course">{item.course_name}</span>
                  <span class="task-sep"></span>
                  <span class="task-type">{item.content_type}</span>
                  {#if item.deadline}
                    <span class="task-sep"></span>
                    <span class="task-date">{item.deadline}</span>
                  {/if}
                </div>
                {#if item.feedback}
                  <div class="task-feedback">{item.feedback}</div>
                {/if}
              </div>
              {#if remaining}
                <span class="remaining" class:overdue={urg === "overdue"} class:critical={urg === "critical"} class:soon={urg === "soon"}>{remaining}</span>
              {/if}
              <svg class="task-arrow" width="7" height="12" viewBox="0 0 7 12" fill="none">
                <path d="M1 1l5 5-5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  </ViewLoader>
</div>
{/if}

<style>
  /* ── Title row (matches other views) ── */
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 12px;
  }
  .title-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title-left h2, .title-row h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .count {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--accent-light);
    padding: 3px 10px;
    border-radius: 12px;
  }
  .count-warn {
    color: var(--orange);
    background: rgba(255, 149, 0, 0.12);
  }
  .refresh-btn {
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all 0.15s;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .refresh-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .refresh-btn:disabled { opacity: 0.4; cursor: default; }
  .spin { animation: spin 0.8s linear infinite; }

  /* ── Count button (overdue toggle) ── */
  .count-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 600;
    font-family: inherit;
    color: var(--text-secondary);
    background: var(--accent-light);
    border: none;
    border-radius: 12px;
    padding: 3px 10px;
    cursor: pointer;
    transition: all 0.15s;
  }
  .count-btn:hover { opacity: 0.8; }
  .count-btn.count-warn { color: var(--orange); background: rgba(255, 149, 0, 0.12); }
  .count-btn.hiding { color: var(--orange); background: rgba(255, 149, 0, 0.08); }
  .count-eye { flex-shrink: 0; opacity: 0.6; }
  .count-btn:hover .count-eye { opacity: 1; }

  /* ── Filters ── */
  .filters {
    display: flex;
    gap: 5px;
    overflow-x: auto;
    margin-bottom: 12px;
    scrollbar-width: none;
    padding-bottom: 2px;
    cursor: grab;
  }
  .filters:active { cursor: grabbing; }
  .filters::-webkit-scrollbar { display: none; }
  .chip {
    flex-shrink: 0;
    padding: 5px 14px;
    border-radius: 16px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    transition: all 0.2s cubic-bezier(0.2, 0.8, 0.2, 1);
    white-space: nowrap;
  }
  .chip:hover { background: var(--bg-hover); }
  .chip.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
    box-shadow: 0 1px 6px rgba(0, 40, 85, 0.2);
  }
  .chip-count {
    font-size: 10px;
    font-weight: 600;
    opacity: 0.6;
    margin-left: 2px;
  }
  .chip.active .chip-count {
    opacity: 0.8;
  }

  /* ── Empty state ── */
  .empty-msg {
    text-align: center;
    color: var(--text-tertiary);
    font-size: 14px;
    padding: 48px 0;
  }

  /* ── Task list ── */
  .task-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .task {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 14px;
    border-radius: 12px;
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    width: 100%;
    transition: all 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
    animation: task-in 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) var(--delay) both;
    position: relative;
  }
  .task:hover {
    background: var(--bg-hover);
  }
  .task:active {
    transform: scale(0.99);
    transition-duration: 0.08s;
  }

  @keyframes task-in {
    from {
      opacity: 0;
      transform: translateY(12px) scale(0.97);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  /* ── Urgency progress bar ── */
  .urgency-bar {
    flex-shrink: 0;
    width: 4px;
    height: 36px;
    border-radius: 2px;
    background: var(--accent-light);
    overflow: hidden;
    position: relative;
    align-self: stretch;
    margin: 2px 0;
  }
  .urgency-fill {
    position: absolute;
    bottom: 0;
    left: 0;
    width: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: height 0.5s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .urgency-bar.overdue .urgency-fill { background: var(--red); }
  .urgency-bar.overdue { background: rgba(255, 59, 48, 0.15); }
  .urgency-bar.critical .urgency-fill {
    background: var(--orange);
    animation: bar-pulse 2s ease-in-out infinite;
  }
  .urgency-bar.critical { background: rgba(255, 149, 0, 0.15); }
  .urgency-bar.soon .urgency-fill { background: #e6b800; }
  .urgency-bar.soon { background: rgba(245, 197, 66, 0.15); }

  @keyframes bar-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  /* ── Remaining label ── */
  .remaining {
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--accent-light);
    color: var(--accent);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .remaining.overdue {
    background: rgba(255, 59, 48, 0.1);
    color: var(--red);
  }
  .remaining.critical {
    background: rgba(255, 149, 0, 0.1);
    color: var(--orange);
  }
  .remaining.soon {
    background: rgba(245, 197, 66, 0.12);
    color: #b8900a;
  }

  /* ── Task body ── */
  .task-body {
    flex: 1;
    min-width: 0;
  }
  .task-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.35;
    margin-bottom: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .task-sub {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--text-tertiary);
    flex-wrap: wrap;
  }
  .task-sep {
    width: 2px;
    height: 2px;
    border-radius: 50%;
    background: var(--text-tertiary);
    flex-shrink: 0;
    opacity: 0.5;
  }
  .task-course {
    font-weight: 500;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .task-type, .task-date {
    white-space: nowrap;
  }
  .task-date {
    font-variant-numeric: tabular-nums;
  }
  .task-feedback {
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  /* ── Arrow ── */
  .task-arrow {
    flex-shrink: 0;
    color: var(--text-tertiary);
    opacity: 0;
    transform: translateX(-4px);
    transition: all 0.2s ease;
  }
  .task:hover .task-arrow {
    opacity: 0.6;
    transform: translateX(0);
  }

  /* ── Title actions ── */
  .title-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* ── AI Capsule Pill ── */
  .ai-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border-radius: 50px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    border: 0.5px solid rgba(175, 82, 222, 0.25);
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.08), rgba(0, 122, 255, 0.06));
    transition: all 0.2s;
    white-space: nowrap;
    max-width: 280px;
  }
  .ai-pill:hover {
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.15), rgba(0, 122, 255, 0.12));
    border-color: rgba(175, 82, 222, 0.4);
  }
  .ai-pill:active { transform: scale(0.97); }
  .ai-pill:disabled { opacity: 0.5; cursor: default; }
  .ai-pill-icon {
    flex-shrink: 0;
    color: rgba(175, 82, 222, 0.85);
  }
  .ai-pill-label {
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.9), rgba(0, 122, 255, 0.9));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }
  .ai-pill-arrow {
    flex-shrink: 0;
    color: rgba(175, 82, 222, 0.5);
    transition: all 0.2s;
  }
  .ai-pill:hover .ai-pill-arrow {
    color: rgba(175, 82, 222, 0.85);
    transform: translateX(1px);
  }

</style>
