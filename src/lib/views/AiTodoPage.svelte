<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { aiAnalyzeTodo } from "../api";
  import type { AiTodoAnalysis, AiTodoTaskGuide } from "../types";

  interface Props {
    initial: AiTodoAnalysis | null;
    onBack: () => void;
  }

  let { initial, onBack }: Props = $props();

  let result = $state<AiTodoAnalysis | null>(null);
  let loading = $state(false);
  let error = $state("");

  $effect(() => {
    result = initial;
  });

  // Cycling tips from overall_advice
  let tipIndex = $state(0);
  let tipFade = $state(true);
  let tipInterval: ReturnType<typeof setInterval> | undefined;
  let tipTimeout: ReturnType<typeof setTimeout> | undefined;

  let tipSentences = $derived(
    result?.advice
      ? result.advice.split(/[。！？\n]+/).map(s => s.trim()).filter(s => s.length > 2)
      : []
  );
  let tipText = $derived(tipSentences.length > 0 ? tipSentences[tipIndex % tipSentences.length] : "");

  // Expanded task keys: "dayIndex-taskIndex"
  let expandedTasks = $state(new Set<string>());

  function toggleTask(key: string) {
    const next = new Set(expandedTasks);
    if (next.has(key)) next.delete(key); else next.add(key);
    expandedTasks = next;
  }

  // Build a map from task_name to guide for quick lookup
  let guideMap = $derived.by(() => {
    const m = new Map<string, AiTodoTaskGuide>();
    if (!result?.task_guides) return m;
    for (const g of result.task_guides) {
      m.set(g.task_name, g);
    }
    return m;
  });

  // Match a daily_plan task string to its guide (by substring match on task_name)
  function findGuide(taskStr: string): AiTodoTaskGuide | undefined {
    for (const [name, guide] of guideMap) {
      if (taskStr.includes(name) || name.includes(taskStr.replace(/（.*?）/g, "").trim())) {
        return guide;
      }
    }
    return undefined;
  }

  function urgencyLabel(u: string): string {
    switch (u) {
      case "overdue": return "期限超過";
      case "critical": return "緊急";
      case "soon": return "まもなく";
      default: return "";
    }
  }

  async function reanalyze() {
    if (loading) return;
    loading = true;
    error = "";
    try {
      result = await aiAnalyzeTodo(true);
      expandedTasks = new Set();
      startTipCycle();
    } catch (e: any) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function startTipCycle() {
    stopTipCycle();
    if (tipSentences.length <= 1) return;
    tipInterval = setInterval(() => {
      tipFade = false;
      tipTimeout = setTimeout(() => {
        tipIndex = (tipIndex + 1) % tipSentences.length;
        tipFade = true;
      }, 300);
    }, 6000);
  }

  function stopTipCycle() {
    if (tipTimeout) { clearTimeout(tipTimeout); tipTimeout = undefined; }
    if (tipInterval) { clearInterval(tipInterval); tipInterval = undefined; }
  }

  onMount(() => {
    if (!result) {
      reanalyze();
    } else {
      startTipCycle();
    }
  });
  onDestroy(() => stopTipCycle());
</script>

<div class="ai-page">
  <!-- Header -->
  <div class="header">
    <div class="header-left">
      <button class="back-btn" onclick={onBack} title="TODOに戻る">
        <svg width="8" height="14" viewBox="0 0 8 14" fill="none">
          <path d="M7 1L1 7l6 6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
      <h2 class="header-title">AI 辅助モード</h2>
    </div>
    <button class="re-btn" onclick={reanalyze} disabled={loading} aria-label="再分析" title="再分析">
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" class:spin={loading}>
        <path d="M14 8A6 6 0 1 1 8 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        <path d="M14 2v4h-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    </button>
  </div>

  {#if error}
    <div class="err">{error}</div>
  {/if}

  {#if loading && !result}
    <div class="loading-state">
      <div class="loading-dots">
        <span></span><span></span><span></span>
      </div>
      <span class="loading-text">タスクとコース情報を分析中...</span>
    </div>
  {:else if result}
    <!-- Advice ticker -->
    {#if tipSentences.length > 0}
      <div class="advice-bar">
        <svg class="advice-icon" width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3">
          <path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke-linejoin="round"/>
          <path d="M15 13l1 2.2L18.2 16l-2.2 1L15 19.2 14 17l-2.2-1L14 15z" stroke-linejoin="round" stroke-width="1"/>
        </svg>
        <span class="advice-text" class:fade={!tipFade}>{tipText}</span>
      </div>
    {/if}

    <!-- Merged daily plan + guides -->
    {#if result.daily_plan && result.daily_plan.length > 0}
      <div class="timeline">
        {#each result.daily_plan as day, di}
          <div class="day-block" style="--di: {di}">
            <div class="day-header">
              <div class="day-dot"></div>
              <span class="day-label">{day.label}</span>
              {#if day.free_hours}
                <span class="day-hours">{day.free_hours}h 空き</span>
              {/if}
            </div>

            <div class="day-tasks">
              {#each day.tasks as taskStr, ti}
                {@const key = `${di}-${ti}`}
                {@const guide = findGuide(taskStr)}
                {@const isOpen = expandedTasks.has(key)}
                <button class="task-card" class:has-guide={!!guide} class:expanded={isOpen} onclick={() => guide && toggleTask(key)}>
                  <div class="task-row">
                    <div class="task-info">
                      {#if guide?.urgency && guide.urgency !== "normal"}
                        <span class="task-urg {guide.urgency}">{urgencyLabel(guide.urgency)}</span>
                      {/if}
                      <span class="task-label">{taskStr}</span>
                      {#if guide?.live_note_summary}
                        <span class="task-note">Live</span>
                      {/if}
                    </div>
                    {#if guide}
                      <svg class="task-chev" class:open={isOpen} width="10" height="10" viewBox="0 0 10 10" fill="none">
                        <path d="M2.5 3.75L5 6.25L7.5 3.75" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
                      </svg>
                    {/if}
                  </div>
                  {#if guide}
                    <div class="task-meta">
                      <span>{guide.course_name}</span>
                      {#if guide.deadline}
                        <span class="dot"></span>
                        <span>{guide.deadline}</span>
                      {/if}
                    </div>
                  {/if}
                  {#if isOpen && guide}
                    <div class="task-detail">
                      {#if guide.background}
                        <div class="detail-block">
                          <div class="detail-label">
                            <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke-linejoin="round"/></svg>
                            関連知識
                          </div>
                          <div class="detail-text">{guide.background}</div>
                        </div>
                      {/if}
                      {#if guide.live_note_summary}
                        <div class="detail-block">
                          <div class="detail-label">
                            <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M4.5 4.5h11v11h-11z"/><path d="M7 8h6M7 11h6M7 14h4" stroke-linecap="round"/></svg>
                            Live ノート
                          </div>
                          <div class="detail-text">{guide.live_note_summary}</div>
                        </div>
                      {/if}
                      {#if guide.study_hints && guide.study_hints.length > 0}
                        <div class="detail-block">
                          <div class="detail-label">
                            <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke-linejoin="round"/></svg>
                            取り組み手順
                          </div>
                          <ol class="detail-ol">
                            {#each guide.study_hints as hint, hi}
                              <li style="--si: {hi}">{hint}</li>
                            {/each}
                          </ol>
                        </div>
                      {/if}
                      {#if guide.ready_to_use}
                        <div class="detail-block">
                          <div class="detail-label">
                            <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M5 4.5h10v11H5z"/><path d="M8 8h4M8 11h6M8 14h5" stroke-linecap="round"/></svg>
                            {guide.ready_to_use_label || "すぐ使える下書き"}
                          </div>
                          <div class="detail-text detail-ready">{guide.ready_to_use}</div>
                        </div>
                      {/if}
                    </div>
                  {/if}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}

    <!-- Guides not in any day (orphans) -->
    {#if result.task_guides && result.task_guides.length > 0}
      {@const planTaskNames = (result.daily_plan ?? []).flatMap(d => d.tasks)}
      {@const orphans = result.task_guides.filter(g => !planTaskNames.some(t => t.includes(g.task_name) || g.task_name.includes(t.replace(/（.*?）/g, "").trim())))}
      {#if orphans.length > 0}
        <div class="orphan-section">
          <div class="orphan-title">その他のタスク</div>
          {#each orphans as guide, i}
            {@const key = `o-${i}`}
            {@const isOpen = expandedTasks.has(key)}
            <button class="task-card has-guide" class:expanded={isOpen} onclick={() => toggleTask(key)}>
              <div class="task-row">
                <div class="task-info">
                  {#if guide.urgency !== "normal"}
                    <span class="task-urg {guide.urgency}">{urgencyLabel(guide.urgency)}</span>
                  {/if}
                  <span class="task-label">{guide.task_name}</span>
                  {#if guide.live_note_summary}
                    <span class="task-note">Live</span>
                  {/if}
                  {#if guide.estimated_minutes}
                    <span class="task-time">{guide.estimated_minutes}min</span>
                  {/if}
                </div>
                <svg class="task-chev" class:open={isOpen} width="10" height="10" viewBox="0 0 10 10" fill="none">
                  <path d="M2.5 3.75L5 6.25L7.5 3.75" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
              </div>
              <div class="task-meta">
                <span>{guide.course_name}</span>
                {#if guide.deadline}
                  <span class="dot"></span>
                  <span>{guide.deadline}</span>
                {/if}
              </div>
              {#if isOpen}
                <div class="task-detail">
                  {#if guide.background}
                    <div class="detail-block">
                      <div class="detail-label">
                        <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke-linejoin="round"/></svg>
                        関連知識
                      </div>
                      <div class="detail-text">{guide.background}</div>
                    </div>
                  {/if}
                  {#if guide.live_note_summary}
                    <div class="detail-block">
                      <div class="detail-label">
                        <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M4.5 4.5h11v11h-11z"/><path d="M7 8h6M7 11h6M7 14h4" stroke-linecap="round"/></svg>
                        Live ノート
                      </div>
                      <div class="detail-text">{guide.live_note_summary}</div>
                    </div>
                  {/if}
                  {#if guide.study_hints && guide.study_hints.length > 0}
                    <div class="detail-block">
                      <div class="detail-label">
                        <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M10 2l2 4.5L16.5 8l-4.5 2L10 14.5 8 10 3.5 8l4.5-2z" stroke-linejoin="round"/></svg>
                        取り組み手順
                      </div>
                      <ol class="detail-ol">
                        {#each guide.study_hints as hint, hi}
                          <li style="--si: {hi}">{hint}</li>
                        {/each}
                      </ol>
                    </div>
                  {/if}
                  {#if guide.ready_to_use}
                    <div class="detail-block">
                      <div class="detail-label">
                        <svg width="10" height="10" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M5 4.5h10v11H5z"/><path d="M8 8h4M8 11h6M8 14h5" stroke-linecap="round"/></svg>
                        {guide.ready_to_use_label || "すぐ使える下書き"}
                      </div>
                      <div class="detail-text detail-ready">{guide.ready_to_use}</div>
                    </div>
                  {/if}
                </div>
              {/if}
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .ai-page {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  /* ── Header ── */
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 16px;
  }
  .header-left {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .back-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all 0.15s;
  }
  .back-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .header-title {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }
  .re-btn {
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
  .re-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .re-btn:disabled { opacity: 0.4; cursor: default; }

  .err {
    font-size: 12px;
    color: var(--red);
    margin-bottom: 10px;
    padding: 6px 10px;
    border-radius: 8px;
    background: rgba(255, 59, 48, 0.08);
  }

  /* ── Loading ── */
  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 48px 0;
  }
  .loading-dots {
    display: flex;
    gap: 6px;
  }
  .loading-dots span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.7), rgba(0, 122, 255, 0.7));
    animation: dot-pulse 1.2s ease-in-out infinite;
  }
  .loading-dots span:nth-child(2) { animation-delay: 0.15s; }
  .loading-dots span:nth-child(3) { animation-delay: 0.3s; }
  .loading-text {
    font-size: 13px;
    color: var(--text-tertiary);
  }
  .spin { animation: spin 0.8s linear infinite; }
  @keyframes dot-pulse {
    0%, 80%, 100% { opacity: 0.3; transform: scale(0.8); }
    40% { opacity: 1; transform: scale(1); }
  }

  /* ── Advice ticker ── */
  .advice-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-radius: 12px;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.06), rgba(0, 122, 255, 0.04));
    border: 0.5px solid rgba(175, 82, 222, 0.15);
    margin-bottom: 16px;
  }
  .advice-icon { flex-shrink: 0; color: rgba(175, 82, 222, 0.8); }
  .advice-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    line-height: 1.45;
    transition: opacity 0.3s;
  }
  .advice-text.fade { opacity: 0; }

  /* ── Timeline ── */
  .timeline {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding-left: 6px;
  }
  .day-block {
    position: relative;
    padding-left: 20px;
    padding-bottom: 20px;
    border-left: 1.5px solid rgba(175, 82, 222, 0.12);
    animation: card-in 0.3s cubic-bezier(0.2, 0.8, 0.2, 1) calc(var(--di) * 0.08s) both;
  }
  .day-block:last-child {
    border-left-color: transparent;
    padding-bottom: 0;
  }
  .day-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .day-dot {
    position: absolute;
    left: -5px;
    top: 3px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: linear-gradient(135deg, #af52de, #007aff);
    border: 2px solid var(--bg-primary, #fff);
  }
  @media (prefers-color-scheme: dark) {
    .day-dot { border-color: var(--bg-primary, #1c1c1e); }
  }
  .day-label {
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }
  .day-hours {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--bg-secondary);
    padding: 2px 8px;
    border-radius: 6px;
  }

  /* ── Task cards ── */
  .day-tasks {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .task-card {
    display: flex;
    flex-direction: column;
    width: 100%;
    text-align: left;
    font-family: inherit;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    transition: all 0.2s cubic-bezier(0.2, 0.8, 0.2, 1);
    cursor: default;
  }
  .task-card.has-guide {
    cursor: pointer;
  }
  .task-card.has-guide:hover { background: var(--bg-hover); }
  .task-card.expanded {
    border-color: rgba(175, 82, 222, 0.2);
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.03), rgba(0, 122, 255, 0.02));
  }

  .task-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .task-info {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1;
  }
  .task-urg {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 4px;
  }
  .task-urg.overdue { background: rgba(255,59,48,0.12); color: var(--red); }
  .task-urg.critical { background: rgba(255,149,0,0.12); color: var(--orange); }
  .task-urg.soon { background: rgba(245,197,66,0.12); color: #b8900a; }
  .task-note {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 999px;
    background: rgba(0, 122, 255, 0.1);
    color: #007aff;
  }
  .task-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .task-time {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--bg-secondary);
    padding: 1px 6px;
    border-radius: 4px;
  }
  .task-chev {
    flex-shrink: 0;
    color: var(--text-tertiary);
    transition: transform 0.2s;
  }
  .task-chev.open { transform: rotate(180deg); }
  .task-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 3px;
  }
  .dot {
    width: 2px;
    height: 2px;
    border-radius: 50%;
    background: var(--text-tertiary);
    opacity: 0.5;
  }

  /* ── Expanded detail ── */
  .task-detail {
    margin-top: 10px;
    padding-top: 10px;
    border-top: 0.5px solid rgba(175, 82, 222, 0.1);
    display: flex;
    flex-direction: column;
    gap: 10px;
    animation: detail-in 0.2s ease;
  }
  .detail-block {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .detail-label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 700;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.85), rgba(0, 122, 255, 0.85));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .detail-label svg {
    flex-shrink: 0;
    color: rgba(175, 82, 222, 0.8);
    -webkit-text-fill-color: initial;
  }
  .detail-text {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.6;
  }
  .detail-ready {
    white-space: pre-wrap;
  }
  .detail-ol {
    margin: 0;
    padding-left: 18px;
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.65;
  }
  .detail-ol li {
    margin-bottom: 3px;
    animation: step-in 0.2s ease calc(var(--si) * 0.05s) both;
  }
  .detail-ol li::marker {
    color: rgba(175, 82, 222, 0.8);
    font-weight: 700;
  }

  /* ── Orphan section ── */
  .orphan-section {
    margin-top: 16px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .orphan-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-tertiary);
    margin-bottom: 4px;
    padding-left: 2px;
  }

  /* ── Animations ── */
  @keyframes card-in {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @keyframes detail-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @keyframes step-in {
    from { opacity: 0; transform: translateX(-6px); }
    to { opacity: 1; transform: translateX(0); }
  }
</style>
