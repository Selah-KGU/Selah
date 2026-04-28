
<script lang="ts" generics="T">
import type { Snippet } from "svelte";

interface Column<T> {
  key: string;
  label: string;
  align?: "left" | "center" | "right";
  width?: string;
  class?: string;
  render?: (row: T) => string;
}

interface Props {
  data: T[];
  columns: Column<T>[];
  rowKey?: (row: T, i: number) => string | number;
  cellSnippet?: Snippet<[{ row: T; col: Column<T>; value: any }]>;
  onrowclick?: (row: T, event: MouseEvent) => void;
}

let { data, columns, rowKey, cellSnippet, onrowclick }: Props = $props();
</script>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        {#each columns as col}
          <th
            style:text-align={col.align ?? "left"}
            style:width={col.width ?? "auto"}
            class={col.class ?? ""}
          >
            {col.label}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each data as row, i (rowKey ? rowKey(row, i) : i)}
        <tr
          class:clickable={!!onrowclick}
          onclick={(e) => onrowclick?.(row, e)}
        >
          {#each columns as col}
            {@const value = col.render ? col.render(row) : (row as any)[col.key]}
            <td
              style:text-align={col.align ?? "left"}
              class={col.class ?? ""}
            >
              {#if cellSnippet}
                {@render cellSnippet({ row, col, value })}
              {:else}
                {value ?? ""}
              {/if}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .table-wrap {
    background: var(--bg-card);
    border-radius: 12px;
    box-shadow: var(--shadow-md);
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    animation: fade-in 0.3s ease;
  }
  table {
    width: max-content;
    min-width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  thead th {
    padding: 10px 14px;
    font-weight: 600;
    font-size: 12px;
    color: var(--text-secondary);
    background: var(--bg-secondary);
    border-bottom: 0.5px solid var(--border);
    position: sticky;
    top: 0;
    white-space: nowrap;
  }
  tbody tr {
    border-bottom: 0.5px solid var(--border);
    transition: background 0.12s ease;
  }
  tbody tr:last-child {
    border-bottom: none;
  }
  tbody tr:hover {
    background: var(--bg-hover);
  }
  tbody tr.clickable {
    cursor: pointer;
  }
  td {
    padding: 10px 14px;
    white-space: normal;
    word-break: break-word;
    line-height: 1.4;
    max-width: 300px;
  }
</style>
