/**
 * Tray status cycling: collects data from cache and sends cycling items to the backend tray.
 */
import { invoke } from "@tauri-apps/api/core";
import { onCacheUpdate, getCached } from "./stores";
import type { LunaTodoItem, ScheduleResponse, KgcCourseRow } from "./types";
import { PERIOD_TIMES, DAY_LABELS, DAY_NUM_LABELS } from "./types";

// ============ State ============

let timetableData: ScheduleResponse | null = null;
let todoItems: LunaTodoItem[] = [];

let unsubscribers: (() => void)[] = [];
let rebuildTimer: ReturnType<typeof setTimeout> | null = null;

// ============ Core Logic ============

/** Convert JS day-of-week (0=Sun..6=Sat) to unified day (1=Mon..7=Sun) */
function jsToUnifiedDay(jsDow: number): number {
  return jsDow === 0 ? 7 : jsDow;
}

function findNextSchoolDay(entries: KgcCourseRow[], fromJsDow: number): { dayLabel: string; dayOffset: number; classes: KgcCourseRow[] } | null {
  for (let offset = 1; offset <= 7; offset++) {
    const jsDow = (fromJsDow + offset) % 7;
    const unifiedDay = jsToUnifiedDay(jsDow);
    const classes = entries.filter(e => e.day === unifiedDay && !e.is_cancelled).sort((a, b) => a.period - b.period);
    if (classes.length > 0) return { dayLabel: DAY_NUM_LABELS[unifiedDay] ?? DAY_LABELS[jsDow], dayOffset: offset, classes };
  }
  return null;
}

/** Truncate text to maxLen, appending ellipsis if needed */
function truncate(s: string, maxLen: number): string {
  return s.length > maxLen ? s.slice(0, maxLen - 1) + "..." : s;
}

function buildStatusItems(): string[] {
  const items: string[] = [];
  const now = new Date();
  const todayDay = jsToUnifiedDay(now.getDay());
  const nowMin = now.getHours() * 60 + now.getMinutes();

  let todayHasRemaining = false;
  let currentOrNext: string | null = null;

  // 1. Today's classes
  if (timetableData?.raw.kgc_entries_current.length) {
    const todayClasses = timetableData.raw.kgc_entries_current
      .filter(e => e.day === todayDay && !e.is_cancelled)
      .sort((a, b) => a.period - b.period);

    const remaining = todayClasses.filter(e => {
      const pt = PERIOD_TIMES[e.period];
      return pt && nowMin < pt.endH * 60 + pt.endM;
    });
    todayHasRemaining = remaining.length > 0;

    for (const entry of todayClasses) {
      const pt = PERIOD_TIMES[entry.period];
      if (!pt) continue;
      const endMin = pt.endH * 60 + pt.endM;
      const startMin = pt.startH * 60 + pt.startM;
      if (nowMin < endMin) {
        const name = truncate(entry.name, 16);
        const isLive = nowMin >= startMin;
        if (isLive) {
          const left = endMin - nowMin;
          currentOrNext = `${entry.period}限 ${name} (残${left}分)`;
        } else {
          const diff = startMin - nowMin;
          if (diff <= 60) {
            currentOrNext = `${diff}分後 ${entry.period}限 ${name}`;
          } else {
            currentOrNext = `次: ${entry.period}限 ${name} ${pt.start}~`;
          }
        }
        break;
      }
    }

    if (currentOrNext) items.push(currentOrNext);

    if (remaining.length > 1) {
      items.push(`今日あと${remaining.length}コマ`);
    } else if (remaining.length === 1 && !currentOrNext) {
      // last class already counted above
    }

    // Next school day preview (when all today's classes are done, or always as secondary info)
    if (!todayHasRemaining) {
      const next = findNextSchoolDay(timetableData.raw.kgc_entries_current, now.getDay());
      if (next) {
        const label = next.dayOffset === 1 ? "明日" : `${next.dayLabel}曜`;
        const first = next.classes[0];
        const pt = PERIOD_TIMES[first.period];
        items.push(`${label} ${next.classes.length}コマ`);
        if (pt) {
          items.push(`${label}${first.period}限 ${truncate(first.name, 14)} ${pt.start}~`);
        }
      }
    }
  }

  // 2. Pending TODOs
  if (todoItems.length > 0) {
    const pending = todoItems
      .filter(t => !t.status.includes("提出済"))
      .sort((a, b) => {
        const da = a.deadline ? new Date(a.deadline.replace(/\//g, "-")).getTime() : Infinity;
        const db = b.deadline ? new Date(b.deadline.replace(/\//g, "-")).getTime() : Infinity;
        return da - db;
      });

    if (pending.length > 0) {
      const first = pending[0];
      if (first.deadline) {
        const dl = new Date(first.deadline.replace(/\//g, "-"));
        const diffMs = dl.getTime() - now.getTime();
        const diffDays = Math.ceil(diffMs / (1000 * 60 * 60 * 24));
        const dlLabel = diffDays <= 0 ? "今日〆切" : diffDays === 1 ? "明日〆切" : `あと${diffDays}日`;
        items.push(`${truncate(first.content_name, 14)} ${dlLabel}`);
      }

      if (pending.length > 1) {
        items.push(`未提出課題 ${pending.length}件`);
      }
    }
  }

  // 3. Fallback: show a calm message so tray is never empty
  if (items.length === 0) {
    const h = now.getHours();
    if (h < 5) items.push("おやすみなさい");
    else if (h < 12) items.push("おはようございます");
    else if (h < 18) items.push("予定なし");
    else items.push("お疲れさまでした");
  }

  return items;
}

function scheduleRebuild() {
  if (rebuildTimer) clearTimeout(rebuildTimer);
  rebuildTimer = setTimeout(() => {
    const items = buildStatusItems();
    invoke("set_tray_status_items", { items }).catch(() => {});
  }, 300);
}

// ============ Public API ============

export function startTrayStatus() {
  // Bootstrap from existing cache (disk or memory)
  timetableData = getCached<ScheduleResponse>("schedule_data");
  todoItems = getCached<LunaTodoItem[]>("luna_todo") ?? [];

  unsubscribers.push(
    onCacheUpdate<ScheduleResponse>("schedule_data", (data) => { timetableData = data; scheduleRebuild(); }),
    onCacheUpdate<LunaTodoItem[]>("luna_todo", (data) => { todoItems = data ?? []; scheduleRebuild(); }),
  );

  // Rebuild periodically (every 90s) to update time-sensitive items like "current class"
  const interval = setInterval(() => scheduleRebuild(), 90_000);
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
