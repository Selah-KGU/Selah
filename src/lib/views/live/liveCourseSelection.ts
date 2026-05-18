import type { LiveCourseInfo } from "../../api";
import type { CourseSlot } from "../../schedule";
import { getHeroCourses } from "../../schedule";
import { DAY_NUM_LABELS, PERIOD_TIMES } from "../../types";

const FREE_NOTE_NAME = "自由ノート";

export function courseKey(course: CourseSlot): string {
  return `${course.day}-${course.period}-${course.kgc_code || course.name}`;
}

export function courseLabel(course: CourseSlot): string {
  const time = PERIOD_TIMES[course.period];
  const day = DAY_NUM_LABELS[course.day] ?? `${course.day}`;
  const timeLabel = time ? `${time.start}-${time.end}` : `${course.period}限`;
  const meta = [day, `${course.period}限`, timeLabel].filter(Boolean).join(" ");
  return `${course.name} (${meta})`;
}

export function toLiveCourse(course: CourseSlot): LiveCourseInfo {
  const time = PERIOD_TIMES[course.period];
  return {
    course_name: course.name,
    course_code: course.kgc_code,
    room: course.room,
    teacher: course.teacher,
    day: course.day,
    period: course.period,
    time_label: time ? `${time.start}-${time.end}` : "",
    is_free_note: false,
  };
}

export function createFreeNoteCourse(): LiveCourseInfo {
  return {
    course_name: FREE_NOTE_NAME,
    course_code: "",
    room: "",
    teacher: "",
    day: 0,
    period: 0,
    time_label: "",
    is_free_note: true,
  };
}

function todayDayNumber(date: Date): number {
  const jsDow = date.getDay();
  return jsDow === 0 ? 7 : jsDow;
}

function closestCourseForNow(courses: CourseSlot[], date: Date): CourseSlot | null {
  if (!courses.length) return null;
  const nowMin = date.getHours() * 60 + date.getMinutes();
  const ranked = courses
    .map((course) => {
      const time = PERIOD_TIMES[course.period];
      if (!time) return { course, distance: Number.MAX_SAFE_INTEGER, startMin: Number.MAX_SAFE_INTEGER };
      const startMin = time.startH * 60 + time.startM;
      const endMin = time.endH * 60 + time.endM;
      let distance = 0;
      if (nowMin < startMin) distance = startMin - nowMin;
      else if (nowMin > endMin) distance = nowMin - endMin;
      return { course, distance, startMin };
    })
    .sort((a, b) => a.distance - b.distance || a.startMin - b.startMin);
  return ranked[0]?.course ?? null;
}

export function defaultCourseForVisibleOptions(courses: CourseSlot[], date: Date): CourseSlot | null {
  if (!courses.length) return null;
  const visibleDay = courses[0]?.day;
  if (visibleDay == null) return courses[0] ?? null;
  if (visibleDay === todayDayNumber(date)) {
    return closestCourseForNow(courses, date) ?? courses[0] ?? null;
  }
  return [...courses].sort((a, b) => a.period - b.period || a.name.localeCompare(b.name))[0] ?? null;
}

export function chooseFocusedCourseOptions(courses: CourseSlot[], date: Date): CourseSlot[] {
  if (!courses.length) return [];
  const today = todayDayNumber(date);
  const nowMin = date.getHours() * 60 + date.getMinutes();

  const grouped = new Map<number, CourseSlot[]>();
  for (const course of courses) {
    if (!grouped.has(course.day)) grouped.set(course.day, []);
    grouped.get(course.day)!.push(course);
  }
  for (const list of grouped.values()) {
    list.sort((a, b) => a.period - b.period || a.name.localeCompare(b.name));
  }

  const todayCourses = grouped.get(today) ?? [];
  const hasRemainingToday = todayCourses.some((course) => {
    const time = PERIOD_TIMES[course.period];
    if (!time) return false;
    const endMin = time.endH * 60 + time.endM;
    return nowMin <= endMin;
  });
  if (todayCourses.length > 0 && hasRemainingToday) {
    return todayCourses;
  }

  for (let offset = 1; offset <= 7; offset++) {
    const day = ((today - 1 + offset) % 7) + 1;
    const nextCourses = grouped.get(day) ?? [];
    if (nextCourses.length > 0) return nextCourses;
  }

  return todayCourses.length > 0
    ? todayCourses
    : [...courses].sort((a, b) => a.day - b.day || a.period - b.period || a.name.localeCompare(b.name));
}

export function defaultSelectedCourseKey(courses: CourseSlot[], date: Date): string {
  const nearest = defaultCourseForVisibleOptions(courses, date);
  if (nearest) return courseKey(nearest);
  const hero = getHeroCourses(courses, date);
  return hero[0] ? courseKey(hero[0].entry) : (courses[0] ? courseKey(courses[0]) : "");
}
