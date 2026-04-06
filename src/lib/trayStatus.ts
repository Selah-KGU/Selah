/**
 * Tray status cycling: collects data from cache and sends cycling items to the backend tray.
 */
import { invoke } from "@tauri-apps/api/core";
import { onCacheUpdate, getCached } from "./stores";
import type { TimetableData } from "./stores";
import type { LunaTodoItem } from "./types";
import { PERIOD_TIMES, DAY_LABELS } from "./types";

// ============ State ============

let timetableData: TimetableData | null = null;
let todoItems: LunaTodoItem[] = [];

let unsubscribers: (() => void)[] = [];
let rebuildTimer: ReturnType<typeof setTimeout> | null = null;

// ============ Core Logic ============

function findNextSchoolDay(entries: { day: string; period: number; course_name: string; is_cancelled: boolean }[], fromDay: number): { dayLabel: string; dayOffset: number; classes: typeof entries } | null {
  for (let offset = 1; offset <= 7; offset++) {
    const idx = (fromDay + offset) % 7;
    const dayLabel = DAY_LABELS[idx];
    const classes = entries.filter(e => e.day === dayLabel && !e.is_cancelled).sort((a, b) => a.period - b.period);
    if (classes.length > 0) return { dayLabel, dayOffset: offset, classes };
  }
  return null;
}

function buildStatusItems(): string[] {
  const items: string[] = [];
  const now = new Date();
  const todayDay = DAY_LABELS[now.getDay()];
  const nowMin = now.getHours() * 60 + now.getMinutes();

  // 1. Today's classes
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

    const remaining = todayClasses.filter(e => {
      const pt = PERIOD_TIMES[e.period];
      return pt && nowMin < pt.endH * 60 + pt.endM;
    });
    if (remaining.length > 1) {
      items.push(`今日あと${remaining.length}コマ`);
    }

    // 2. Next school day preview (always show, not just when today has no classes)
    if (!foundNext) {
      const next = findNextSchoolDay(timetableData.entries, now.getDay());
      if (next) {
        const label = next.dayOffset === 1 ? "明日" : `${next.dayLabel}曜`;
        items.push(`${label} ${next.classes.length}コマ`);
        const first = next.classes[0];
        const pt = PERIOD_TIMES[first.period];
        if (pt) {
          items.push(`${label}${first.period}限 ${first.course_name} ${pt.start}~`);
        }
      }
    }
  }

  // 3. All pending todos (not just within 5 days)
  if (todoItems.length > 0) {
    const pending = todoItems
      .filter(t => !t.status.includes("提出済"))
      .sort((a, b) => {
        const da = a.deadline ? new Date(a.deadline.replace(/\//g, "-")).getTime() : Infinity;
        const db = b.deadline ? new Date(b.deadline.replace(/\//g, "-")).getTime() : Infinity;
        return da - db;
      });

    if (pending.length > 0) {
      // Show the nearest deadline
      const first = pending[0];
      if (first.deadline) {
        const dl = new Date(first.deadline.replace(/\//g, "-"));
        const diffDays = Math.ceil((dl.getTime() - now.getTime()) / (1000 * 60 * 60 * 24));
        const dlLabel = diffDays <= 0 ? "今日〆" : diffDays === 1 ? "明日〆" : `${diffDays}日後〆`;
        items.push(`${first.content_name} ${dlLabel}`);
      } else {
        items.push(`${first.content_name}`);
      }

      if (pending.length > 1) {
        items.push(`未提出 ${pending.length}件`);
      }
    }
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
  timetableData = getCached<TimetableData>("timetable");
  todoItems = getCached<LunaTodoItem[]>("luna_todo") ?? [];

  unsubscribers.push(
    onCacheUpdate<TimetableData>("timetable", (data) => { timetableData = data; scheduleRebuild(); }),
    onCacheUpdate<LunaTodoItem[]>("luna_todo", (data) => { todoItems = data ?? []; scheduleRebuild(); }),
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
