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
}

export interface LunaNotification {
  date: string;
  course_info: string;
  module: string;
  content: string;
  url: string;
  idnumber: string;
}

export interface LunaCourse {
  idnumber: string;
  name: string;
  teacher: string;
  period: number;
  day: number;
}

export interface LunaCommunity {
  idnumber: string;
  name: string;
}

// ============ Period Times ============

export interface PeriodTime {
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
