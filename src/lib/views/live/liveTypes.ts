import type { LiveTodoSuggestion } from "../../api";

export type NoticeKind = "error" | "success" | "warning";
export type NoticeSource = "general" | "readiness" | "stt";
export type NoticeAction = "open-ai-settings";
export type SttPhase = "idle" | "checking" | "starting" | "initializing" | "listening";
export type LiveControlPhase =
  | "idle"
  | "blocked"
  | "booting"
  | "recording"
  | "paused"
  | "thinking"
  | "saved";
export type LiveControlTone = "neutral" | "ready" | "recording" | "paused" | "thinking" | "blocked" | "saved";
export type LivePrimaryAction = "start" | "pause" | "resume" | "settings" | "none";

export type LiveControlModel = {
  phase: LiveControlPhase;
  tone: LiveControlTone;
  statusLabel: string;
  targetLabel: string;
  targetMeta: string;
  progressLabel: string;
  saveSteps: readonly string[];
  saveStepIndex: number;
  detailLabel: string;
  elapsedLabel: string;
  lineCountLabel: string;
  summaryCountLabel: string;
  pauseHintLabel: string;
  primaryAction: LivePrimaryAction;
  primaryLabel: string;
  primaryDisabled: boolean;
  primaryTitle: string;
  showModeSelect: boolean;
  showSummaryAction: boolean;
  showSaveAction: boolean;
  showClearAction: boolean;
};

export type NoticeState = {
  kind: NoticeKind;
  text: string;
  source: NoticeSource;
  action?: NoticeAction;
} | null;

export type LiveTodoDraft = LiveTodoSuggestion & { selected: boolean };

export type TermFloatLabels = {
  title: string;
  boardTitle: string;
  empty: string;
  source: string;
  externalSource: string;
  externalNode: string;
  collapse: string;
  expand: string;
  previous: string;
  next: string;
  selectAll: string;
  deselectAll: string;
};

export type WhiteboardStagePreset = { width: number; height: number; zoom: number };

export type BoardHighlight = {
  nodes: Set<string>;
  edges: Set<string>;
} | null;
