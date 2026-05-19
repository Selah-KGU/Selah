import { writable, get, derived } from "svelte/store";
import { getAiConfig, isDemoActive } from "../api";

const STORAGE_KEY = "selah-onboarding-v1";
const RESUME_KEY = "selah-onboarding-resume";

export type OnboardingPurpose = "summary" | "agent" | "live" | "voice";

export interface OnboardingRecord {
  version: 1;
  purposes: OnboardingPurpose[];
  skippedAt?: string;
  completedAt?: string;
}

const EMPTY: OnboardingRecord = { version: 1, purposes: [] };

function load(): OnboardingRecord {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...EMPTY };
    const parsed = JSON.parse(raw);
    if (parsed?.version !== 1) return { ...EMPTY };
    return { ...EMPTY, ...parsed };
  } catch {
    return { ...EMPTY };
  }
}

function persist(rec: OnboardingRecord) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(rec)); } catch { /* ignore */ }
}

export const onboardingRecord = writable<OnboardingRecord>(load());
export const onboardingVisible = writable<boolean>(false);

/** True when the Home onboarding banner should appear. Reactive — re-derives whenever record changes. */
export const showHomeOnboardingCard = derived(onboardingRecord, (rec) => {
  if (isDemoActive()) return false;
  return !rec.completedAt;
});

export function updateRecord(patch: Partial<OnboardingRecord>) {
  onboardingRecord.update((rec) => {
    const next = { ...rec, ...patch };
    persist(next);
    return next;
  });
}

export function skipOnboarding() {
  updateRecord({ skippedAt: new Date().toISOString() });
  onboardingVisible.set(false);
}

export function completeOnboarding() {
  updateRecord({ completedAt: new Date().toISOString() });
  onboardingVisible.set(false);
}

export function reopenOnboarding() {
  onboardingVisible.set(true);
}

/** Save which step to resume to after a settings detour. */
export function markResume(step: string) {
  try { sessionStorage.setItem(RESUME_KEY, step); } catch { /* ignore */ }
}

/** Consume the resume token (one-shot). */
export function consumeResume(): string | null {
  try {
    const v = sessionStorage.getItem(RESUME_KEY);
    if (v) sessionStorage.removeItem(RESUME_KEY);
    return v;
  } catch {
    return null;
  }
}

export function hasResume(): boolean {
  try { return !!sessionStorage.getItem(RESUME_KEY); } catch { return false; }
}

/** Reset the onboarding record so the gate fires again on next boot. */
export function resetOnboarding() {
  try { localStorage.removeItem(STORAGE_KEY); } catch { /* ignore */ }
  try { sessionStorage.removeItem(RESUME_KEY); } catch { /* ignore */ }
  onboardingRecord.set({ ...EMPTY });
  onboardingVisible.set(false);
}

/** Clear every per-page first-visit tip dismissal. Returns count removed. */
export function clearAllFirstVisitTips(): number {
  let removed = 0;
  try {
    const keys: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && k.startsWith("selah-tip-") && k.endsWith("-v1")) keys.push(k);
    }
    for (const k of keys) { localStorage.removeItem(k); removed++; }
  } catch { /* ignore */ }
  return removed;
}

/**
 * Decide whether to auto-show onboarding modal on app boot.
 * Trigger only when never completed/skipped AND AI looks unconfigured.
 */
export async function shouldAutoShow(): Promise<boolean> {
  if (isDemoActive()) return false;
  const rec = get(onboardingRecord);
  if (rec.completedAt || rec.skippedAt) return false;
  try {
    const cfg = await getAiConfig();
    const hasApiKey = !!cfg.api_key?.trim();
    const isDefaultLocal = cfg.provider === "local";
    return !hasApiKey && isDefaultLocal;
  } catch {
    return true;
  }
}
