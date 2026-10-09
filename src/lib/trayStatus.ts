/**
 * Tray status cycling: collects data from cache and sends cycling items to the backend tray.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ResourceScope } from "./resourceScope";
import { LatestViewRead } from "./latestViewRead";
import { TrayStatusRefresh, TrayStatusWriter } from "./trayStatusRefresh";
import { onCacheUpdate, getCached, registerTask, updateTask } from "./stores";
import type { LunaTodoItem, ScheduleResponse, KgcCourseRow } from "./types";
import { PERIOD_TIMES, DAY_LABELS, DAY_NUM_LABELS } from "./types";

// ============ State ============

let timetableData: ScheduleResponse | null = null;
let todoItems: LunaTodoItem[] = [];
let activeRun: ResourceScope | null = null;
const statusWriter = new TrayStatusWriter(items => invoke("set_tray_status_items", { items }));

interface LiveTrayStatus {
  active: boolean;
  listening: boolean;
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

type DeadlineInfo = {
  dueMs: number;
  dayStartMs: number;
};

function parseDeadlineInfo(deadline: string): DeadlineInfo | null {
  const text = normalizeText(deadline);
  if (!text) return null;

  const dateMatch = text.match(/(\d{4})[\/\-年](\d{1,2})[\/\-月](\d{1,2})/);
  const timeMatch = text.match(/(\d{1,2}):(\d{2})/);
  if (dateMatch) {
    const year = Number(dateMatch[1]);
    const month = Number(dateMatch[2]) - 1;
    const day = Number(dateMatch[3]);
    const hasTime = !!timeMatch;
    const hour = hasTime ? Number(timeMatch![1]) : 23;
    const minute = hasTime ? Number(timeMatch![2]) : 59;
    const due = new Date(year, month, day, hour, minute, hasTime ? 0 : 59, hasTime ? 0 : 999);
    const dayStart = new Date(year, month, day);
    if (Number.isFinite(due.getTime())) {
      return { dueMs: due.getTime(), dayStartMs: dayStart.getTime() };
    }
  }

  const normalized = text
    .replace(/\//g, "-")
    .replace(/年|月/g, "-")
    .replace(/日/g, "");
  const parsed = new Date(normalized);
  const dueMs = parsed.getTime();
  if (!Number.isFinite(dueMs)) return null;
  const dayStart = new Date(parsed.getFullYear(), parsed.getMonth(), parsed.getDate()).getTime();
  return { dueMs, dayStartMs: dayStart };
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

function buildLiveStatusItem(nowMs: number, live: LiveTrayStatus): string | null {
  if (!live.active) return null;
  const elapsed = formatElapsedMinutes(parseLiveStartedAt(live.started_at), nowMs);
  const suffix = elapsed ? ` ${elapsed}` : "";
  return live.listening ? `Live記録中${suffix}` : `Live一時停止${suffix}`;
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

const COMPLETED_TODO_STATUS = /提出済|提出済み|完了|採点済|受験済/;
const DAY_MS = 86_400_000;

type TrayTodoItem = {
  todo: LunaTodoItem;
  title: string;
  deadline: DeadlineInfo;
};

function hasSpecificTodoTitle(title: string): boolean {
  const normalized = normalizeText(title);
  if (normalized.length < 2) return false;
  if (GENERIC_TODO_TITLES.has(normalized)) return false;
  if (/^(本日|今日|明日)?\s*[〆締]切$/.test(normalized)) return false;
  return /[\p{L}\p{N}]/u.test(normalized);
}

function buildTodoDisplayTitle(todo: LunaTodoItem): string {
  const contentName = normalizeText(todo.content_name);
  const courseName = normalizeText(todo.course_name);
  const contentType = normalizeText(todo.content_type);

  if (hasSpecificTodoTitle(contentName)) {
    if (contentType && !contentName.includes(contentType)) {
      return `${contentType}: ${contentName}`;
    }
    return contentName;
  }

  if (hasSpecificTodoTitle(courseName) && contentType) {
    return `${courseName} ${contentType}`;
  }
  if (hasSpecificTodoTitle(courseName)) return courseName;
  return "";
}

function deadlineLabel(info: DeadlineInfo, now: Date): string {
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const days = Math.max(0, Math.round((info.dayStartMs - todayStart) / DAY_MS));
  if (days === 0) return "本日締切";
  if (days === 1) return "明日締切";
  return `あと${days}日`;
}

function buildActiveUpcomingTodos(now: Date): TrayTodoItem[] {
  return todoItems
    .map((todo) => ({ todo, title: buildTodoDisplayTitle(todo), deadline: parseDeadlineInfo(todo.deadline) }))
    .filter((item): item is TrayTodoItem => {
      if (!item.deadline) return false;
      if (COMPLETED_TODO_STATUS.test(item.todo.status)) return false;
      return item.deadline.dueMs >= now.getTime();
    })
    .sort((a, b) => a.deadline.dueMs - b.deadline.dueMs);
}

function buildStatusItems(live: LiveTrayStatus): string[] {
  const items: string[] = [];
  const now = new Date();
  const liveItem = buildLiveStatusItem(now.getTime(), live);
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
    const activeUpcomingTodos = buildActiveUpcomingTodos(now);

    const firstReadableDeadline = activeUpcomingTodos.find(({ title }) => !!title);
    if (firstReadableDeadline) {
      items.push(`${truncate(firstReadableDeadline.title, 22)} ${deadlineLabel(firstReadableDeadline.deadline, now)}`);
    }

    if (activeUpcomingTodos.length > 1 || (activeUpcomingTodos.length === 1 && !firstReadableDeadline)) {
      items.push(`未提出課題 ${activeUpcomingTodos.length}件`);
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

// ============ Public API ============

export function startTrayStatus() {
  if (activeRun) return;
  const scope = new ResourceScope();
  activeRun = scope;
  registerTask("tray_status", "トレイ表示更新", "system", 90_000);
  timetableData = getCached<ScheduleResponse>("schedule_data");
  todoItems = getCached<LunaTodoItem[]>("luna_todo") ?? [];

  const refresh = new TrayStatusRefresh(scope, {
    read: () => invoke<LiveTrayStatus>("live_get_tray_status"),
    build: buildStatusItems,
    writer: statusWriter,
    activity: patch => updateTask("tray_status", patch),
    failed: error => console.warn("[Selah] tray status refresh failed:", error),
  });
  const scheduleRead = new LatestViewRead(scope,
    () => invoke<ScheduleResponse>("get_schedule_snapshot"),
    data => { timetableData = data; refresh.request(); });

  scope.own(onCacheUpdate<ScheduleResponse>("schedule_data", scope.guard((data) => {
    scheduleRead.invalidate();
    timetableData = data;
    refresh.request();
  })));
  scope.own(onCacheUpdate<LunaTodoItem[]>("luna_todo", scope.guard((data) => {
    todoItems = data ?? [];
    refresh.request();
  })));

  function subscribe<T>(name: string, handler: (payload: T) => void) {
    // Both the pending registration and queued callbacks belong to this run;
    // a restart must not adopt the old run's unlisten handle or callbacks.
    void scope.acquire(() => listen<T>(name, scope.guard(event => handler(event.payload)))).catch(error => {
      if (scope.active) console.warn("[Selah] tray subscription failed:", error);
    });
  }
  subscribe("live-session-updated", () => refresh.request());
  subscribe<{ caller: string }>("stt-state", payload => {
    if (payload.caller === "live") refresh.request();
  });
  subscribe("live-surface-saved", () => refresh.request());
  subscribe("live-surface-compact-saved", () => refresh.request());

  // Managed caches normally supply the initial timetable. Only recover from
  // the backend when it is absent, and never load a real schedule in demo mode.
  let demo = false;
  try { demo = localStorage.getItem("selah-demo-mode") === "1"; } catch {}
  if (!timetableData && !demo) void scheduleRead.refresh().catch(error => {
    if (scope.active) console.warn("[Selah] tray timetable recovery failed:", error);
  });

  const interval = setInterval(() => refresh.request(), 90_000);
  scope.own(() => clearInterval(interval));
  refresh.request();
}

export function stopTrayStatus() {
  const scope = activeRun;
  activeRun = null;
  scope?.dispose();
  timetableData = null;
  todoItems = [];
  updateTask("tray_status", { running: false });
  // Clear after any already-issued write. An immediate restart can supersede
  // this queued clear; its first current update will then publish in order.
  void statusWriter.write([], () => activeRun === null).catch(error => {
    if (!activeRun) console.warn("[Selah] tray status clear failed:", error);
  });
}
