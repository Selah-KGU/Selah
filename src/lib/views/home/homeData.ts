import type { NotificationEntry } from "../../stores";
import type { KwicPortalHome } from "../../api";
import type { LunaNotification } from "../../types";

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

export interface AiNotifCache {
  timestamp: number;
  result: AiNotifResult;
  sources: UnifiedNotif[];
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

export const AI_CACHE_KEY = "ai-notif-cache:v2";
export const AI_REFRESH_MS = 12 * 60 * 60 * 1000;

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

  merged.sort((a, b) => {
    const dateA = new Date(a.date.replace(/\//g, "-")).getTime() || 0;
    const dateB = new Date(b.date.replace(/\//g, "-")).getTime() || 0;
    return dateB - dateA;
  });

  return merged.slice(0, 3);
}

export function buildLocalSystemPrompt(nowStr: string, lang: string): string {
  let prompt = `あなたは関西学院大学の学生向けパーソナル通知アシスタントです。
学生のプロフィール（学部・キャンパス・履修科目・課題状況）と通知一覧を受け取り、今この学生にとって重要な情報をJSON形式で出力します。

現在の日時: ${nowStr}
この日時がすべての判断の基準です。

# キャンパスと学部
- 西宮上ケ原（NUC）：神学部、文学部、社会学部、法学部、経済学部、商学部、人間福祉学部、国際学部、教育学部
- 神戸三田（KSC）：総合政策学部、理学部、工学部、生命環境学部、建築学部
NUCとKSCは約40km離れており、別キャンパスの通知は基本無関係です。

# 判定基準
各通知について以下を判定してください：
- 日程が現在より前 → 終了済み → 除外
- 学生の所属キャンパス・学部と無関係 → 除外
- 各通知は独立です。似たタイトルでも各通知の「内容:」から個別に日程を読み取ること
- 「内容:」がない通知はタイトルと日付で判断

# 出力ルール

summary（80〜150字）:
- 除外した通知には言及しない
- 課題の締切は「あとN日」と残り日数を書く
- 具体的な情報（教室名・時間・場所）を引用する

important（最大5件）:
- 除外した通知は入れない
- indexは通知一覧の番号（1始まり）と一致させる
- 優先: 履修科目関連 > 学部関連 > キャンパス内イベント > 全学共通

suggestions（最大3件、各10〜20字）:
- 通知の繰り返しではなく、一歩踏み込んだ行動提案を書く
- カジュアルな口調（丁寧語・命令形は使わない）
- 終了済みイベントのsuggestionsは書かない
- 良い例:「レポートは構成だけ先に書いとくといいよ」
- 悪い例:「〇〇のクイズ、あと3日」（これはsummaryの内容）

# 出力形式
以下のJSONのみ出力。JSON以外のテキスト・説明・前置きは絶対に書かないでください。

{"summary":"...","important":[{"title":"20字以内","reason":"15字以内","index":番号}],"suggestions":["..."]}`;
  if (lang) prompt += `\n\nsummary, title, reason, suggestionsは${lang}で書くこと。`;
  return prompt;
}

export function parseAiNotifResponse(raw: string): AiNotifResult {
  let cleaned = raw.replace(/<think[\s\S]*?<\/think>/gi, "");
  cleaned = cleaned.replace(/<think>/gi, "").replace(/<\/think>/gi, "").trim();
  const match = cleaned.match(/\{[\s\S]*\}/);
  if (!match) throw new Error("AI応答の解析に失敗しました");
  const parsed = JSON.parse(match[0]);
  delete parsed._check;
  return parsed;
}

export function daysUntil(deadline: string, now: Date): number {
  const target = new Date(deadline.replace(/\//g, "-"));
  return Math.ceil((target.getTime() - now.getTime()) / 86400000);
}
