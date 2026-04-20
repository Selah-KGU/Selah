<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { kwicOpenLink } from "../api";

  interface IctTool {
    id: string;
    title: string;
    url?: string;
    host: string;
    systemBrowser?: boolean;
  }

  const tools: IctTool[] = [
    {
      id: "facility",
      title: "施設予約",
      host: "kwic.kwansei.ac.jp",
    },
    {
      id: "zoom",
      title: "Zoom",
      url: "https://sso.kwansei.ac.jp/app/kwansei_zoom_1/exk6xakpncQIXiZ2K697/sso/saml",
      host: "zoom.us",
    },
    {
      id: "box",
      title: "Box",
      url: "https://kwansei.box.com",
      host: "kwansei.box.com",
    },
    {
      id: "slack",
      title: "Slack",
      url: "https://kwansei.enterprise.slack.com",
      host: "kwansei.enterprise.slack.com",
      systemBrowser: true,
    },
    {
      id: "onedrive",
      title: "OneDrive",
      url: "https://kwanseio365-my.sharepoint.com/",
      host: "kwanseio365-my.sharepoint.com",
    },
    {
      id: "remote",
      title: "リモートPC",
      url: "https://remotegate.kwansei.ac.jp",
      host: "remotegate.kwansei.ac.jp",
    },
    {
      id: "alias",
      title: "別名アドレス・送信元アドレス設定",
      url: "https://webservice.kwansei.ac.jp/nickname/index",
      host: "webservice.kwansei.ac.jp",
    },
    {
      id: "migration",
      title: "データ移行（移行可能期間のみ使用可）",
      url: "https://webservice.kwansei.ac.jp/",
      host: "webservice.kwansei.ac.jp",
    },
  ];

  async function openTool(tool: IctTool) {
    try {
      if (tool.id === "facility") {
        await invoke("open_facility_reservation");
        return;
      }
      if (!tool.url) return;
      if (tool.systemBrowser) {
        await invoke("open_in_system_browser", { url: tool.url });
        return;
      }
      await kwicOpenLink(tool.url, tool.title);
    } catch (e) {
      console.error("Failed to open tool:", e);
    }
  }
</script>

<div class="view">
  <h2>ツール</h2>
  <div class="tool-grid">
    {#each tools as tool, i}
      <button class="tool-card" onclick={() => openTool(tool)} style={`animation: fade-in 0.28s ease ${i * 0.04}s both;`}>
        <div class="tool-icon-wrap">
          {#if tool.id === "migration"}
            <!-- Data migration / sync arrows -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/></svg>
          {:else if tool.id === "facility"}
            <!-- Facility reservation / building -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="6" width="8" height="16" rx="1"/><rect x="13" y="2" width="8" height="20" rx="1"/><line x1="6" y1="9" x2="8" y2="9"/><line x1="6" y1="12" x2="8" y2="12"/><line x1="6" y1="15" x2="8" y2="15"/><line x1="16" y1="5" x2="18" y2="5"/><line x1="16" y1="8" x2="18" y2="8"/><line x1="16" y1="11" x2="18" y2="11"/><line x1="16" y1="14" x2="18" y2="14"/><line x1="16" y1="17" x2="18" y2="17"/></svg>
          {:else if tool.id === "zoom"}
            <!-- Zoom video camera -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="14" height="14" rx="2"/><path d="M22 7l-6 4 6 4V7z" fill="currentColor"/></svg>
          {:else if tool.id === "box"}
            <!-- Box cloud storage -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/><polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/></svg>
          {:else if tool.id === "slack"}
            <!-- Slack hash / channels -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="9" x2="20" y2="9"/><line x1="4" y1="15" x2="20" y2="15"/><line x1="10" y1="3" x2="8" y2="21"/><line x1="16" y1="3" x2="14" y2="21"/></svg>
          {:else if tool.id === "onedrive"}
            <!-- OneDrive cloud -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/></svg>
          {:else if tool.id === "alias"}
            <!-- Email / at sign -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94"/></svg>
          {:else if tool.id === "remote"}
            <!-- Remote desktop / monitor -->
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
          {/if}
        </div>
        <div class="tool-main">
          <span class="tool-title">{tool.title}</span>
          <span class="tool-host">{tool.host}</span>
        </div>
        <span class="tool-arrow">›</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .view h2 {
    margin-bottom: 12px;
  }

  .tool-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 10px;
  }

  .tool-card {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    border: 0.5px solid var(--border);
    border-radius: 12px;
    background: var(--bg-card);
    box-shadow: var(--shadow-sm);
    text-align: left;
    padding: 10px 12px;
    transition: transform 0.16s ease, box-shadow 0.16s ease, border-color 0.16s ease;
  }

  .tool-card:hover {
    transform: translateY(-1px);
    box-shadow: var(--shadow-md);
    border-color: var(--border-strong);
  }

  .tool-icon-wrap {
    width: 40px;
    height: 40px;
    border-radius: 8px;
    background: var(--bg-secondary);
    display: grid;
    place-items: center;
    overflow: hidden;
    flex-shrink: 0;
    color: var(--text-secondary);
  }

  .tool-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tool-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.35;
  }

  .tool-host {
    font-size: 11px;
    color: var(--text-secondary);
  }

  .tool-arrow {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 16px;
    line-height: 1;
    flex-shrink: 0;
  }

  @media (max-width: 760px) {
    .tool-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
