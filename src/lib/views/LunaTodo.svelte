<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { lunaInvoke } from "../api";
  import { cachedFetch, onCacheUpdate, lunaAuthState } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import Icon from "../Icon.svelte";
  import type { LunaTodoItem } from "../types";

  let loading = $state(true);
  let error = $state("");
  let todoItems = $state<LunaTodoItem[]>([]);

  let pendingCount = $derived(todoItems.filter(t => !t.status.includes("提出済")).length);

  // SWR: update UI when background polling brings fresh data
  const unsubTodo = onCacheUpdate<LunaTodoItem[]>("luna_todo", (fresh) => { todoItems = fresh; });
  onDestroy(() => unsubTodo());

  onMount(async () => {
    await loadTodo();
  });

  async function loadTodo() {
    loading = true;
    error = "";
    try {
      todoItems = await cachedFetch("luna_todo", () => lunaInvoke<LunaTodoItem[]>("luna_fetch_todo"));
    } catch (e: any) {
      error = String(e);
    }
    loading = false;
  }

  function isOverdue(deadline: string): boolean {
    if (!deadline) return false;
    const d = new Date(deadline.replace(/\//g, "-"));
    return d < new Date();
  }

  async function openDetail(path: string, title: string) {
    if (!path) return;
    try {
      await lunaInvoke("luna_open_detail_window", { path, title });
    } catch (e: any) {
      console.error("Failed to open detail window:", e);
    }
  }
</script>

<div class="view">
  <div class="title-row">
    <h2>TODO</h2>
    <div class="title-controls">
      {#if pendingCount > 0}
        <span class="pending-badge">{pendingCount}件未提出</span>
      {/if}
      <button class="pill-btn" onclick={loadTodo} disabled={loading}>
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" class:spin={loading}>
          <path d="M14 8A6 6 0 1 1 8 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
          <path d="M14 2v4h-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        更新
      </button>
    </div>
  </div>

  <ViewLoader {loading} {error} empty={todoItems.length === 0 && !loading} emptyMessage="TODOはありません">
    {#if !$lunaAuthState.authenticated && todoItems.length === 0 && !loading}
      <div class="state-msg">Luna LMSに接続されていません</div>
    {:else}
      <div class="todo-list">
        {#each todoItems as item, i}
          {@const overdue = isOverdue(item.deadline) && item.status.includes("未提出")}
          {@const done = item.status.includes("提出済")}
          <button
            class="todo-card"
            class:overdue
            class:done
            style="animation: slide-up 0.3s ease {Math.min(i * 0.04, 0.4)}s both;"
            onclick={() => openDetail(item.url, item.content_name || item.content_type)}
          >
            <div class="todo-header">
              <span class="todo-type">{item.content_type}</span>
              <span class="todo-badge" class:badge-done={done} class:badge-pending={!done}>
                {done ? "提出済" : "未提出"}
              </span>
            </div>
            <div class="todo-title">{item.content_name}</div>
            <div class="todo-meta">
              <span class="todo-course">{item.course_name}</span>
              {#if item.deadline}
                <span class="todo-deadline" class:overdue>〆 {item.deadline}</span>
              {/if}
            </div>
            {#if item.feedback}
              <div class="todo-fb">{item.feedback}</div>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </ViewLoader>
</div>

<style>
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  .title-row h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .title-controls { display: flex; align-items: center; gap: 8px; }
  .pending-badge {
    font-size: 11px;
    padding: 3px 10px;
    border-radius: 12px;
    background: rgba(255, 149, 0, 0.12);
    color: var(--orange);
    font-weight: 600;
  }
  .pill-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 5px 11px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    transition: all 0.15s ease;
  }
  .pill-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .pill-btn:disabled { opacity: 0.5; cursor: default; }
  .spin { animation: spin 0.8s linear infinite; }

  .state-msg {
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
    padding: 40px 0;
  }
  .todo-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .todo-card {
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    border-radius: 10px;
    padding: 12px 16px;
    box-shadow: var(--shadow-sm);
    transition: background 0.15s ease, transform 0.15s ease, box-shadow 0.15s ease;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    width: 100%;
  }
  .todo-card:hover {
    background: var(--bg-hover);
    transform: translateX(2px);
    box-shadow: var(--shadow-md);
  }
  .todo-card.overdue {
    border-left: 3px solid var(--red);
  }
  .todo-card.done {
    opacity: 0.65;
  }
  .todo-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 4px;
  }
  .todo-type {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
  }
  .todo-badge {
    font-size: 10px;
    padding: 2px 8px;
    border-radius: 6px;
    font-weight: 600;
    letter-spacing: 0.3px;
  }
  .badge-done {
    background: rgba(52, 199, 89, 0.12);
    color: var(--green);
  }
  .badge-pending {
    background: rgba(255, 149, 0, 0.12);
    color: var(--orange);
  }
  .todo-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    margin-bottom: 6px;
    line-height: 1.4;
  }
  .todo-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12px;
    color: var(--text-tertiary);
  }
  .todo-course {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 55%;
  }
  .todo-deadline {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .todo-deadline.overdue {
    color: var(--red);
    font-weight: 600;
  }
  .todo-fb {
    margin-top: 6px;
    font-size: 12px;
    color: var(--text-tertiary);
    font-style: italic;
  }
</style>
