import { invoke } from "@tauri-apps/api/core";

/** Send a native macOS notification via osascript */
export async function nativeNotify(title: string, body?: string) {
  await invoke("test_notification", { title, body: body ?? "" });
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
export async function notifyNewKgc(entries: KgcNotif[]) {
  if (!entries.length) return;
  const seen = await getSeenIds("kgc");
  const newEntries = entries.filter((e) => !seen.has(e.id));
  if (!newEntries.length) {
    // First run: mark all as seen without notifying
    if (seen.size === 0) {
      for (const e of entries) seen.add(e.id);
      await saveSeenIds("kgc", seen);
    }
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
}

/** Check Luna notifications for new items and send native notifications */
export async function notifyNewLuna(items: LunaNotif[]) {
  if (!items.length) return;
  const seen = await getSeenIds("luna");
  // Luna notifications don't have a unique ID, use composite key
  const makeKey = (n: LunaNotif) => `${n.date}|${n.course_info}|${n.content}`;
  const newItems = items.filter((n) => !seen.has(makeKey(n)));
  if (!newItems.length) {
    if (seen.size === 0) {
      for (const n of items) seen.add(makeKey(n));
      await saveSeenIds("luna", seen);
    }
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
}

export interface KwicNotif {
  id: string;
  title: string;
  date: string;
  category: string;
  important: boolean;
}

/** Check KWIC Portal notifications for new items and send native notifications */
export async function notifyNewKwic(items: KwicNotif[]) {
  if (!items.length) return;
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
    nativeNotify(
      n.category ? `[${n.category}] ${n.title}` : n.title,
      n.date,
    ).catch(console.warn);
    seen.add(n.id);
  }
  await saveSeenIds("kwic", seen);
}

interface MailNotif {
  id: string;
  subject: string | null;
  from: { emailAddress: { name: string | null; address: string | null } } | null;
  isRead: boolean | null;
}

/** Check mail inbox for new unread items and send native notifications */
export async function notifyNewMail(items: MailNotif[]) {
  if (!items.length) return;
  const seen = await getSeenIds("mail");
  const newItems = items.filter((n) => n.id && !seen.has(n.id) && !n.isRead);
  if (!newItems.length) {
    if (seen.size === 0) {
      for (const n of items) if (n.id) seen.add(n.id);
      await saveSeenIds("mail", seen);
    }
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
}
