import type { LiveTodoSuggestion } from "../../api";

export type NoticeKind = "error" | "success" | "warning";
export type NoticeSource = "general" | "readiness" | "stt";
export type NoticeAction = "open-ai-settings";
export type SttPhase = "idle" | "checking" | "starting" | "initializing" | "listening";

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
};

export type WhiteboardStagePreset = { width: number; height: number; zoom: number };

export type BoardHighlight = {
  nodes: Set<string>;
  edges: Set<string>;
} | null;
