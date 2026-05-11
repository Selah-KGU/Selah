import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { DownloadEvent, RuntimeUpdate } from "./updaterRuntime";

export type AppUpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "up-to-date"
  | "downloading"
  | "installing"
  | "error"
  | "unsupported";

export interface AppUpdateState {
  phase: AppUpdatePhase;
  available: boolean;
  version: string;
  notes: string;
  status: string;
  checking: boolean;
  downloadedBytes: number;
  totalBytes: number | null;
  progressPercent: number | null;
}

export type DistributionChannel = "direct" | "appstore" | "msstore";

function normalizeDistributionChannel(value: unknown): DistributionChannel {
  if (value === "appstore") return "appstore";
  if (value === "msstore") return "msstore";
  return "direct";
}

const RAW_DISTRIBUTION_CHANNEL = import.meta.env.VITE_SELAH_DISTRIBUTION_CHANNEL;
const IS_STORE_MANAGED_BUILD =
  RAW_DISTRIBUTION_CHANNEL === "appstore" || RAW_DISTRIBUTION_CHANNEL === "msstore";

export const distributionChannel = normalizeDistributionChannel(RAW_DISTRIBUTION_CHANNEL);
export const updaterManagedByStore = IS_STORE_MANAGED_BUILD;

function defaultStatus(): string {
  if (distributionChannel === "appstore") {
    return "このビルドの更新は Mac App Store から配信されます。";
  }
  if (distributionChannel === "msstore") {
    return "このビルドの更新は Microsoft Store から配信されます。";
  }
  return "更新を確認すると、GitHub Releases から新しい版を取得します。";
}

const DEFAULT_STATUS = defaultStatus();

const initialState: AppUpdateState = {
  phase: "idle",
  available: false,
  version: "",
  notes: "",
  status: DEFAULT_STATUS,
  checking: false,
  downloadedBytes: 0,
  totalBytes: null,
  progressPercent: null,
};

export const appUpdateState = writable<AppUpdateState>(initialState);

let pendingUpdate: RuntimeUpdate | null = null;
let silentCheckStarted = false;
let activeCheckPromise: Promise<void> | null = null;

function readDemoFlag(): boolean {
  try {
    return localStorage.getItem("selah-demo-mode") === "1";
  } catch {
    return false;
  }
}

function normalizeUpdaterError(error: unknown): { message: string; unsupported: boolean } {
  const message = error instanceof Error ? error.message : String(error);
  if (
    message.includes("plugin") ||
    message.includes("updater") ||
    message.includes("command check not found")
  ) {
    return {
      unsupported: true,
      message: "このビルドでは自動更新を利用できません。Releases から更新してください。",
    };
  }
  if (message.includes("Network") || message.includes("timed out")) {
    return {
      unsupported: false,
      message: "更新を確認できませんでした。ネットワーク接続を確認してください。",
    };
  }
  return {
    unsupported: false,
    message: `更新の確認に失敗しました: ${message}`,
  };
}

async function replacePendingUpdate(next: RuntimeUpdate | null) {
  if (pendingUpdate && pendingUpdate !== next) {
    try {
      await pendingUpdate.close();
    } catch {
      // noop
    }
  }
  pendingUpdate = next;
}

function updateStore(patch: Partial<AppUpdateState>) {
  appUpdateState.update((state) => ({ ...state, ...patch }));
}

function applyDownloadEvent(event: DownloadEvent) {
  if (event.event === "Started") {
    updateStore({
      phase: "downloading",
      status: "更新をダウンロードしています...",
      downloadedBytes: 0,
      totalBytes: event.data.contentLength ?? null,
      progressPercent: event.data.contentLength ? 0 : null,
    });
    return;
  }

  if (event.event === "Progress") {
    const current = get(appUpdateState);
    const downloadedBytes = current.downloadedBytes + event.data.chunkLength;
    const progressPercent = current.totalBytes && current.totalBytes > 0
      ? Math.min(100, Math.round((downloadedBytes / current.totalBytes) * 100))
      : null;
    updateStore({
      phase: "downloading",
      status: progressPercent != null
        ? `更新をダウンロードしています... ${progressPercent}%`
        : "更新をダウンロードしています...",
      downloadedBytes,
      progressPercent,
    });
    return;
  }

  updateStore({
    phase: "installing",
    status: "ダウンロードが完了しました。更新を適用しています...",
    progressPercent: 100,
  });
}

async function runCheck(options: { silent: boolean }): Promise<void> {
  if (IS_STORE_MANAGED_BUILD) {
    updateStore({
      phase: "unsupported",
      available: false,
      checking: false,
      status: DEFAULT_STATUS,
      downloadedBytes: 0,
      totalBytes: null,
      progressPercent: null,
    });
    return;
  }

  if (readDemoFlag()) {
    return;
  }

  const current = get(appUpdateState);
  if (current.phase === "downloading" || current.phase === "installing") {
    return;
  }

  if (activeCheckPromise) {
    await activeCheckPromise;
    return;
  }

  activeCheckPromise = (async () => {
    updateStore({
      checking: true,
      phase: options.silent ? current.phase : "checking",
      status: options.silent ? current.status : "更新を確認しています...",
    });

    try {
      const { checkForRuntimeUpdate } = await import("./updaterRuntime");
      const update = await checkForRuntimeUpdate({ timeout: 30_000 });
      await replacePendingUpdate(update);

      if (!update) {
        updateStore({
          phase: "up-to-date",
          available: false,
          version: "",
          notes: "",
          status: "現在のバージョンが最新です。",
          checking: false,
          downloadedBytes: 0,
          totalBytes: null,
          progressPercent: null,
        });
        return;
      }

      updateStore({
        phase: "available",
        available: true,
        version: update.version,
        notes: (update.body || "").trim(),
        status: `バージョン ${update.version} を利用できます。`,
        checking: false,
        downloadedBytes: 0,
        totalBytes: null,
        progressPercent: null,
      });
    } catch (error) {
      const normalized = normalizeUpdaterError(error);
      updateStore({
        phase: normalized.unsupported ? "unsupported" : "error",
        available: false,
        version: "",
        notes: "",
        status: normalized.message,
        checking: false,
        downloadedBytes: 0,
        totalBytes: null,
        progressPercent: null,
      });
    } finally {
      activeCheckPromise = null;
    }
  })();

  await activeCheckPromise;
}

export async function startSilentUpdateCheck(): Promise<void> {
  if (silentCheckStarted) return;
  silentCheckStarted = true;
  await runCheck({ silent: true });
}

export async function checkForAppUpdate(): Promise<void> {
  await runCheck({ silent: false });
}

export async function downloadAndInstallAppUpdate(): Promise<void> {
  if (!pendingUpdate) return;

  updateStore({
    phase: "downloading",
    checking: false,
    downloadedBytes: 0,
    totalBytes: null,
    progressPercent: null,
    status: "更新をダウンロードしています...",
  });

  try {
    await pendingUpdate.downloadAndInstall((event) => {
      applyDownloadEvent(event);
    }, { timeout: 120_000 });

    await replacePendingUpdate(null);

    updateStore({
      phase: "installing",
      available: false,
      status: "更新をインストールしました。アプリを再起動しています...",
      progressPercent: 100,
    });

    await invoke<void>("request_app_restart");
  } catch (error) {
    updateStore({
      phase: "error",
      status: "更新のインストールに失敗しました: " + String(error),
      available: true,
    });
  }
}
