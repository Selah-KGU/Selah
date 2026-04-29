/**
 * Tray status cycling: collects data from cache and sends cycling items to the backend tray.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { onCacheUpdate, getCached, registerTask, updateTask } from "./stores";
import type { LunaTodoItem, ScheduleResponse, KgcCourseRow } from "./types";
import { PERIOD_TIMES, DAY_LABELS, DAY_NUM_LABELS } from "./types";

// ============ State ============

let timetableData: ScheduleResponse | null = null;
let todoItems: LunaTodoItem[] = [];
let liveTrayStatus: { active: boolean; listening: boolean; startedAtMs: number | null } | null = null;

let unsubscribers: (() => void)[] = [];
let rebuildTimer: ReturnType<typeof setTimeout> | null = null;
let trayStatusStarted = false;

interface LiveSessionSnapshot {
  active: boolean;
  started_at: string | null;
}

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

function normalizeText(s: string | null | undefined): string {
  return (s ?? "").replace(/\s+/g, " ").trim();
}

function parseDeadlineTime(deadline: string): number {
  const normalized = normalizeText(deadline)
    .replace(/\//g, "-")
    .replace(/年|月/g, "-")
    .replace(/日/g, "");
  const ts = new Date(normalized).getTime();
  return Number.isFinite(ts) ? ts : Infinity;
}

function parseLiveStartedAt(value: string | null | undefined): number | null {
  if (!value) return null;
  const ts = new Date(value.replace(" ", "T")).getTime();
  return Number.isFinite(ts) ? ts : null;
}

function formatElapsedMinutes(startedAtMs: number | null, nowMs: number): string {
  if (!startedAtMs) return "";
  const minutes = Math.max(0, Math.floor((nowMs - startedAtMs) / 60_000));
  if (minutes < 60) return `${minutes}分`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return `${hours}時間${String(rest).padStart(2, "0")}分`;
}

function buildLiveStatusItem(nowMs: number): string | null {
  if (!liveTrayStatus?.active) return null;
  const elapsed = formatElapsedMinutes(liveTrayStatus.startedAtMs, nowMs);
  const suffix = elapsed ? ` ${elapsed}` : "";
  return liveTrayStatus.listening ? `Live記録中${suffix}` : `Live一時停止${suffix}`;
}

const GENERIC_TODO_TITLES = new Set([
  "課題",
  "レポート",
  "テスト",
  "小テスト",
  "掲示板",
  "アンケート",
  "出席",
  "その他",
  "未設定",
]);

function hasSpecificTodoTitle(title: string): boolean {
  const normalized = normalizeText(title);
  if (normalized.length < 2) return false;
  if (GENERIC_TODO_TITLES.has(normalized)) return false;
  if (/^(本日|今日|明日)?\s*[〆締]切$/.test(normalized)) return false;
  return /[\p{L}\p{N}]/u.test(normalized);
}

function buildTodoDisplayTitle(todo: LunaTodoItem): string {
  const contentName = normalizeText(todo.content_name);
  if (hasSpecificTodoTitle(contentName)) return contentName;

  const courseName = normalizeText(todo.course_name);
  const contentType = normalizeText(todo.content_type);
  if (hasSpecificTodoTitle(courseName) && contentType) {
    return `${courseName} ${contentType}`;
  }
  if (hasSpecificTodoTitle(courseName)) return courseName;
  return "";
}

function buildStatusItems(): string[] {
  const items: string[] = [];
  const now = new Date();
  const liveItem = buildLiveStatusItem(now.getTime());
  if (liveItem) items.push(liveItem);

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
        const da = a.deadline ? parseDeadlineTime(a.deadline) : Infinity;
        const db = b.deadline ? parseDeadlineTime(b.deadline) : Infinity;
        return da - db;
      });

    if (pending.length > 0) {
      const firstReadableDeadline = pending.find(t => {
        const title = buildTodoDisplayTitle(t);
        return title && Number.isFinite(parseDeadlineTime(t.deadline));
      });
      if (firstReadableDeadline) {
        const dlTime = parseDeadlineTime(firstReadableDeadline.deadline);
        const diffMs = dlTime - now.getTime();
        const diffDays = Math.ceil(diffMs / (1000 * 60 * 60 * 24));
        const dlLabel = diffDays <= 0 ? "本日締切" : diffDays === 1 ? "明日締切" : `あと${diffDays}日`;
        items.push(`${truncate(buildTodoDisplayTitle(firstReadableDeadline), 14)} ${dlLabel}`);
      }

      if (pending.length > 1 || !firstReadableDeadline) {
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

async function refreshLiveTrayStatus() {
  try {
    const [snapshot, running, caller] = await Promise.all([
      invoke<LiveSessionSnapshot>("live_get_session"),
      invoke<boolean>("stt_is_running"),
      invoke<string | null>("stt_get_active_caller"),
    ]);
    liveTrayStatus = snapshot.active
      ? {
          active: true,
          listening: running && caller === "live",
          startedAtMs: parseLiveStartedAt(snapshot.started_at),
        }
      : null;
  } catch {
    liveTrayStatus = null;
  }
}

function scheduleRebuild() {
  if (rebuildTimer) clearTimeout(rebuildTimer);
  rebuildTimer = setTimeout(async () => {
    if (!trayStatusStarted) return;
    updateTask("tray_status", { running: true });
    await refreshLiveTrayStatus();
    if (!trayStatusStarted) return;
    const items = buildStatusItems();
    invoke("set_tray_status_items", { items }).then(
      () => updateTask("tray_status", { running: false, lastRunTs: Date.now(), lastOk: true }),
      () => updateTask("tray_status", { running: false, lastRunTs: Date.now(), lastOk: false }),
    );
  }, 300);
}

function addEventListener<T>(event: string, handler: (payload: T) => void) {
  listen<T>(event, (ev) => handler(ev.payload))
    .then((unlisten: UnlistenFn) => {
      if (trayStatusStarted) {
        unsubscribers.push(unlisten);
      } else {
        unlisten();
      }
    })
    .catch(() => {});
}

// ============ Public API ============

export function startTrayStatus() {
  if (trayStatusStarted) return;
  trayStatusStarted = true;
  registerTask("tray_status", "トレイ表示更新", "system", 90_000);
  // Bootstrap from existing cache (disk or memory)
  timetableData = getCached<ScheduleResponse>("schedule_data");
  todoItems = getCached<LunaTodoItem[]>("luna_todo") ?? [];

  unsubscribers.push(
    onCacheUpdate<ScheduleResponse>("schedule_data", (data) => { timetableData = data; scheduleRebuild(); }),
    onCacheUpdate<LunaTodoItem[]>("luna_todo", (data) => { todoItems = data ?? []; scheduleRebuild(); }),
  );

  addEventListener<LiveSessionSnapshot>("live-session-updated", () => scheduleRebuild());
  addEventListener<{ state: string; caller: string }>("stt-state", (payload) => {
    if (payload.caller === "live") scheduleRebuild();
  });
  addEventListener("live-session-saved", () => scheduleRebuild());

  // Rebuild periodically (every 90s) to update time-sensitive items like "current class"
  const interval = setInterval(() => scheduleRebuild(), 90_000);
  unsubscribers.push(() => clearInterval(interval));

  // Initial build from whatever is already cached
  scheduleRebuild();
}

export function stopTrayStatus() {
  trayStatusStarted = false;
  liveTrayStatus = null;
  for (const unsub of unsubscribers) unsub();
  unsubscribers = [];
  if (rebuildTimer) {
    clearTimeout(rebuildTimer);
    rebuildTimer = null;
  }
  invoke("set_tray_status_items", { items: [] }).catch(() => {});
}
