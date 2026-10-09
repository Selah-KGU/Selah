import test from "node:test";
import assert from "node:assert/strict";
import { loadTypeScript } from "./load-typescript.mjs";

const { applyLiveFinishProgress, isLiveBusy, LIVE_FINISH_LABELS, liveSavePresentation } = await loadTypeScript("src/lib/views/live/liveFinish.ts");
const { liveSurfaceSnapshot, mergeLiveSnapshot } = await loadTypeScript("src/lib/views/live/liveTranscript.ts");
const fullSession = () => ({
  session_id: "recording-a", active: true, course: null, started_at: "2026-10-07 10:00:00",
  transcript_lines: [{ text: "speech", at: "10:00:01" }], pending_lines: [], summaries: [],
  finish_phase: null, finish_revision: 0,
  summarizing: false, next_summary_at_ms: null,
});
const session = () => liveSurfaceSnapshot(fullSession());
const progress = (finish_phase, finish_revision, session_id = "recording-a") => ({ session_id, finish_phase, finish_revision });

test("a reloaded page recovers every save stage and blocks operations without a local promise", () => {
  for (const [index, phase] of ["stopping", "saving_record", "summarizing", "saving_final"].entries()) {
    const recovered = JSON.parse(JSON.stringify({ ...fullSession(), finish_phase: phase, finish_revision: index + 1 }));
    assert.equal(isLiveBusy(recovered, false), true);
    const presentation = liveSavePresentation(recovered);
    assert.equal(presentation.steps.length, 0);
    assert.ok(presentation.label.startsWith(LIVE_FINISH_LABELS[phase]));
  }
  assert.equal(isLiveBusy(session(), true), true);
  assert.equal(isLiveBusy(session(), false), false);
  assert.equal(liveSavePresentation(session()), null);
});

test("a short recording goes directly from record saving to final saving without an invented AI step", () => {
  const record = applyLiveFinishProgress(session(), progress("saving_record", 2));
  const final = applyLiveFinishProgress(record, progress("saving_final", 3));
  const presentation = liveSavePresentation(final);
  assert.equal(presentation.label, `${LIVE_FINISH_LABELS.saving_final}…`);
  assert.deepEqual(presentation.steps, []);
  assert.equal(presentation.label.includes("AI"), false);
});

test("an older recovery read retains the current save stage while accepting new speech and summaries", () => {
  const current = applyLiveFinishProgress(session(), progress("saving_final", 4));
  const older = { ...fullSession(), finish_phase: "saving_record", finish_revision: 2,
    summarizing: true, next_summary_at_ms: 123,
    transcript_lines: [...current.visible_lines, { text: "decoder tail", at: "10:00:02" }],
    summaries: [{ body: "summary" }] };
  const merged = mergeLiveSnapshot(current, older);
  assert.equal(merged.finish_phase, "saving_final");
  assert.equal(merged.finish_revision, 4);
  assert.equal(merged.summarizing, false);
  assert.equal(merged.next_summary_at_ms, null);
  assert.equal(merged.transcript_line_count, 2);
  assert.equal(merged.summaries, older.summaries);
});

test("failure unlocks retry and late progress from the first attempt cannot regress it", () => {
  const saving = applyLiveFinishProgress(session(), progress("saving_final", 4));
  const failed = mergeLiveSnapshot(saving, { ...fullSession(), finish_revision: 5 });
  assert.equal(isLiveBusy(failed, false), false);
  assert.equal(liveSavePresentation(failed), null);
  assert.equal(applyLiveFinishProgress(failed, progress("summarizing", 3)), failed);
  const retry = applyLiveFinishProgress(failed, progress("stopping", 6));
  assert.equal(applyLiveFinishProgress(retry, progress("saving_final", 4)), retry);
  assert.equal(mergeLiveSnapshot(retry, { ...fullSession(), finish_revision: failed.finish_revision }).finish_phase, "stopping");
  assert.equal(isLiveBusy(retry, false), true);
});

test("save progress cannot contaminate a replacement or completed recording and preserves heavy array references", () => {
  const current = session();
  const saving = applyLiveFinishProgress(current, progress("saving_record", 2));
  assert.equal(saving.visible_lines, current.visible_lines);
  assert.equal(saving.summaries, current.summaries);
  assert.equal(applyLiveFinishProgress(current, progress("saving_final", 4, "old-recording")), current);
  const completed = { ...session(), active: false };
  assert.equal(applyLiveFinishProgress(completed, progress("saving_final", 4)), completed);
  assert.equal(isLiveBusy(completed, false), false);
});
