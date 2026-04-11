<!-- SF Symbols-inspired SVG icons for Selah app -->
<script lang="ts" module>
  export type IconName =
    | "calendar"
    | "chart.bar"
    | "arrow.triangle.swap"
    | "list.clipboard"
    | "bell"
    | "sun"
    | "moon"
    | "xmark"
    | "book"
    | "chevron.left"
    | "chevron.right"
    | "checkmark.circle"
    | "paperclip"
    | "video"
    | "square.grid.2x2"
    | "gear"
    | "wrench"
    | "pin"
    | "building.2"
    | "envelope"
    | "arrow.clockwise"
    | "exclamationmark.triangle";
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    size?: number;
  }
  let { name, size = 20 }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  class="icon"
>
  {#if name === "calendar"}
    <rect x="3" y="4" width="18" height="18" rx="2" />
    <line x1="16" y1="2" x2="16" y2="6" />
    <line x1="8" y1="2" x2="8" y2="6" />
    <line x1="3" y1="10" x2="21" y2="10" />
    <rect x="7" y="13" width="3" height="3" rx="0.5" fill="currentColor" stroke="none" />
  {:else if name === "chart.bar"}
    <rect x="3" y="12" width="4" height="9" rx="1" />
    <rect x="10" y="7" width="4" height="14" rx="1" />
    <rect x="17" y="3" width="4" height="18" rx="1" />
  {:else if name === "arrow.triangle.swap"}
    <path d="M7 4v16" />
    <path d="M4 7l3-3 3 3" />
    <path d="M17 20V4" />
    <path d="M14 17l3 3 3-3" />
  {:else if name === "list.clipboard"}
    <path d="M9 5H7a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2" />
    <rect x="9" y="3" width="6" height="4" rx="1" />
    <line x1="9" y1="12" x2="15" y2="12" />
    <line x1="9" y1="16" x2="13" y2="16" />
  {:else if name === "bell"}
    <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
    <path d="M13.73 21a2 2 0 0 1-3.46 0" />
  {:else if name === "sun"}
    <circle cx="12" cy="12" r="5" />
    <line x1="12" y1="1" x2="12" y2="3" />
    <line x1="12" y1="21" x2="12" y2="23" />
    <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
    <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
    <line x1="1" y1="12" x2="3" y2="12" />
    <line x1="21" y1="12" x2="23" y2="12" />
    <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
    <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
  {:else if name === "moon"}
    <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
  {:else if name === "xmark"}
    <line x1="18" y1="6" x2="6" y2="18" />
    <line x1="6" y1="6" x2="18" y2="18" />
  {:else if name === "book"}
    <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
    <path d="M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z" />
    <circle cx="14" cy="10" r="3" />
    <line x1="16.2" y1="12.2" x2="18" y2="14" />
  {:else if name === "chevron.left"}
    <polyline points="15 18 9 12 15 6" />
  {:else if name === "chevron.right"}
    <polyline points="9 6 15 12 9 18" />
  {:else if name === "checkmark.circle"}
    <circle cx="12" cy="12" r="10" />
    <polyline points="9 12 11.5 14.5 16 9.5" />
  {:else if name === "paperclip"}
    <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
  {:else if name === "video"}
    <rect x="2" y="5" width="14" height="14" rx="2" />
    <path d="M22 7l-6 4 6 4V7z" fill="currentColor" />
  {:else if name === "square.grid.2x2"}
    <rect x="3" y="3" width="8" height="8" rx="1.5" />
    <rect x="13" y="3" width="8" height="8" rx="1.5" />
    <rect x="3" y="13" width="8" height="8" rx="1.5" />
    <rect x="13" y="13" width="8" height="8" rx="1.5" />
  {:else if name === "gear"}
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  {:else if name === "wrench"}
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
  {:else if name === "pin"}
    <path d="M7.3 2.1a2.6 2.6 0 0 1 4.24.76l.92 1.99a4.6 4.6 0 0 0 3.12 2.56l3.68.87c2 .48 2.7 2.96 1.25 4.41l-3.2 3.2 5.58 5.58a1.12 1.12 0 1 1-1.6 1.6L15.72 17.5l-3.3 3.31c-1.46 1.45-3.94.75-4.41-1.25l-.88-3.68a4.6 4.6 0 0 0-2.56-3.12l-1.99-.92A2.6 2.6 0 0 1 1.82 7.6L7.3 2.1Z" fill="currentColor" stroke="none" />
  {:else if name === "building.2"}
    <rect x="3" y="6" width="8" height="16" rx="1" />
    <rect x="13" y="2" width="8" height="20" rx="1" />
    <line x1="6" y1="9" x2="8" y2="9" />
    <line x1="6" y1="12" x2="8" y2="12" />
    <line x1="6" y1="15" x2="8" y2="15" />
    <line x1="16" y1="5" x2="18" y2="5" />
    <line x1="16" y1="8" x2="18" y2="8" />
    <line x1="16" y1="11" x2="18" y2="11" />
    <line x1="16" y1="14" x2="18" y2="14" />
    <line x1="16" y1="17" x2="18" y2="17" />
  {:else if name === "envelope"}
    <rect x="2" y="4" width="20" height="16" rx="2" />
    <polyline points="22 4 12 13 2 4" />
  {:else if name === "arrow.clockwise"}
    <path d="M21 2v6h-6" />
    <path d="M21 8A9 9 0 1 0 18.36 18.36" />
  {:else if name === "exclamationmark.triangle"}
    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
    <line x1="12" y1="9" x2="12" y2="13" />
    <line x1="12" y1="17" x2="12.01" y2="17" />
  {/if}
</svg>

<style>
  .icon {
    display: inline-block;
    vertical-align: middle;
    flex-shrink: 0;
  }
</style>
