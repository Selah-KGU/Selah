type PrebootLog = {
  type: "error" | "warn" | "info";
  message: string;
  time: string;
};

declare global {
  interface Window {
    __SELAH_PREBOOT_LOGS__?: PrebootLog[];
    __TAURI_INTERNALS__?: unknown;
  }
}

const maxLogs = 50;
const logs: PrebootLog[] = [];
window.__SELAH_PREBOOT_LOGS__ = logs;

function addLog(type: PrebootLog["type"], message: string): void {
  if (logs.length >= maxLogs) logs.shift();
  logs.push({
    type,
    message,
    time: new Date().toLocaleTimeString("ja-JP"),
  });
}

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
