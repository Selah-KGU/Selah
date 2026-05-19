import { invoke } from "@tauri-apps/api/core";
import {
  gcalCheckSession,
  getAiConfig,
  isAiReady,
  isDemoActive,
  mailCheckSession,
} from "../api";
import type { OnboardingPurpose } from "./onboardingState";

export type OnboardingStatus = "loading" | "ok" | "warn" | "off";

export interface OnboardingCheckRow {
  key: string;
  label: string;
  detail: string;
  status: OnboardingStatus;
  actionLabel: string;
  panel: string;
}

async function notificationRow(): Promise<OnboardingCheckRow> {
  try {
    const granted = isDemoActive()
      ? true
      : await invoke<boolean>("native_notification_permission_granted");
    return {
      key: "notif",
      label: "通知",
      detail: granted ? "システム通知が許可されています" : "未許可。授業・課題のお知らせを受け取れません",
      status: granted ? "ok" : "warn",
      actionLabel: granted ? "設定" : "許可する",
      panel: "notification",
    };
  } catch {
    return {
      key: "notif",
      label: "通知",
      detail: "状態を取得できません",
      status: "warn",
      actionLabel: "設定",
      panel: "notification",
    };
  }
}

async function mailRow(): Promise<OnboardingCheckRow> {
  try {
    const s = await mailCheckSession();
    return {
      key: "mail",
      label: "メール (Microsoft 365)",
      detail: s.authenticated
        ? `連携済み${s.email ? ` (${s.email})` : ""}`
        : "未連携。学内メールの受信ができません",
      status: s.authenticated ? "ok" : "warn",
      actionLabel: s.authenticated ? "管理" : "連携する",
      panel: "mail",
    };
  } catch {
    return {
      key: "mail",
      label: "メール (Microsoft 365)",
      detail: "状態を取得できません",
      status: "warn",
      actionLabel: "連携する",
      panel: "mail",
    };
  }
}

async function calendarRow(): Promise<OnboardingCheckRow> {
  try {
    const s = await gcalCheckSession();
    return {
      key: "gcal",
      label: "Google カレンダー",
      detail: s.authenticated
        ? `連携済み${s.synced_events != null ? ` (${s.synced_events}件同期)` : ""}`
        : "未連携（任意）。時間割を Google カレンダーに同期できます",
      status: s.authenticated ? "ok" : "off",
      actionLabel: s.authenticated ? "管理" : "連携する",
      panel: "calendar",
    };
  } catch {
    return {
      key: "gcal",
      label: "Google カレンダー",
      detail: "任意。連携で時間割を同期できます",
      status: "off",
      actionLabel: "連携する",
      panel: "calendar",
    };
  }
}

async function downloadRow(): Promise<OnboardingCheckRow> {
  try {
    const cfg = isDemoActive()
      ? { download_dir: "" }
      : await invoke<{ download_dir?: string; classify_by_course?: boolean }>("get_download_config");
    const dir = cfg.download_dir || "";
    return {
      key: "download",
      label: "資料フォルダ",
      detail: dir || "アプリ標準の資料フォルダを使用します",
      status: dir ? "ok" : "off",
      actionLabel: dir ? "変更" : "選ぶ",
      panel: "download",
    };
  } catch {
    return {
      key: "download",
      label: "資料フォルダ",
      detail: "状態を取得できません",
      status: "warn",
      actionLabel: "設定",
      panel: "download",
    };
  }
}

export async function isSttReady(): Promise<boolean> {
  if (isDemoActive()) return true;
  try {
    const [models, cfg] = await Promise.all([
      invoke<any[]>("list_stt_models"),
      invoke<any>("get_stt_config"),
    ]);
    const selected = models.find((m) => m.id === cfg?.selected_model);
    return !!selected?.downloaded;
  } catch {
    return false;
  }
}

async function sttRow(): Promise<OnboardingCheckRow> {
  const downloaded = await isSttReady();
  return {
    key: "stt",
    label: "音声認識モデル (LIVE / 音声 Agent)",
    detail: downloaded
      ? "ダウンロード済み。LIVE で文字起こしができます"
      : "未ダウンロード。LIVE と音声 Agent で必要です",
    status: downloaded ? "ok" : "warn",
    actionLabel: downloaded ? "管理" : "ダウンロード",
    panel: "ai",
  };
}

export async function loadOnboardingChecks(purposes: OnboardingPurpose[]): Promise<OnboardingCheckRow[]> {
  const checks = [
    notificationRow(),
    mailRow(),
    calendarRow(),
    downloadRow(),
  ];

  if (purposes.includes("live") || purposes.includes("voice")) {
    checks.push(sttRow());
  }

  return Promise.all(checks);
}

export async function getAiReadinessLabel(): Promise<{ ready: boolean; note: string }> {
  try {
    const cfg = await getAiConfig();
    if (cfg.ai_enabled === false) return { ready: false, note: "AI が無効です" };
    const ready = await isAiReady();
    if (ready) return { ready: true, note: "利用可能" };
    return {
      ready: false,
      note: cfg.provider === "local" ? "ローカルモデル未ダウンロード" : "API キー未設定",
    };
  } catch {
    return { ready: false, note: "状態を取得できません" };
  }
}
