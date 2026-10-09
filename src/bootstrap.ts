type PrebootLog = {
  type: "error" | "warn" | "info";
  message: string;
  time: string;
};

declare global {
  interface Window {
    __SELAH_PREBOOT_LOGS__?: PrebootLog[];
    __SELAH_REPORT_ERROR__?: (message: string) => void;
    __TAURI_INTERNALS__?: unknown;
  }
}

const maxLogs = 50;
const logs: PrebootLog[] = [];
window.__SELAH_PREBOOT_LOGS__ = logs;
const errorLogKey = "selah-frontend-errors";
// Keep bounded local diagnostics across a recovery reload. No transcript or
// remote telemetry is added; this is the same error feed the debug UI displays.
try {
  const saved: unknown = JSON.parse(localStorage.getItem(errorLogKey) || "[]");
  if (Array.isArray(saved)) {
    logs.push(...saved.filter((entry) => entry?.type === "error" && typeof entry.message === "string")
      .slice(-maxLogs));
  }
} catch { /* storage may be unavailable */ }

function addLog(type: PrebootLog["type"], message: string): void {
  if (logs.length >= maxLogs) logs.shift();
  logs.push({
    type,
    message: message.slice(0, 8192),
    time: new Date().toLocaleTimeString("ja-JP"),
  });
  if (type === "error") {
    try {
      localStorage.setItem(errorLogKey, JSON.stringify(logs.filter((entry) => entry.type === "error")));
    } catch { /* diagnostics must not cause another render failure */ }
    // A local native log survives a WebContent crash. Keep this independent of
    // console calls, which are removed from production builds.
    try {
      const bridge = window.__TAURI_INTERNALS__ as { invoke?: (command: string, args: unknown) => Promise<unknown> } | undefined;
      void bridge?.invoke?.("frontend_report_error", { message: message.slice(0, 8192) }).catch(() => {});
    } catch { /* reporting must never throw */ }
  }
}

// Production drops console calls, so handled render errors need a direct sink.
window.__SELAH_REPORT_ERROR__ = (message) => addLog("error", message);

window.addEventListener("error", event => {
  addLog("error", event.message + (event.filename ? ` @ ${event.filename}:${event.lineno}` : ""));
});

window.addEventListener("unhandledrejection", event => {
  const reason = event.reason;
  addLog("error", `Unhandled Promise: ${reason instanceof Error ? reason.message : String(reason)}`);
});

const originalError = console.error;
console.error = (...args: unknown[]) => {
  addLog("error", args.join(" "));
  originalError.apply(console, args);
};

const originalWarn = console.warn;
console.warn = (...args: unknown[]) => {
  addLog("warn", args.join(" "));
  originalWarn.apply(console, args);
};

addLog("info", `Pre-boot started at ${new Date().toLocaleTimeString()}`);
addLog("info", `Location: ${window.location.href}`);
addLog("info", `UA: ${navigator.userAgent.substring(0, 80)}`);
addLog("info", `#app element: ${document.getElementById("app") ? "found" : "NOT FOUND"}`);
addLog("info", `__TAURI_INTERNALS__: ${typeof window.__TAURI_INTERNALS__ !== "undefined" ? "available" : "NOT available"}`);

import { isAuxiliarySurface } from "./lib/surfaceKind";

if (navigator.userAgent.includes("Windows")) {
  document.body.classList.add("platform-windows");
}

// Secondary WebViews must not parse the dashboard graph or register app-wide listeners.
if (isAuxiliarySurface()) {
  void import("./surfaceMain");
} else {
  void import("./main");
}

export {};
