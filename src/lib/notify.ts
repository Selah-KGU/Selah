import { invoke } from "@tauri-apps/api/core";

const SEEN_KGC_KEY = "selah_seen_kgc_notifs";
const SEEN_LUNA_KEY = "selah_seen_luna_notifs";
const SEEN_KWIC_KEY = "selah_seen_kwic_notifs";

/** Send a native macOS notification via osascript */
export async function nativeNotify(title: string, body?: string) {
  await invoke("test_notification", { title, body: body ?? "" });
}

function getSeenIds(key: string): Set<string> {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return new Set();
    return new Set(JSON.parse(raw));
  } catch {
    return new Set();
  }
}

function saveSeenIds(key: string, ids: Set<string>) {
  try {
    // Keep only last 500 IDs to avoid unbounded growth
    const arr = [...ids].slice(-500);
    localStorage.setItem(key, JSON.stringify(arr));
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
  const seen = getSeenIds(SEEN_KGC_KEY);
  const newEntries = entries.filter((e) => !seen.has(e.id));
  if (!newEntries.length) {
    // First run: mark all as seen without notifying
    if (seen.size === 0) {
      for (const e of entries) seen.add(e.id);
      saveSeenIds(SEEN_KGC_KEY, seen);
    }
    return;
  }

  const granted = await ensurePermission();
  if (!granted) return;

  for (const e of newEntries) {
    nativeNotify(
      e.category ? `[${e.category}] ${e.title}` : e.title,
      e.date,
    );
    seen.add(e.id);
  }
  saveSeenIds(SEEN_KGC_KEY, seen);
}

/** Check Luna notifications for new items and send native notifications */
export async function notifyNewLuna(items: LunaNotif[]) {
  if (!items.length) return;
  const seen = getSeenIds(SEEN_LUNA_KEY);
  // Luna notifications don't have a unique ID, use composite key
  const makeKey = (n: LunaNotif) => `${n.date}|${n.course_info}|${n.content}`;
  const newItems = items.filter((n) => !seen.has(makeKey(n)));
  if (!newItems.length) {
    if (seen.size === 0) {
      for (const n of items) seen.add(makeKey(n));
      saveSeenIds(SEEN_LUNA_KEY, seen);
    }
    return;
  }

  const granted = await ensurePermission();
  if (!granted) return;

  for (const n of newItems) {
    nativeNotify(
      n.module ? `[${n.module}] ${n.content}` : n.content,
      `${n.course_info} — ${n.date}`,
    );
    seen.add(makeKey(n));
  }
  saveSeenIds(SEEN_LUNA_KEY, seen);
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
  const seen = getSeenIds(SEEN_KWIC_KEY);
  const newItems = items.filter((n) => n.id && !seen.has(n.id));
  if (!newItems.length) {
    if (seen.size === 0) {
      for (const n of items) if (n.id) seen.add(n.id);
      saveSeenIds(SEEN_KWIC_KEY, seen);
    }
    return;
  }

  const granted = await ensurePermission();
  if (!granted) return;

  for (const n of newItems) {
    nativeNotify(
      n.category ? `[${n.category}] ${n.title}` : n.title,
      n.date,
    );
    seen.add(n.id);
  }
  saveSeenIds(SEEN_KWIC_KEY, seen);
}
