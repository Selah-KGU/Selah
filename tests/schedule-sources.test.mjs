import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { buildCourseSlots, supplementalKgcCourse } = await loadTypeScript("src/lib/schedule.ts");
const luna = (name, day = 1, period = 1) => ({ name, day, period, luna_id: "2026000000000301", teacher: "Luna teacher" });
const kgc = (name, day = 1, period = 1, extra = {}) => ({
  name, day, period, kgc_code: "kgc", room: "A101", detail_path: "/kgc/detail", is_cancelled: false, is_makeup: false, ...extra,
});
const schedule = (courses, entries) => ({ luna_year: "2026", luna_term: "03", raw: { luna_courses: courses, kgc_entries_current: entries } });

test("Luna courses remain complete when KGC is absent or has only part of the timetable", () => {
  const courses = [luna("First"), luna("Second", 2, 2)];
  for (const entries of [[], [kgc("First")]]) {
    const slots = buildCourseSlots(schedule(courses, entries));
    assert.deepEqual(slots.map(course => course.name), ["First", "Second"]);
    assert.ok(slots.every(course => course.luna_id));
  }
});

test("conflicting KGC data cannot rename or cancel a Luna course", () => {
  const slots = buildCourseSlots(schedule([luna("Current Luna")], [kgc("Old KGC", 1, 1, { is_cancelled: true })]));
  assert.equal(slots.length, 1);
  assert.equal(slots[0].name, "Current Luna");
  assert.equal(slots[0].is_cancelled, false);
  assert.equal(slots[0].room, "");
  assert.equal(slots[0].teacher, "Luna teacher");
});

test("matching KGC data supplements room and cancellation while preserving Luna identity", () => {
  const slots = buildCourseSlots(schedule([luna("データ Ａ")], [kgc("データA", 1, 1, { is_cancelled: true })]));
  assert.equal(slots[0].name, "データ Ａ");
  assert.equal(slots[0].room, "A101");
  assert.equal(slots[0].is_cancelled, true);
  assert.equal(slots[0].kgc_code, "kgc");
  assert.equal(slots[0].luna_id, "2026000000000301");
});

test("a moved Luna course is not duplicated at its stale KGC time", () => {
  const courses = [luna("Moved", 2, 3)];
  const entries = [kgc("Moved", 1, 1)];
  assert.deepEqual(buildCourseSlots(schedule(courses, entries)).map(course => [course.day, course.period]), [[2, 3]]);
  assert.equal(supplementalKgcCourse(entries, courses, 1, 1, "03", "2026"), undefined);
});

test("KGC still supplies makeup classes and courses missing from Luna", () => {
  const slots = buildCourseSlots(schedule([luna("Regular")], [
    kgc("Regular", 2, 2, { is_makeup: true }), kgc("KGC only", 3, 3),
  ]));
  assert.deepEqual(slots.map(course => [course.name, course.day]), [["Regular", 1], ["Regular", 2], ["KGC only", 3]]);
});

test("prior semesters and duplicate Luna rows do not displace the current course", () => {
  const current = luna("Current");
  const old = { ...luna("Old"), luna_id: "2025000000000301" };
  const slots = buildCourseSlots(schedule([old, current, { ...current }], []));
  assert.deepEqual(slots.map(course => course.name), ["Current"]);
});
