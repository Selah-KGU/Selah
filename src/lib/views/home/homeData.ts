import type { NotificationEntry } from "../../stores";
import type { KwicPortalHome } from "../../api";
import type { LunaNotification } from "../../types";
import { compareNotificationDatesDesc } from "../../date";

export interface UnifiedNotif {
  source: "kgc" | "luna" | "kwic";
  title: string;
  category: string;
  courseInfo?: string;
  date: string;
  section?: string;
  sender?: string;
  url?: string;
  body?: string;
  kwicId?: string;
  informationType?: string;
  personCategoryCd?: string;
  categoryCd?: string;
}

export interface AiNotifResult {
  summary: string;
  important: { title: string; reason: string; index: number }[];
  suggestions: string[];
}

const WMO_DESCRIPTIONS: Record<number, { label: string; icon: string }> = {
  0: { label: "快晴", icon: "☀️" },
  1: { label: "晴れ", icon: "🌤" },
  2: { label: "くもり", icon: "⛅" },
  3: { label: "曇天", icon: "☁️" },
  45: { label: "霧", icon: "🌫" },
  48: { label: "霧氷", icon: "🌫" },
  51: { label: "小雨", icon: "🌦" },
  53: { label: "雨", icon: "🌧" },
  55: { label: "強い雨", icon: "🌧" },
  56: { label: "着氷性の霧雨", icon: "🌧" },
  57: { label: "着氷性の雨", icon: "🌧" },
  61: { label: "小雨", icon: "🌦" },
  63: { label: "雨", icon: "🌧" },
  65: { label: "大雨", icon: "🌧" },
  66: { label: "着氷性の雨", icon: "🧊" },
  67: { label: "着氷性の大雨", icon: "🧊" },
  71: { label: "小雪", icon: "🌨" },
  73: { label: "雪", icon: "❄️" },
  75: { label: "大雪", icon: "❄️" },
  77: { label: "霧雪", icon: "🌨" },
  80: { label: "にわか雨", icon: "🌦" },
  81: { label: "にわか雨", icon: "🌧" },
  82: { label: "激しいにわか雨", icon: "⛈" },
  85: { label: "にわか雪", icon: "🌨" },
  86: { label: "激しいにわか雪", icon: "❄️" },
  95: { label: "雷雨", icon: "⛈" },
  96: { label: "雷雨（雹）", icon: "⛈" },
  99: { label: "激しい雷雨（雹）", icon: "⛈" },
};

const GREETINGS: Record<string, string[]> = {
  night: [
    "おやすみなさい", "夜更かしはほどほどに",
    "明日に備えよう", "そろそろ休もう",
  ],
  morning: [
    "おはよう", "いい朝だね",
    "今日もがんばろう", "いい一日にしよう",
  ],
  day: [
    "こんにちは", "午後もがんばろう",
    "もうひとふんばり", "いい調子",
  ],
  evening: [
    "おつかれさま", "今日もおつかれ",
    "ゆっくり休んでね", "もうひと息",
  ],
};

export function getGreetingSlot(date: Date) {
  const hour = date.getHours();
  return hour < 5 ? 0 : hour < 11 ? 1 : hour < 17 ? 2 : 3;
}

export function pickStableGreeting(date: Date): string {
  const hour = date.getHours();
  const slot = hour < 5 ? "night" : hour < 11 ? "morning" : hour < 17 ? "day" : "evening";
  const pool = GREETINGS[slot];
  const daySeed = date.getFullYear() * 400 + date.getMonth() * 32 + date.getDate();
  return pool[daySeed % pool.length];
}

export function getWeatherInfo(code: number) {
  return WMO_DESCRIPTIONS[code] ?? { label: "不明", icon: "🌡" };
}

export function getRecentNotifications(
  kgcNotifs: NotificationEntry[],
  lunaNotifs: LunaNotification[],
  kwicHome: KwicPortalHome | null,
): UnifiedNotif[] {
  const merged: UnifiedNotif[] = [];
  const seen = new Set<string>();
  const addUniq = (notif: UnifiedNotif) => {
    const key = `${notif.source}|${notif.title.trim().replace(/\s+/g, "")}|${notif.date}`;
    if (seen.has(key)) return;
    seen.add(key);
    merged.push(notif);
  };

  for (const notif of kgcNotifs) {
    addUniq({
      source: "kgc",
      title: notif.title,
      category: notif.category,
      date: notif.date,
    });
  }

  for (const notif of lunaNotifs) {
    addUniq({
      source: "luna",
      title: notif.content,
      category: notif.module || notif.course_info,
      courseInfo: notif.course_info,
      date: notif.date,
      url: notif.url,
    });
  }

  if (kwicHome) {
    const notifSections = kwicHome.sections.filter(
      section => section.title !== "メインリンク" && section.title !== "注目コンテンツ",
    );
    for (const section of notifSections) {
      for (const item of section.items) {
        addUniq({
          source: "kwic",
          title: item.title,
          category: item.category || section.title,
          date: item.date,
          section: section.title,
          kwicId: item.id,
          informationType: item.information_type,
          personCategoryCd: item.person_category_cd,
          categoryCd: item.category_cd,
        });
      }
    }
  }

  merged.sort((a, b) => compareNotificationDatesDesc(a.date, b.date));

  return merged.slice(0, 3);
}

export function daysUntil(deadline: string, now: Date): number {
  const target = new Date(deadline.replace(/\//g, "-"));
  return Math.ceil((target.getTime() - now.getTime()) / 86400000);
}
