import { writable, get } from "svelte/store";

export const demoMode = writable(false);

export function isDemoMode(): boolean {
  return get(demoMode);
}
