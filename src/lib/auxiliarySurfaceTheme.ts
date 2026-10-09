import { invoke } from "@tauri-apps/api/core";
import { normalizeEffectiveTheme } from "./themePreference";

export function applyAuxiliaryTheme(value: unknown): void {
  const theme = normalizeEffectiveTheme(value);
  if (theme) {
    document.documentElement.setAttribute("data-theme", theme);
    document.body.setAttribute("data-theme", theme);
  } else {
    document.documentElement.removeAttribute("data-theme");
    document.body.removeAttribute("data-theme");
  }
}

export async function syncAuxiliaryTheme(active: () => boolean = () => true): Promise<void> {
  if (!active()) return;
  try {
    const stored = localStorage.getItem("selah-theme") || "";
    const appTheme = await invoke<string>("get_app_theme");
    if (active()) applyAuxiliaryTheme(normalizeEffectiveTheme(appTheme) || stored);
  } catch {
    try {
      if (active()) applyAuxiliaryTheme(localStorage.getItem("selah-theme") || "");
    } catch {}
  }
}
