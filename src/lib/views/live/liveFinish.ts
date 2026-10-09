import type { LiveFinishPhase, LiveFinishProgress, LiveSessionSnapshot } from "../../liveSessionApi";
import { isCurrentLiveSessionEvent } from "./liveTranscript";

export const LIVE_FINISH_LABELS = {
  stopping: "録音を停止中",
  saving_record: "録音内容を保存中",
  summarizing: "AI要約を生成中",
  saving_final: "最終ファイルを保存中",
} as const;

type SavePresentation = { steps: readonly string[]; index: number; label: string };
const phases: LiveFinishPhase[] = ["stopping", "saving_record", "summarizing", "saving_final"];
const presentations: Partial<Record<LiveFinishPhase, SavePresentation>> = Object.fromEntries(phases.map((phase) => [phase, {
  steps: [], index: 0, label: `${LIVE_FINISH_LABELS[phase]}…`,
}]));

export function isLiveBusy(snapshot: Pick<LiveSessionSnapshot, "active" | "finish_phase">, localBusy: boolean): boolean {
  return localBusy || (snapshot.active && snapshot.finish_phase != null);
}

export function liveSavePresentation(snapshot: Pick<LiveSessionSnapshot, "active" | "finish_phase">): SavePresentation | null {
  if (!snapshot.active || !snapshot.finish_phase) return null;
  const presentation = presentations[snapshot.finish_phase];
  if (!presentation) return null;
  // The backend drains the decoder before deciding whether AI is needed. Its
  // current stage is authoritative; do not predict a step count before draining.
  return presentation;
}

/** A failed attempt and its retry keep the same recording ID. */
export function applyLiveFinishProgress<T extends Pick<LiveSessionSnapshot, "active" | "session_id" | "finish_phase" | "finish_revision">>(
  snapshot: T,
  progress: LiveFinishProgress,
): T {
  if (!isCurrentLiveSessionEvent(snapshot, progress)
    || progress.finish_revision < (snapshot.finish_revision ?? 0)) return snapshot;
  return { ...snapshot, finish_phase: progress.finish_phase, finish_revision: progress.finish_revision };
}
