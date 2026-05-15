// ============ Shared Types ============
// Consolidated from HomePage, LunaTodo, NotificationsUnified, Timetable, trayStatus

export interface LunaTodoItem {
  course_name: string;
  content_type: string;
  content_name: string;
  url: string;
  deadline: string;
  status: string;
  feedback: string;
  source?: string;
  local_id?: string;
  source_path?: string;
  source_excerpt?: string;
}

export interface LunaNotification {
  date: string;
  course_info: string;
  module: string;
  content: string;
  url: string;
  idnumber: string;
}

interface LunaCommunity {
  idnumber: string;
  name: string;
}

// ============ Period Times ============

interface PeriodTime {
  start: string;
  end: string;
  startH: number;
  startM: number;
  endH: number;
  endM: number;
}

export const PERIOD_TIMES: Record<number, PeriodTime> = {
  1: { start: "9:00",  end: "10:30", startH: 9, startM: 0,  endH: 10, endM: 30 },
  2: { start: "11:00", end: "12:30", startH: 11, startM: 0,  endH: 12, endM: 30 },
  3: { start: "13:30", end: "15:00", startH: 13, startM: 30, endH: 15, endM: 0 },
  4: { start: "15:10", end: "16:40", startH: 15, startM: 10, endH: 16, endM: 40 },
  5: { start: "16:50", end: "18:20", startH: 16, startM: 50, endH: 18, endM: 20 },
};

export const DAY_LABELS = ["日", "月", "火", "水", "木", "金", "土"] as const;

// Day number (1-6) to label
export const DAY_NUM_LABELS: Record<number, string> = {
  1: "月", 2: "火", 3: "水", 4: "木", 5: "金", 6: "土",
};

// ============ Common Types ============

interface SelectOption {
  value: string;
  label: string;
  selected: boolean;
}

// ============ Schedule (AI-driven timetable) ============

interface LunaCountsData {
  announcements: number;
  new_announcements: number;
  reports: number;
  exams: number;
  discussions: number;
}

interface SessionPlanData {
  session_num: number;
  topic: string;
  delivery_mode: string;
  study_outside: string;
}

export interface KgcCourseRow {
  id: number;
  kgc_code: string;
  name: string;
  day: number;
  period: number;
  room: string;
  detail_path: string;
  is_cancelled: boolean;
  is_makeup: boolean;
  is_room_changed: boolean;
  week_label: string;
}

export interface LunaCourseRow {
  id: number;
  luna_id: string;
  name: string;
  teacher: string;
  day: number;
  period: number;
}

export interface AiScheduleItem {
  day: number;
  period: number;
  course_name: string;
  delivery_mode: string;
  room: string;
  teacher: string;
  session_topic: string;
  is_cancelled: boolean;
  notifications: string[];
  assignments: string[];
  exams: string[];
}

export interface AiScheduleResult {
  current_week_label: string;
  next_week_label: string;
  current_week: AiScheduleItem[];
  next_week: AiScheduleItem[];
  weekly_summary: string;
  cross_week_insights: string;
}

export interface AiTodoTaskGuide {
  task_name: string;
  course_name: string;
  deadline: string;
  urgency: "overdue" | "critical" | "soon" | "normal";
  background: string;
  live_note_summary: string;
  study_hints: string[];
  ready_to_use_label: string;
  ready_to_use: string;
  estimated_minutes: number;
}

export interface AiTodoDailyPlan {
  label: string;
  tasks: string[];
  free_hours: number;
}

export interface AiTodoAnalysis {
  task_guides: AiTodoTaskGuide[];
  daily_plan: AiTodoDailyPlan[];
  advice: string;
}

export interface ScheduleRawData {
  kgc_entries_current: KgcCourseRow[];
  kgc_entries_next: KgcCourseRow[];
  luna_courses: LunaCourseRow[];
  session_plans: [string, SessionPlanData[]][];
  luna_counts: [string, LunaCountsData][];
  luna_activities: LunaActivityItem[];
  kgc_course_details: KgcCourseDetailItem[];
  current_week_label: string;
  next_week_label: string;
  luna_communities: LunaCommunity[];
}

export interface LunaActivityItem {
  luna_id: string;
  activity_type: string;
  title: string;
  period: string;
  status: string;
  detail_path?: string;
}

export interface KgcCourseDetailItem {
  kgc_code: string;
  fields: [string, string][];
  delivery_mode: string;
}

export interface ScheduleResponse {
  raw: ScheduleRawData;
  ai_result: AiScheduleResult | null;
  ai_stale: boolean;
  snapshot_updated_at: number;
  luna_communities: LunaCommunity[];
  luna_year_options: SelectOption[];
  luna_term_options: SelectOption[];
  luna_year: string;
  luna_term: string;
}
