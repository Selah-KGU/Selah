import { invoke } from "@tauri-apps/api/core";

function isDemoStorageFlag(): boolean {
  try {
    return localStorage.getItem("selah-demo-mode") === "1";
  } catch {
    return false;
  }
}

export interface OpenExternalUrlOptions {
  allowInDemo?: boolean;
}

export async function openExternalUrl(
  url: string,
  options: OpenExternalUrlOptions = {}
): Promise<void> {
  if (isDemoStorageFlag() && !options.allowInDemo) return;
  await invoke<void>("open_external_url", { url });
}

export async function setAppTheme(theme: "light" | "dark" | "system"): Promise<void> {
  if (isDemoStorageFlag()) return;
  await invoke<void>("set_app_theme", { theme });
}
