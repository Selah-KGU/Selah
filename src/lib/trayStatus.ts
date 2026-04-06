/**
 * Tray status cycling: collects data from cache and sends cycling items to the backend tray.
 */
import { invoke } from "@tauri-apps/api/core";
import { onCacheUpdate, cachedFetch } from "./stores";
import type { TimetableData, TimetableEntry, NotificationsData } from "./stores";
import type { KwicPortalHome } from "./api";

// ============ Types (local) ============

interface LunaTodoItem {
  course_name: string;
  content_type: string;
  content_name: string;
  url: string;
  deadline: string;
  status: string;
  feedback: string;
}

interface LunaNotification {
  date: string;
  course_info: string;
  module: string;
  content: string;
  url: string;
  idnumber: string;
}

// ============ Period Times ============

const PERIOD_TIMES: Record<number, { start: string; end: string; startH: number; startM: number; endH: number; endM: number }> = {
  1: { start: "9:00",  end: "10:30", startH: 9, startM: 0,  endH: 10, endM: 30 },
  2: { start: "11:00", end: "12:30", startH: 11, startM: 0,  endH: 12, endM: 30 },
  3: { start: "13:30", end: "15:00", startH: 13, startM: 30, endH: 15, endM: 0 },
  4: { start: "15:10", end: "16:40", startH: 15, startM: 10, endH: 16, endM: 40 },
  5: { start: "16:50", end: "18:20", startH: 16, startM: 50, endH: 18, endM: 20 },
};

const DAY_LABELS = ["日", "月", "火", "水", "木", "金", "土"];

// ============ State ============

let timetableData: TimetableData | null = null;
let todoItems: LunaTodoItem[] = [];
let kgcNotifs: NotificationsData | null = null;
let lunaNotifs: LunaNotification[] = [];
let kwicHome: KwicPortalHome | null = null;

let unsubscribers: (() => void)[] = [];
let rebuildTimer: ReturnType<typeof setTimeout> | null = null;

// ============ Core Logic ============

function buildStatusItems(): string[] {
  const items: string[] = [];
  const now = new Date();
  const todayDay = DAY_LABELS[now.getDay()];
  const nowMin = now.getHours() * 60 + now.getMinutes();

  // 1. Next/current class
  if (timetableData?.entries.length) {
    const todayClasses = timetableData.entries
      .filter(e => e.day === todayDay && !e.is_cancelled)
      .sort((a, b) => a.period - b.period);

    let foundNext = false;
    for (const entry of todayClasses) {
      const pt = PERIOD_TIMES[entry.period];
      if (!pt) continue;
      const endMin = pt.endH * 60 + pt.endM;
      const startMin = pt.startH * 60 + pt.startM;
      if (nowMin < endMin) {
        const isLive = nowMin >= startMin;
        if (isLive) {
          items.push(`${entry.period}限 ${entry.course_name} ~${pt.end}`);
        } else {
          items.push(`次: ${entry.period}限 ${entry.course_name} ${pt.start}~`);
        }
        foundNext = true;
        break;
      }
    }

    // Today's remaining count
    const remaining = todayClasses.filter(e => {
      const pt = PERIOD_TIMES[e.period];
      return pt && nowMin < pt.endH * 60 + pt.endM;
    });
    if (remaining.length > 1) {
      items.push(`今日あと${remaining.length}コマ`);
    } else if (!foundNext) {
      // No more classes today - show tomorrow preview
      const tomorrowIdx = (now.getDay() + 1) % 7;
      const tomorrowDay = DAY_LABELS[tomorrowIdx];
      const tomorrowClasses = timetableData.entries
        .filter(e => e.day === tomorrowDay && !e.is_cancelled);
      if (tomorrowClasses.length > 0) {
        items.push(`明日 ${tomorrowClasses.length}コマ`);
        const first = tomorrowClasses.sort((a, b) => a.period - b.period)[0];
        const pt = PERIOD_TIMES[first.period];
        if (pt) {
          items.push(`明日${first.period}限 ${first.course_name} ${pt.start}~`);
        }
      }
    }
  }

  // 2. Urgent todos (within 5 days, not submitted)
  if (todoItems.length > 0) {
    const limit = new Date(now);
    limit.setDate(limit.getDate() + 5);
    const urgent = todoItems
      .filter(t => {
        if (t.status.includes("提出済")) return false;
        if (!t.deadline) return false;
        const d = new Date(t.deadline.replace(/\//g, "-"));
        return d >= now && d <= limit;
      })
      .sort((a, b) => {
        const da = new Date(a.deadline.replace(/\//g, "-")).getTime();
        const db = new Date(b.deadline.replace(/\//g, "-")).getTime();
        return da - db;
      });

    if (urgent.length > 0) {
      // Show the most urgent one
      const first = urgent[0];
      const dl = new Date(first.deadline.replace(/\//g, "-"));
      const diffDays = Math.ceil((dl.getTime() - now.getTime()) / (1000 * 60 * 60 * 24));
      const dlLabel = diffDays <= 0 ? "今日〆" : diffDays === 1 ? "明日〆" : `${diffDays}日後〆`;
      items.push(`${first.content_name} ${dlLabel}`);

      if (urgent.length > 1) {
        items.push(`課題${urgent.length}件 未提出`);
      }
    }
  }

  // 3. Notification counts
  const kgcCount = kgcNotifs?.entries?.length ?? 0;
  const lunaCount = lunaNotifs.length;
  const kwicCount = kwicHome?.sections?.reduce((sum, s) => sum + s.items.length, 0) ?? 0;
  const totalNotifs = kgcCount + lunaCount + kwicCount;
  if (totalNotifs > 0) {
    const parts: string[] = [];
    if (kgcCount > 0) parts.push(`KGC ${kgcCount}`);
    if (lunaCount > 0) parts.push(`Luna ${lunaCount}`);
    if (kwicCount > 0) parts.push(`KWIC ${kwicCount}`);
    items.push(`通知: ${parts.join(" / ")}`);
  }

  return items;
}

function scheduleRebuild() {
  if (rebuildTimer) clearTimeout(rebuildTimer);
  rebuildTimer = setTimeout(() => {
    const items = buildStatusItems();
    invoke("set_tray_status_items", { items }).catch((e) => {
      console.warn("[TrayStatus] failed to send items:", e);
    });
  }, 300);
}

// ============ Public API ============

export function startTrayStatus() {
  unsubscribers.push(
    onCacheUpdate<TimetableData>("timetable", (data) => { timetableData = data; scheduleRebuild(); }),
    onCacheUpdate<LunaTodoItem[]>("luna_todo", (data) => { todoItems = data ?? []; scheduleRebuild(); }),
    onCacheUpdate<NotificationsData>("notifications", (data) => { kgcNotifs = data; scheduleRebuild(); }),
    onCacheUpdate<LunaNotification[]>("luna_updates", (data) => { lunaNotifs = data ?? []; scheduleRebuild(); }),
    onCacheUpdate<KwicPortalHome>("kwic_home", (data) => { kwicHome = data ?? null; scheduleRebuild(); }),
  );

  // Also rebuild periodically (every 60s) to update time-sensitive items like "current class"
  const interval = setInterval(() => scheduleRebuild(), 60_000);
  unsubscribers.push(() => clearInterval(interval));

  // Initial build from whatever is already cached
  scheduleRebuild();
}

export function stopTrayStatus() {
  for (const unsub of unsubscribers) unsub();
  unsubscribers = [];
  if (rebuildTimer) {
    clearTimeout(rebuildTimer);
    rebuildTimer = null;
  }
  invoke("set_tray_status_items", { items: [] }).catch(() => {});
}
