import { invoke } from "@tauri-apps/api/core";

export type SttStreamPhase = "idle" | "initializing" | "listening" | "stopping";
export interface SttStreamOwner {
  caller: string;
  live_session_id: string | null;
  input_session_id?: string | null;
}
export interface SttStreamState {
  phase: SttStreamPhase;
  session_id: number | null;
  owner: SttStreamOwner | null;
}

export const idleSttStreamState = (): SttStreamState => ({ phase: "idle", session_id: null, owner: null });
export const getSttStreamState = () => invoke<SttStreamState>("stt_get_stream_state");

export function startSttStream(owner: SttStreamOwner, preempt = false) {
  return invoke<SttStreamOwner | null>("stt_start_stream", {
    caller: owner.caller,
    preempt,
    ...(owner.live_session_id ? { liveSessionId: owner.live_session_id } : {}),
    ...(owner.input_session_id ? { inputSessionId: owner.input_session_id } : {}),
  });
}

export function stopSttStream(owner: SttStreamOwner) {
  return invoke<void>("stt_stop_stream", {
    caller: owner.caller,
    ...(owner.live_session_id ? { liveSessionId: owner.live_session_id } : {}),
    ...(owner.input_session_id ? { inputSessionId: owner.input_session_id } : {}),
  });
}

/** A paused LIVE UUID cannot inherit the microphone of a different recording. */
export function ownsSttStream(state: SttStreamState, caller: string, liveSessionId?: string | null): boolean {
  return state.owner?.caller === caller &&
    (caller !== "live" || (!!liveSessionId && state.owner.live_session_id === liveSessionId));
}

export function isSttStreamActive(state: SttStreamState, caller: string, liveSessionId?: string | null): boolean {
  return ownsSttStream(state, caller, liveSessionId) &&
    (state.phase === "initializing" || state.phase === "listening");
}

export function liveSttPhase(state: SttStreamState, liveSessionId?: string | null): "idle" | "initializing" | "listening" {
  if (!ownsSttStream(state, "live", liveSessionId)) return "idle";
  return state.phase === "initializing" || state.phase === "listening" ? state.phase : "idle";
}
