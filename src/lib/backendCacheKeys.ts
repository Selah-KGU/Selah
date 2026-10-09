/** UI keys whose SQLite cache names differ. Shared by cold reads and sync. */
export const BACKEND_CACHE_DB_KEYS: Record<string, string> = {
  exams: "exam_timetable",
};
