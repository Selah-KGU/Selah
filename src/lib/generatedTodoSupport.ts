export const LIVE_GENERATED_TODO_KEY = "live_generated_todo";
export const DETAIL_GENERATED_TODO_KEY = "detail_generated_todo";

type MessageWithId = {
  id?: unknown;
};

export interface GeneratedTodoLike {
  id?: string;
  title?: string;
  course_name?: string;
  content_type?: string;
  deadline?: string;
  note?: string;
  source_excerpt?: string;
  source_path?: string;
  source_url?: string;
  completed_at?: string;
  archived_at?: string;
  day?: number;
  period?: number;
}

export interface LunaTodoLike {
  course_name: string;
  content_type: string;
  content_name: string;
  url: string;
  deadline: string;
  status: string;
  feedback: string;
  source?: string;
  local_id?: string;
  source_path?: string;
  source_excerpt?: string;
}

function commonAsciiSuffixLength(left: string, right: string): number {
  let count = 0;
  let i = left.length - 1;
  let j = right.length - 1;
  while (i >= 0 && j >= 0 && left.charCodeAt(i) === right.charCodeAt(j)) {
    count += 1;
    i -= 1;
    j -= 1;
  }
  return count;
}

function hasMatchingIdPrefix(partial: string, full: string): boolean {
  const prefixLength = Math.min(partial.length, 24);
  return prefixLength >= 8 && full.startsWith(partial.slice(0, prefixLength));
}

export function repairMailSourceUrl(sourceUrl: unknown, messages: readonly MessageWithId[]): string {
  const trimmed = String(sourceUrl || "").trim();
  if (!trimmed.startsWith("mail://")) return trimmed;
  const id = trimmed.slice("mail://".length).trim();
  if (!id || messages.some((message) => String(message.id || "") === id)) return trimmed;

  let bestId = "";
  let bestScore = 0;
  let tied = false;
  for (const message of messages) {
    const candidate = String(message.id || "");
    if (!hasMatchingIdPrefix(id, candidate)) continue;
    const score = commonAsciiSuffixLength(id, candidate);
    if (score < 12) continue;
    if (score === bestScore) {
      tied = true;
    } else if (score > bestScore) {
      bestId = candidate;
      bestScore = score;
      tied = false;
    }
  }
  return bestId && !tied ? "mail://" + bestId : trimmed;
}

export function generatedTodoIdentityKey(item: {
  course_name?: string;
  title?: string;
  content_name?: string;
  deadline?: string;
}): string {
  return [item.course_name, item.title ?? item.content_name, item.deadline]
    .map((part) => String(part || "").trim().toLowerCase().replace(/\s+/g, " "))
    .join("|");
}

function isActiveGeneratedTodo(item: GeneratedTodoLike): boolean {
  return !item.completed_at && !item.archived_at && String(item.title || "").trim().length > 0;
}

function isGeneratedLunaTodoItem(item: unknown): boolean {
  if (!item || typeof item !== "object") return false;
  const row = item as { source?: string; url?: string; feedback?: string };
  return (
    row.source === "live" ||
    String(row.url || "").startsWith("live-generated://") ||
    String(row.feedback || "").startsWith("Liveから追加")
  );
}

function isDetailLunaTodoItem(item: unknown): boolean {
  if (!item || typeof item !== "object") return false;
  const row = item as { source?: string; url?: string };
  return row.source === "detail" || String(row.url || "").startsWith("detail-generated://");
}

function liveGeneratedTodoToLunaItem(item: GeneratedTodoLike): LunaTodoLike {
  return {
    course_name: item.course_name || "",
    content_type: item.content_type || "課題",
    content_name: item.title || "",
    url: "live-generated://" + encodeURIComponent(item.id || ""),
    deadline: item.deadline || "",
    status: "未提出",
    feedback: item.note ? "Liveから追加: " + item.note : "Liveから追加",
    source: "live",
    local_id: item.id || "",
    source_path: item.source_path || "",
    source_excerpt: item.source_excerpt || "",
  };
}

function detailGeneratedTodoToLunaItem(item: GeneratedTodoLike): LunaTodoLike {
  return {
    course_name: item.course_name || "",
    content_type: item.content_type || "課題",
    content_name: item.title || "",
    url: "detail-generated://" + encodeURIComponent(item.id || ""),
    deadline: item.deadline || "",
    status: "未提出",
    feedback: item.note ? "マグネット: " + item.note : "マグネットで追加",
    source: "detail",
    local_id: item.id || "",
    source_path: item.source_url || "",
    source_excerpt: item.source_excerpt || "",
  };
}

function mergeLocalTodosIntoLunaTodos(
  base: unknown,
  generated: readonly GeneratedTodoLike[],
  isLocal: (item: unknown) => boolean,
  toLuna: (item: GeneratedTodoLike) => LunaTodoLike,
): LunaTodoLike[] {
  const list = (Array.isArray(base) ? base : []).filter((item) => item && typeof item === "object" && !isLocal(item)) as LunaTodoLike[];
  const seen = new Set(list.map((item) => generatedTodoIdentityKey({
    course_name: item.course_name,
    title: item.content_name,
    deadline: item.deadline,
  })));
  const merged = [...list];
  for (const item of generated) {
    if (!isActiveGeneratedTodo(item)) continue;
    const key = generatedTodoIdentityKey(item);
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(toLuna(item));
  }
  return merged;
}

export function mergeGeneratedTodosIntoLunaTodos(
  base: unknown,
  generated: readonly GeneratedTodoLike[],
): LunaTodoLike[] {
  return mergeLocalTodosIntoLunaTodos(base, generated, isGeneratedLunaTodoItem, liveGeneratedTodoToLunaItem);
}

export function mergeDetailTodosIntoLunaTodos(
  base: unknown,
  generated: readonly GeneratedTodoLike[],
): LunaTodoLike[] {
  return mergeLocalTodosIntoLunaTodos(base, generated, isDetailLunaTodoItem, detailGeneratedTodoToLunaItem);
}

function assignmentLabelFromGeneratedTodo(item: GeneratedTodoLike): string {
  const type = item.content_type || "課題";
  const deadline = item.deadline ? " (締切: " + item.deadline + ")" : "";
  return "Live追加 " + type + ": " + item.title + deadline;
}

export function mergeGeneratedTodosIntoSchedule<T>(base: T, generated: readonly GeneratedTodoLike[]): T {
  const record = base as { ai_result?: { current_week?: unknown; next_week?: unknown } | null } | null;
  if (!record?.ai_result) return base;
  const cloned = JSON.parse(JSON.stringify(base)) as {
    ai_result?: { current_week?: unknown; next_week?: unknown } | null;
  };
  const activeGenerated = generated.filter(isActiveGeneratedTodo);
  const mergeWeek = (items: unknown) => {
    if (!Array.isArray(items)) return;
    for (const cell of items) {
      if (!cell || typeof cell !== "object") continue;
      const row = cell as { course_name?: string; day?: number; period?: number; assignments?: unknown[] };
      if (Array.isArray(row.assignments)) {
        row.assignments = row.assignments.filter((label) => !String(label).startsWith("Live追加 "));
      }
      for (const todo of activeGenerated) {
        const matchesCourse = Boolean(todo.course_name) && row.course_name === todo.course_name;
        const matchesSlot = Number(todo.day) > 0 && Number(todo.period) > 0 && row.day === todo.day && row.period === todo.period;
        if (!matchesCourse && !matchesSlot) continue;
        const label = assignmentLabelFromGeneratedTodo(todo);
        if (!Array.isArray(row.assignments)) row.assignments = [];
        if (!row.assignments.includes(label)) row.assignments.push(label);
      }
    }
  };
  mergeWeek(cloned.ai_result?.current_week);
  mergeWeek(cloned.ai_result?.next_week);
  return cloned as T;
}

export async function repairDetailGeneratedTodoSourceUrls<T extends { source_url?: string }>(
  items: readonly T[],
  readCache: (key: string) => Promise<string | null>,
  writeCache: (key: string, json: string) => Promise<void>,
): Promise<T[]> {
  if (!items.some((item) => String(item?.source_url || "").startsWith("mail://"))) return [...items];
  const inboxJson = await readCache("mail_inbox");
  if (!inboxJson) return [...items];
  let messages: MessageWithId[] = [];
  try {
    const parsed = JSON.parse(inboxJson);
    if (Array.isArray(parsed)) messages = parsed;
  } catch {
    return [...items];
  }
  if (messages.length === 0) return [...items];

  let changed = false;
  const repaired = items.map((item) => {
    const nextSourceUrl = repairMailSourceUrl(item?.source_url || "", messages);
    if (nextSourceUrl === (item?.source_url || "")) return item;
    changed = true;
    return { ...item, source_url: nextSourceUrl };
  });
  if (changed) await writeCache(DETAIL_GENERATED_TODO_KEY, JSON.stringify(repaired));
  return repaired;
}
