import { check, type CheckOptions, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

export type RuntimeUpdate = Update;
export type { DownloadEvent };

export function checkForRuntimeUpdate(options?: CheckOptions): Promise<RuntimeUpdate | null> {
  return check(options);
}
