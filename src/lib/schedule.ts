import { PERIOD_TIMES, type LunaCourseRow, type ScheduleResponse } from "./types";

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

export function lunaTermCode(lunaId: string): string | null {
  if (lunaId.length < 14) return null;
  const code = lunaId.slice(12, 14);
  return /^\d{2}$/.test(code) ? code : null;
}

export function lunaIdYear(lunaId: string): string | null {
  if (lunaId.length < 4) return null;
  const year = lunaId.slice(0, 4);
  return /^\d{4}$/.test(year) ? year : null;
}

export function preferredLunaCourse(
  courses: LunaCourseRow[],
  day: number,
  period: number,
  term: string,
  year = "",
): LunaCourseRow | undefined {
  const matches = courses.filter((course) => course.day === day && course.period === period);
  if (matches.length === 0) return undefined;
  const inYear = matches.filter((course) => {
    if (!year) return true;
    const idYear = lunaIdYear(course.luna_id);
    return !idYear || idYear === year;
  });
  if (inYear.length === 0) return undefined;
  if (!term) return inYear[0];
  return (
    inYear.find((course) => lunaTermCode(course.luna_id) === term) ??
    inYear.find((course) => {
      const code = lunaTermCode(course.luna_id);
      return code !== "02" && code !== "03";
    })
  );
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
  const term = schedule.luna_term || "";
  const year = schedule.luna_year || "";
  if (kgc.length === 0) {
    const seen = new Set<string>();
    const slots: CourseSlot[] = [];
    for (const course of luna) {
      const picked = preferredLunaCourse(luna, course.day, course.period, term, year);
      if (!picked || picked.luna_id !== course.luna_id || picked.day !== course.day || picked.period !== course.period) continue;
      const key = picked.day + ":" + picked.period + ":" + picked.luna_id;
      if (seen.has(key)) continue;
      seen.add(key);
      slots.push({
        name: picked.name,
        kgc_code: "",
        day: picked.day,
        period: picked.period,
        room: "",
        detail_path: "",
        is_cancelled: false,
        luna_id: picked.luna_id,
        teacher: picked.teacher,
      });
    }
    return slots;
  }
  return kgc.map((entry) => {
    const lunaCourse = preferredLunaCourse(luna, entry.day, entry.period, term, year);
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
