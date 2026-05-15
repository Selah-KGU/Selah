<script lang="ts">
  interface DraftItem {
    title: string;
    course_name: string;
    content_type?: string;
    deadline: string;
    note?: string;
    source_excerpt?: string;
    selected: boolean;
  }

  interface Props {
    title: string;
    subtitle: string;
    drafts: DraftItem[];
    saving?: boolean;
    inline?: boolean;
    courseFallback?: string;
    errorMessage?: string;
    confirmLabel?: string;
    onToggle: (index: number) => void;
    onClose: () => void;
    onConfirm: () => void;
  }

  let {
    title,
    subtitle,
    drafts,
    saving = false,
    inline = false,
    courseFallback = "",
    errorMessage = "",
    confirmLabel,
    onToggle,
    onClose,
    onConfirm,
  }: Props = $props();

  const selectedCount = $derived(drafts.filter((d) => d.selected).length);
</script>

<section class="todo-confirm-card" class:inline aria-label={title}>
  <div class="todo-confirm-head">
    <div>
      <div class="todo-confirm-title">{title}</div>
      <div class="todo-confirm-sub">{subtitle}</div>
    </div>
    <button class="todo-confirm-close" onclick={onClose} disabled={saving} aria-label="閉じる">×</button>
  </div>
  <div class="todo-draft-list">
    {#each drafts as item, idx}
      <label class="todo-draft-row" class:selected={item.selected}>
        <input
          type="checkbox"
          checked={item.selected}
          onchange={() => onToggle(idx)}
          disabled={saving}
        />
        <span class="todo-draft-main">
          <span class="todo-draft-title">{item.title}</span>
          <span class="todo-draft-meta">
            <span>{item.course_name || courseFallback}</span>
            {#if item.content_type}<span>{item.content_type}</span>{/if}
            <span class:missing={!item.deadline}>
              {item.deadline ? `DDL ${item.deadline}` : "DDL未判定"}
            </span>
          </span>
          {#if item.note}
            <span class="todo-draft-note">{item.note}</span>
          {/if}
          {#if item.source_excerpt}
            <span class="todo-draft-source">“{item.source_excerpt}”</span>
          {/if}
        </span>
      </label>
    {/each}
  </div>
  {#if errorMessage}
    <div class="todo-confirm-error">{errorMessage}</div>
  {/if}
  <div class="todo-confirm-actions">
    <button class="todo-confirm-btn secondary" onclick={onClose} disabled={saving}>追加しない</button>
    <button
      class="todo-confirm-btn primary"
      onclick={onConfirm}
      disabled={saving || selectedCount === 0}
    >
      {saving ? "追加中…" : (confirmLabel ?? `選択した${selectedCount}件を追加`)}
    </button>
  </div>
</section>

<style>
  .todo-confirm-card {
    width: fit-content;
    max-height: min(520px, 58vh);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border-radius: 12px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-sm);
    text-align: left;
  }

  .todo-confirm-card.inline {
    margin-top: 14px;
    align-self: center;
    width: 100%;
    max-width: 620px;
  }

  .todo-confirm-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .todo-confirm-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .todo-confirm-sub {
    margin-top: 3px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-secondary);
  }

  .todo-confirm-close {
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 50%;
    background: var(--bg-secondary);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
  }

  .todo-draft-list {
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-right: 2px;
  }

  .todo-draft-row {
    display: grid;
    grid-template-columns: 18px 1fr;
    gap: 10px;
    padding: 11px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg-secondary);
    cursor: pointer;
  }

  .todo-draft-row.selected {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-secondary));
  }

  .todo-draft-row input {
    margin-top: 2px;
    accent-color: var(--accent);
  }

  .todo-draft-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .todo-draft-title {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .todo-draft-meta,
  .todo-draft-note,
  .todo-draft-source {
    font-size: 11px;
    line-height: 1.45;
    color: var(--text-secondary);
  }

  .todo-draft-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .todo-draft-meta span {
    padding: 2px 7px;
    border-radius: 999px;
    background: var(--bg-tertiary);
  }

  .todo-draft-meta span.missing {
    color: var(--orange);
    background: color-mix(in srgb, var(--orange) 12%, transparent);
  }

  .todo-draft-source {
    color: var(--text-tertiary);
    word-break: break-word;
  }

  .todo-confirm-error {
    padding: 8px 12px;
    border-radius: 8px;
    background: rgba(255, 59, 48, 0.08);
    color: var(--red);
    font-size: 12px;
  }

  .todo-confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .todo-confirm-btn {
    border: none;
    border-radius: 999px;
    padding: 8px 13px;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
  }

  .todo-confirm-btn.secondary {
    background: var(--bg-secondary);
    color: var(--text-secondary);
  }

  .todo-confirm-btn.primary {
    background: var(--accent);
    color: white;
  }

  .todo-confirm-btn:disabled,
  .todo-confirm-close:disabled {
    opacity: 0.55;
    cursor: default;
  }
</style>
