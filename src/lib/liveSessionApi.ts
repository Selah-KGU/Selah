import { invoke } from "@tauri-apps/api/core";
import { liveSurfaceSnapshot, liveSurfaceSaveResult } from "./liveSurfaceSnapshot";
import { expandLiveSurface, expandLiveSurfaceSave, type CompactLiveSurfaceSnapshot, type CompactLiveSurfaceSaveResult } from "./liveBoardTransport";

/** LIVE session types and Tauri commands. Demo playback stays with the session. */

function _isDemo(): boolean {
  try { return localStorage.getItem("selah-demo-mode") === "1"; } catch { return false; }
}

export interface LiveCourseInfo {
  course_name: string;
  course_code: string;
  room: string;
  teacher: string;
  day: number;
  period: number;
  time_label: string;
  is_free_note: boolean;
}

export interface LiveTranscriptLine {
  text: string;
  at: string;
}

export interface LiveTermExplanation {
  term: string;
  explanation: string;
  source_excerpt?: string;
  external_source?: string;
}

export interface LiveWhiteboardNode {
  id: string;
  label: string;
  detail?: string;
  node_type?: "structure" | "term" | string;
  kind?: "core" | "support" | "question" | "result" | string;
  role?: "main" | "branch" | string;
  parent_id?: string;
  source_type?: "lecture" | "external" | string;
  source_excerpt?: string;
  external_source?: string;
}

export interface LiveWhiteboardEdge {
  from: string;
  to: string;
  label?: string;
}

export interface LiveWhiteboard {
  title: string;
  layout?: "flow" | "hub" | "compare" | "cycle" | "grid" | string;
  nodes?: LiveWhiteboardNode[];
  edges?: LiveWhiteboardEdge[];
  /** Protocol version. 0 = legacy, 1 = node_type + normalized_by supported. */
  schema_version?: number;
  /** Which layer last performed structural normalization: "backend" | "". */
  normalized_by?: string;
}

export interface LiveSummaryChunk {
  title: string;
  range_label: string;
  body: string;
  line_count: number;
  terms?: LiveTermExplanation[];
  whiteboard?: LiveWhiteboard | null;
}

export type LiveFinishPhase = "stopping" | "saving_record" | "summarizing" | "saving_final";

export interface LiveFinishProgress {
  session_id: string;
  finish_phase: LiveFinishPhase;
  finish_revision: number;
}

export interface LiveSessionSnapshot {
  /** Capture order across recordings in the running native backend. */
  update_revision?: number;
  /** Client-side consumed prefix from a slim update, even before gap recovery. */
  pending_from_line?: number;
  session_id?: string | null;
  active: boolean;
  course: LiveCourseInfo | null;
  started_at: string | null;
  transcript_lines: LiveTranscriptLine[];
  pending_lines: LiveTranscriptLine[];
  summaries: LiveSummaryChunk[];
  /** Epoch millis when the next periodic summary is due. */
  next_summary_at_ms?: number | null;
  /** True while a periodic summary is being generated. */
  summarizing?: boolean;
  /** Current backend save stage, also available after a WebView reload. */
  finish_phase?: LiveFinishPhase | null;
  /** Orders save stages and retries within a recording. */
  finish_revision?: number;
}

/** Page wire reply; complete transcript and pending arrays stay in the backend. */
export interface LiveSurfaceSnapshot extends Omit<LiveSessionSnapshot, "transcript_lines" | "pending_lines"> {
  transcript_line_count: number;
  visible_lines: LiveTranscriptLine[];
  pending_from_line: number;
}

/** Complete decoded update; versioned event references are resolved before application. */
export interface LiveSessionUpdate {
  update_revision: number;
  session_id: string | null;
  active: boolean;
  course: LiveCourseInfo | null;
  started_at: string | null;
  next_summary_at_ms: number | null;
  summarizing: boolean;
  finish_phase: LiveFinishPhase | null;
  finish_revision: number;
  transcript_line_count: number;
  pending_line_count: number;
  summary_count: number;
  latest_summary?: LiveSummaryChunk | null;
}

export interface LiveTranscriptUpdate {
  session_id: string;
  line_count: number;
  line: LiveTranscriptLine;
  /** Native STT capture order; absent for a manually appended line. */
  seq?: number;
}

export interface LiveSaveResult {
  saved: boolean;
  path: string;
  markdown: string;
  snapshot: LiveSessionSnapshot;
  suggested_todos?: LiveTodoSuggestion[];
  /** TODO/DDL extraction is running in the background; suggestions arrive via
   *  the `live-todo-suggestions` event. */
  todos_pending?: boolean;
}

export interface LiveSurfaceSaveResult extends Omit<LiveSaveResult, "snapshot" | "markdown"> {
  snapshot: LiveSurfaceSnapshot;
  /** Exact Markdown previously rendered in the saved-note preview. */
  summary_markdown: string;
}

/** Payload of the `live-todo-suggestions` event. */
export interface LiveTodoSuggestionsEvent {
  suggestions: LiveTodoSuggestion[];
  source_path: string;
}

export interface LiveTodoSuggestion {
  title: string;
  course_name: string;
  content_type: string;
  deadline: string;
  note: string;
  source_excerpt: string;
  day: number;
  period: number;
}

export interface LiveGeneratedTodo extends LiveTodoSuggestion {
  id: string;
  created_at: string;
  source_path: string;
  completed_at?: string;
  archived_at?: string;
}

const DEMO_LIVE_KEY = "selah-demo-live-session";
function emptyDemoLiveSession(): LiveSessionSnapshot {
  return {
    active: false,
    course: null,
    started_at: null,
    transcript_lines: [],
    pending_lines: [],
    summaries: [],
  };
}

function loadDemoLiveSession(): LiveSessionSnapshot {
  if (!_isDemo()) return emptyDemoLiveSession();
  try {
    const raw = localStorage.getItem(DEMO_LIVE_KEY);
    if (!raw) return emptyDemoLiveSession();
    const parsed = JSON.parse(raw) as Partial<LiveSessionSnapshot>;
    const snapshot: LiveSessionSnapshot = {
      session_id: typeof parsed.session_id === "string" ? parsed.session_id : null,
      active: parsed.active === true,
      course: parsed.course ?? null,
      started_at: parsed.started_at ?? null,
      transcript_lines: Array.isArray(parsed.transcript_lines) ? parsed.transcript_lines : [],
      pending_lines: Array.isArray(parsed.pending_lines) ? parsed.pending_lines : [],
      summaries: Array.isArray(parsed.summaries) ? parsed.summaries : [],
    };
    if (snapshot.active && !snapshot.session_id) {
      snapshot.session_id = crypto.randomUUID();
      saveDemoLiveSession(snapshot);
    }
    return snapshot;
  } catch {
    return emptyDemoLiveSession();
  }
}

function saveDemoLiveSession(snapshot: LiveSessionSnapshot): LiveSessionSnapshot {
  if (_isDemo()) {
    try { localStorage.setItem(DEMO_LIVE_KEY, JSON.stringify(snapshot)); } catch {}
  }
  return snapshot;
}

function demoLiveCourseMatches(a: LiveCourseInfo | null, b: LiveCourseInfo | null): boolean {
  if (!a || !b) return false;
  return a.course_name === b.course_name && a.day === b.day && a.period === b.period;
}

function buildDemoLiveSummaries(lines: LiveTranscriptLine[]): LiveSummaryChunk[] {
  if (lines.length === 0) return [];
  const recent = lines.slice(-3).map((line) => line.text).join(" / ");
  return [{
    title: "デモ用要約",
    range_label: "最近",
    body: `### 全体要約\n${recent || "このセッションでは授業内容の要点がまとめられます。"}\n\n### 次に見るポイント\n- キーワードを 2〜3 個に絞って見返す\n- 宿題や小テストに関係する箇所を先に確認する`,
    line_count: lines.length,
    terms: [
      {
        term: "メタ認知",
        explanation: "自分の理解度や学習方法を客観的に確認する考え方。復習時は、何が分かっていて何が曖昧かを分けて見る観点になる。",
        source_excerpt: "キーワードを短くメモし、あとで見返しやすい形に整理",
        external_source: "Flavell, J. H. (1979), Metacognition and cognitive monitoring, American Psychologist",
      },
      {
        term: "想起練習",
        explanation: "資料を眺めるだけでなく、覚えている内容を自分で思い出す復習方法。小テスト対策では、要点を閉じた状態で説明できるかを確認する。",
        source_excerpt: "課題や小テストにつながるポイント",
        external_source: "Roediger, H. L. & Karpicke, J. D. (2006), Test-enhanced learning, Psychological Science",
      },
    ],
    whiteboard: {
      title: "知識整理の流れ",
      layout: "flow",
      nodes: [
        { id: "n1", label: "キーワード", detail: "短く拾う", node_type: "structure", kind: "core", role: "main", source_type: "lecture", source_excerpt: "重要語を先に拾う" },
        { id: "n2", label: "理解確認", detail: "説明できるか", node_type: "structure", kind: "support", role: "branch", parent_id: "n1", source_type: "lecture", source_excerpt: "自分の言葉で説明" },
        { id: "n3", label: "想起練習", detail: "外部補足: 記憶定着の方法", node_type: "term", kind: "support", role: "branch", parent_id: "n1", source_type: "external", external_source: "Roediger & Karpicke (2006), Psychological Science" },
        { id: "n4", label: "課題接続", detail: "提出物へつなぐ", node_type: "structure", kind: "result", role: "main", source_type: "lecture", source_excerpt: "課題や小テストにつながるポイント" },
      ],
      edges: [
        { from: "n1", to: "n2", label: "整理" },
        { from: "n1", to: "n3", label: "" },
        { from: "n2", to: "n4", label: "活用" },
      ],
    },
  }];
}

function buildDemoLiveTranscript(course: LiveCourseInfo): LiveTranscriptLine[] {
  const now = new Date();
  const at = (offsetMin: number) =>
    new Date(now.getTime() + offsetMin * 60_000).toLocaleTimeString("ja-JP", {
      hour: "2-digit",
      minute: "2-digit",
    });
  const name = course.course_name || "自由ノート";
  return [
    { at: at(0), text: `${name} のデモセッションを開始しました。今日のテーマと到達目標を確認します。` },
    { at: at(2), text: "授業で強調されたキーワードを短くメモし、あとで見返しやすい形に整理します。" },
    { at: at(4), text: "課題や小テストにつながるポイントを先に押さえておくと復習が楽になります。" },
  ];
}

export async function liveGetSurface(): Promise<LiveSurfaceSnapshot> {
  if (_isDemo()) return liveSurfaceSnapshot(loadDemoLiveSession());
  return expandLiveSurface(await invoke<CompactLiveSurfaceSnapshot>("live_get_surface_compact"));
}

export async function livePeekDaySurface(course: LiveCourseInfo): Promise<LiveSurfaceSnapshot> {
  if (_isDemo()) return liveSurfaceSnapshot(await livePeekDayCache(course));
  return expandLiveSurface(await invoke<CompactLiveSurfaceSnapshot>("live_peek_day_surface_compact", { course }));
}

export async function liveStartSurface(course: LiveCourseInfo): Promise<LiveSurfaceSnapshot> {
  if (_isDemo()) return liveSurfaceSnapshot(await liveStartSession(course));
  return expandLiveSurface(await invoke<CompactLiveSurfaceSnapshot>("live_start_surface_compact", { course }));
}

export async function liveGetSession(): Promise<LiveSessionSnapshot> {
  if (_isDemo()) return loadDemoLiveSession();
  return invoke<LiveSessionSnapshot>("live_get_session");
}

/** Startup needs only activity, not the full recording history. */
export async function liveHasActiveSession(): Promise<boolean> {
  if (_isDemo()) return loadDemoLiveSession().active;
  return invoke<boolean>("live_has_active_session");
}

export async function livePeekDayCache(course: LiveCourseInfo): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    return demoLiveCourseMatches(snapshot.course, course) ? snapshot : emptyDemoLiveSession();
  }
  return invoke<LiveSessionSnapshot>("live_peek_day_cache", { course });
}

export async function liveStartSession(course: LiveCourseInfo): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    if (loadDemoLiveSession().active) throw new Error("Liveセッションが使用中です");
    const transcript_lines = buildDemoLiveTranscript(course);
    return saveDemoLiveSession({
      session_id: crypto.randomUUID(),
      active: true,
      course,
      started_at: new Date().toISOString(),
      transcript_lines,
      pending_lines: [],
      summaries: buildDemoLiveSummaries(transcript_lines),
    });
  }
  return invoke<LiveSessionSnapshot>("live_start_session", { course });
}

export async function liveAppendTranscript(text: string): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    if (!snapshot.active || !text.trim()) return snapshot;
    const next: LiveSessionSnapshot = {
      ...snapshot,
      transcript_lines: [
        ...snapshot.transcript_lines,
        {
          text: text.trim(),
          at: new Date().toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" }),
        },
      ],
      pending_lines: [],
    };
    return saveDemoLiveSession(next);
  }
  return invoke<LiveSessionSnapshot>("live_append_transcript", { text });
}

export async function liveFlushSummary(force: boolean = false): Promise<LiveSessionSnapshot> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    if (!force && snapshot.transcript_lines.length === 0) return snapshot;
    const next = {
      ...snapshot,
      summaries: buildDemoLiveSummaries(snapshot.transcript_lines),
    };
    return saveDemoLiveSession(next);
  }
  return invoke<LiveSessionSnapshot>("live_flush_summary", { force });
}

function requireDemoLiveOwner(sessionId: string): void {
  const snapshot = loadDemoLiveSession();
  if (!sessionId || !snapshot.active || snapshot.session_id !== sessionId) {
    throw new Error("Liveセッションが切り替わりました");
  }
}

export async function liveGenerateOverallSummary(sessionId: string): Promise<string> {
  if (_isDemo()) {
    requireDemoLiveOwner(sessionId);
    const snapshot = await liveFlushSummary(true);
    requireDemoLiveOwner(sessionId);
    const content = snapshot.transcript_lines.map((line) => line.text).join(" / ");
    return `### 全体要約\n${content || "このセッションの内容はまだありません。"}\n\n### 今回の論点\n- 現在までの文字起こし全体を対象に生成したデモ要約`;
  }
  return invoke<string>("live_generate_overall_summary", { sessionId });
}

export async function liveCancelSession(sessionId: string): Promise<void> {
  if (_isDemo()) {
    requireDemoLiveOwner(sessionId);
    saveDemoLiveSession(emptyDemoLiveSession());
    return;
  }
  return invoke<void>("live_cancel_session", { sessionId });
}

export async function liveClearDayCache(course: LiveCourseInfo): Promise<void> {
  if (_isDemo()) {
    const snapshot = loadDemoLiveSession();
    if (demoLiveCourseMatches(snapshot.course, course)) {
      saveDemoLiveSession(emptyDemoLiveSession());
    }
    return;
  }
  return invoke<void>("live_clear_day_cache", { course });
}

export async function liveFinishSession(sessionId: string): Promise<LiveSaveResult> {
  if (_isDemo()) {
    requireDemoLiveOwner(sessionId);
    const snapshot = await liveFlushSummary(true);
    requireDemoLiveOwner(sessionId);
    const saved = snapshot.transcript_lines.length > 0;
    const markdown = saved
      ? `# ${snapshot.course?.course_name ?? "LIVE Demo"}\n\n${snapshot.summaries.map((chunk) => chunk.body).join("\n\n")}\n\n## Transcript\n${snapshot.transcript_lines.map((line) => `- ${line.at} ${line.text}`).join("\n")}`
      : "";
    const result: LiveSaveResult = {
      saved,
      path: saved ? `/DemoNotes/${(snapshot.course?.course_name ?? "live-demo").replace(/[^\w\u3040-\u30ff\u4e00-\u9faf-]+/g, "_")}.md` : "",
      markdown,
      snapshot: { ...snapshot, active: false, summarizing: false, next_summary_at_ms: null, finish_phase: null },
    };
    saveDemoLiveSession(emptyDemoLiveSession());
    return result;
  }
  return invoke<LiveSaveResult>("live_finish_session", { sessionId });
}

export async function liveFinishSurface(sessionId: string): Promise<LiveSurfaceSaveResult> {
  if (_isDemo()) return liveSurfaceSaveResult(await liveFinishSession(sessionId));
  return expandLiveSurfaceSave(await invoke<CompactLiveSurfaceSaveResult>("live_finish_surface_compact", { sessionId }));
}
