import { invoke } from "@tauri-apps/api/core";

/** Send a native notification */
export async function nativeNotify(title: string, body?: string) {
  await invoke("test_notification", { title, body: body ?? "" });
}

interface NotificationConfig {
  notify_important: boolean;
  notify_faculty: boolean;
  notify_class: boolean;
  notify_other: boolean;
  notify_mail: boolean;
}

async function getNotifConfig(): Promise<NotificationConfig> {
  try {
    return await invoke("get_notification_config") as NotificationConfig;
  } catch {
    return { notify_important: true, notify_faculty: true, notify_class: true, notify_other: true, notify_mail: true };
  }
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
    if (!cfg.notify_class) {
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
    if (!cfg.notify_class) {
      for (const n of newItems) seen.add(makeKey(n));
      await saveSeenIds("luna", seen);
      return;
    }
    const granted = await ensurePermission();
    if (!granted) return;
    for (const n of newItems) {
      nativeNotify(
        n.module ? `[${n.module}] ${n.content}` : n.content,
        `${n.course_info} — ${n.date}`,
      ).catch(console.warn);
      seen.add(makeKey(n));
    }
    await saveSeenIds("luna", seen);
  });
}

export interface KwicNotif {
  id: string;
  title: string;
  date: string;
  category: string;
  important: boolean;
}

/** Map KWIC category to notification config key */
function kwicCategoryAllowed(category: string, cfg: NotificationConfig): boolean {
  if (category === "呼出し・重要なお知らせ") return cfg.notify_important;
  if (category === "学部・研究科からのお知らせ") return cfg.notify_faculty;
  // "その他" and any unknown categories
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
    const granted = await ensurePermission();
    if (!granted) return;
    for (const n of newItems) {
      if (kwicCategoryAllowed(n.category, cfg)) {
        nativeNotify(
          n.category ? `[${n.category}] ${n.title}` : n.title,
          n.date,
        ).catch(console.warn);
      }
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
