import { invoke } from "@tauri-apps/api/core";

/** Send a native notification */
export async function nativeNotify(title: string, body?: string) {
  await invoke("test_notification", { title, body: body ?? "" });
}

interface NotificationConfig {
  notify_important: boolean;
  notify_faculty: boolean;
  notify_class: boolean;
  notify_class_general: boolean;
  notify_class_announcement: boolean;
  notify_class_assignment: boolean;
  notify_class_exam: boolean;
  notify_class_discussion: boolean;
  notify_class_survey: boolean;
  notify_class_attendance: boolean;
  notify_other: boolean;
  notify_mail: boolean;
}

const DEFAULT_NOTIF_CONFIG: NotificationConfig = {
  notify_important: true,
  notify_faculty: true,
  notify_class: true,
  notify_class_general: true,
  notify_class_announcement: true,
  notify_class_assignment: true,
  notify_class_exam: true,
  notify_class_discussion: true,
  notify_class_survey: true,
  notify_class_attendance: true,
  notify_other: true,
  notify_mail: true,
};

async function getNotifConfig(): Promise<NotificationConfig> {
  try {
    const cfg = await invoke<Partial<NotificationConfig>>("get_notification_config");
    return { ...DEFAULT_NOTIF_CONFIG, ...cfg };
  } catch {
    return { ...DEFAULT_NOTIF_CONFIG };
  }
}

type CourseNotificationKind =
  | "general"
  | "announcement"
  | "assignment"
  | "exam"
  | "discussion"
  | "survey"
  | "attendance";

function classifyCourseNotification(module: string): CourseNotificationKind {
  const normalized = module.trim().toLowerCase();
  if (!normalized) return "general";
  if (
    normalized.includes("掲示板")
    || normalized.includes("forum")
    || normalized.includes("discussion")
    || normalized.includes("comment")
    || normalized.includes("返信")
  ) {
    return "discussion";
  }
  if (
    normalized.includes("アンケート")
    || normalized.includes("survey")
    || normalized.includes("questionnaire")
  ) {
    return "survey";
  }
  if (normalized.includes("出席") || normalized.includes("attendance")) {
    return "attendance";
  }
  if (
    normalized.includes("小テスト")
    || normalized.includes("テスト")
    || normalized.includes("試験")
    || normalized.includes("exam")
    || normalized.includes("quiz")
  ) {
    return "exam";
  }
  if (
    normalized.includes("課題")
    || normalized.includes("レポート")
    || normalized.includes("assignment")
    || normalized.includes("report")
    || normalized.includes("提出")
  ) {
    return "assignment";
  }
  if (
    normalized.includes("お知らせ")
    || normalized.includes("資料")
    || normalized.includes("announcement")
    || normalized.includes("material")
    || normalized.includes("連絡")
  ) {
    return "announcement";
  }
  return "general";
}

function courseNotificationAllowed(kind: CourseNotificationKind, cfg: NotificationConfig): boolean {
  if (!cfg.notify_class) return false;
  if (kind === "announcement") return cfg.notify_class_announcement;
  if (kind === "assignment") return cfg.notify_class_assignment;
  if (kind === "exam") return cfg.notify_class_exam;
  if (kind === "discussion") return cfg.notify_class_discussion;
  if (kind === "survey") return cfg.notify_class_survey;
  if (kind === "attendance") return cfg.notify_class_attendance;
  return cfg.notify_class_general;
}

// Per-source mutex to prevent concurrent notify calls from sending duplicate pushes
const locks = new Map<string, Promise<void>>();
function withLock(source: string, fn: () => Promise<void>): Promise<void> {
  const prev = locks.get(source) ?? Promise.resolve();
  const next = prev.then(fn, fn);
  locks.set(source, next);
  return next;
}

async function getSeenIds(source: string): Promise<Set<string>> {
  try {
    const ids: string[] = await invoke("get_seen_notif_ids", { source });
    return new Set(ids);
  } catch {
    return new Set();
  }
}

async function saveSeenIds(source: string, ids: Set<string>) {
  try {
    const arr = [...ids].slice(-500);
    await invoke("save_seen_notif_ids", { source, ids: arr });
  } catch { /* ignore */ }
}

async function ensurePermission(): Promise<boolean> {
  try {
    const granted: boolean = await invoke("plugin:notification|is_permission_granted");
    if (!granted) {
      // On macOS, the OS will prompt for permission on first notification.
      // We can't programmatically request it via WKWebView, so just return true
      // and let the OS handle the prompt.
      return true;
    }
    return granted;
  } catch {
    return true; // Fallback: try sending anyway
  }
}

export interface KgcNotif {
  id: string;
  title: string;
  date: string;
  category: string;
}

export interface LunaNotif {
  date: string;
  course_info: string;
  module: string;
  content: string;
}

/** Check KG-Course notifications for new items and send native notifications */
export function notifyNewKgc(entries: KgcNotif[]): Promise<void> {
  if (!entries.length) return Promise.resolve();
  return withLock("kgc", async () => {
    const cfg = await getNotifConfig();
    const seen = await getSeenIds("kgc");
    const newEntries = entries.filter((e) => !seen.has(e.id));
    if (!newEntries.length) {
      if (seen.size === 0) {
        for (const e of entries) seen.add(e.id);
        await saveSeenIds("kgc", seen);
      }
      return;
    }
    if (!courseNotificationAllowed("general", cfg)) {
      for (const e of newEntries) seen.add(e.id);
      await saveSeenIds("kgc", seen);
      return;
    }
    const granted = await ensurePermission();
    if (!granted) return;
    for (const e of newEntries) {
      nativeNotify(
        e.category ? `[${e.category}] ${e.title}` : e.title,
        e.date,
      ).catch(console.warn);
      seen.add(e.id);
    }
    await saveSeenIds("kgc", seen);
  });
}

/** Check Luna notifications for new items and send native notifications */
export function notifyNewLuna(items: LunaNotif[]): Promise<void> {
  if (!items.length) return Promise.resolve();
  return withLock("luna", async () => {
    const cfg = await getNotifConfig();
    const seen = await getSeenIds("luna");
    const makeKey = (n: LunaNotif) => `${n.date}|${n.course_info}|${n.content}`;
    const newItems = items.filter((n) => !seen.has(makeKey(n)));
    if (!newItems.length) {
      if (seen.size === 0) {
        for (const n of items) seen.add(makeKey(n));
        await saveSeenIds("luna", seen);
      }
      return;
    }
    const allowedItems = newItems.filter((n) =>
      courseNotificationAllowed(classifyCourseNotification(n.module), cfg)
    );
    if (!allowedItems.length) {
      for (const n of newItems) seen.add(makeKey(n));
      await saveSeenIds("luna", seen);
      return;
    }
    const granted = await ensurePermission();
    if (!granted) return;
    for (const n of allowedItems) {
      nativeNotify(
        n.module ? `[${n.module}] ${n.content}` : n.content,
        `${n.course_info} — ${n.date}`,
      ).catch(console.warn);
    }
    for (const n of newItems) seen.add(makeKey(n));
    await saveSeenIds("luna", seen);
  });
}

export interface KwicNotif {
  id: string;
  title: string;
  date: string;
  section: string;
  category: string;
  important: boolean;
}

/** Map KWIC section title to notification config key */
function kwicCategoryAllowed(section: string, cfg: NotificationConfig): boolean {
  if (section === "呼出し・重要なお知らせ") return cfg.notify_important;
  if (section === "学部・研究科からのお知らせ") return cfg.notify_faculty;
  if (section === "授業のお知らせ") return courseNotificationAllowed("general", cfg);
  return cfg.notify_other;
}

/** Check KWIC Portal notifications for new items and send native notifications */
export function notifyNewKwic(items: KwicNotif[]): Promise<void> {
  if (!items.length) return Promise.resolve();
  return withLock("kwic", async () => {
    const cfg = await getNotifConfig();
    const seen = await getSeenIds("kwic");
    const newItems = items.filter((n) => n.id && !seen.has(n.id));
    if (!newItems.length) {
      if (seen.size === 0) {
        for (const n of items) if (n.id) seen.add(n.id);
        await saveSeenIds("kwic", seen);
      }
      return;
    }
    const allowedItems = newItems.filter((n) => kwicCategoryAllowed(n.section, cfg));
    if (!allowedItems.length) {
      for (const n of newItems) seen.add(n.id);
      await saveSeenIds("kwic", seen);
      return;
    }
    const granted = await ensurePermission();
    if (!granted) return;
    for (const n of allowedItems) {
      nativeNotify(
        n.category ? `[${n.category}] ${n.title}` : n.title,
        n.date,
      ).catch(console.warn);
    }
    for (const n of newItems) {
      seen.add(n.id);
    }
    await saveSeenIds("kwic", seen);
  });
}

interface MailNotif {
  id: string;
  subject: string | null;
  from: { emailAddress: { name: string | null; address: string | null } } | null;
  isRead: boolean | null;
}

/** Check mail inbox for new unread items and send native notifications */
export function notifyNewMail(items: MailNotif[]): Promise<void> {
  if (!items.length) return Promise.resolve();
  return withLock("mail", async () => {
    const cfg = await getNotifConfig();
    const seen = await getSeenIds("mail");
    const newItems = items.filter((n) => n.id && !seen.has(n.id) && !n.isRead);
    if (!newItems.length) {
      if (seen.size === 0) {
        for (const n of items) if (n.id) seen.add(n.id);
        await saveSeenIds("mail", seen);
      }
      return;
    }
    if (!cfg.notify_mail) {
      for (const n of newItems) seen.add(n.id);
      await saveSeenIds("mail", seen);
      return;
    }
    const granted = await ensurePermission();
    if (!granted) return;
    for (const n of newItems) {
      const sender = n.from?.emailAddress?.name || n.from?.emailAddress?.address || "";
      const subj = n.subject || "(件名なし)";
      nativeNotify(
        sender ? `${sender}` : "新着メール",
        subj,
      ).catch(console.warn);
      seen.add(n.id);
    }
    await saveSeenIds("mail", seen);
  });
}
