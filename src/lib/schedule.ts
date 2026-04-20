import type { ScheduleResponse } from "./types";
import { PERIOD_TIMES } from "./types";

export interface CourseSlot {
  name: string;
  kgc_code: string;
  day: number;
  period: number;
  room: string;
  detail_path: string;
  is_cancelled: boolean;
  luna_id: string;
  teacher: string;
}

export interface HeroCourse {
  entry: CourseSlot;
  time: (typeof PERIOD_TIMES)[number];
  live: boolean;
}

export function buildCourseSlots(schedule: ScheduleResponse | null): CourseSlot[] {
  if (!schedule) return [];
  const kgc = schedule.raw.kgc_entries_current;
  const luna = schedule.raw.luna_courses;
  return kgc.map((entry) => {
    const lunaCourse = luna.find((course) => course.day === entry.day && course.period === entry.period);
    return {
      name: entry.name,
      kgc_code: entry.kgc_code,
      day: entry.day,
      period: entry.period,
      room: entry.room,
      detail_path: entry.detail_path,
      is_cancelled: entry.is_cancelled,
      luna_id: lunaCourse?.luna_id ?? "",
      teacher: lunaCourse?.teacher ?? "",
    };
  });
}

export function getHeroCourses(entries: CourseSlot[], now: Date): HeroCourse[] {
  if (!entries.length) return [];
  const jsDow = now.getDay();
  const todayDay = jsDow === 0 ? 7 : jsDow;
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const todayClasses = entries
    .filter((entry) => entry.day === todayDay && !entry.is_cancelled)
    .sort((a, b) => a.period - b.period);

  const result: HeroCourse[] = [];
  for (const entry of todayClasses) {
    const time = PERIOD_TIMES[entry.period];
    if (!time) continue;
    const startMin = time.startH * 60 + time.startM;
    const endMin = time.endH * 60 + time.endM;
    if (nowMin < endMin) {
      result.push({ entry, time, live: nowMin >= startMin });
      if (result.length >= 2) break;
    }
  }
  return result;
}
