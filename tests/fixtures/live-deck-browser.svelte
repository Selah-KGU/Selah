<script lang="ts">
  import { onMount, tick } from "svelte";
  import LiveRightRail from "../../src/lib/views/live/LiveRightRail.svelte";
  import { renderMd } from "../../src/lib/views/live/liveMarkdown";
  import DisplayClock from "test-live-display-clock";
  import TranscriptFollow from "test-live-transcript-follow";

  const body = (prefix: string, count: number) => Array.from({ length: count }, (_, i) => `- ${prefix} ${i} **Unicode 🙂**`).join("\n");
  const chunk = (prefix: string, count: number) => ({ range_label: "10:00–10:15", body: body(prefix, count), isOverall: false });
  let entries = $state.raw([chunk("要点", 12)]);
  let terms = $state.raw(Array.from({ length: 12 }, (_, i) => ({ term: `術語 ${i}`, explanation: `完全な説明 ${i}` })));
  let front = $state(0);
  let transcriptTick = $state(0);
  let surfaceVisible = $state(true);
  let covered = $state(false);
  let clockSnapshot = $state.raw({ active: true, lineCount: 0 });
  let followSnapshot = $state.raw({ active: true, session_id: "recording-a", transcript_line_count: 20 });
  let followListening = $state(true), followAuto = $state(true);
  let opens = 0;
  let renders = 0;
  const noop = () => {};
  const renderer = (text: string) => { renders++; return renderMd(text); };

  onMount(() => {
    void run();
  });

  async function run() {
    const probe = (window as any).__deckProbe;
    const checks: string[] = [];
    const check = (condition: unknown, label: string) => {
      if (!condition) throw new Error(label);
      checks.push(label);
    };
    const query = (selector: string) => document.querySelector<HTMLElement>(selector)!;
    const text = (selector: string) => query(selector)?.textContent?.trim();
    try {
      await tick();
      check(document.querySelectorAll(".summary-pt-card").length === 3, "summary mounts three cards");
      check(document.querySelectorAll(".term-card").length === 3, "terms mount three cards");
      check(text(".summary-pt-card.front")?.startsWith("要点 0"), "initial summary front");
      check(text(".term-card.active .term-card-term") === "術語 0", "initial term front");
      check(getComputedStyle(query(".summary-pt-card.front")).position === "relative", "front keeps layout styling");
      check(getComputedStyle(query(".term-card.peek")).pointerEvents === "auto", "back term remains clickable");
      check(probe.timerCount() === 1, "one summary rotation timer");
      check(probe.timerCount(30_000) === 1, "one visible LIVE display clock");
      check(text(".live-clock-probe") === "1000000", "visible clock refreshes immediately");
      const timerIds = probe.timerIds().join(",");
      const clockIds = probe.timerIds(30_000).join(",");
      const renderedBefore = renders;
      for (let i = 0; i < 100; i++) {
        transcriptTick++;
        clockSnapshot = { ...clockSnapshot, lineCount: transcriptTick };
        await tick();
      }
      check(renders === renderedBefore, "unrelated parent updates do not render summary Markdown");
      check(probe.timerIds().join(",") === timerIds, "unrelated updates do not restart rotation");
      check(probe.timerIds(30_000).join(",") === clockIds, "speech snapshots do not restart display clock");
      const queuedRotation = probe.timerCallbacks()[0], queuedClock = probe.timerCallbacks(30_000)[0];
      const beforeHide = text(".summary-pt-card.front");
      surfaceVisible = false; await tick();
      check(probe.timerCount() === 0 && probe.timerCount(30_000) === 0, "hidden surface releases both display timers");
      const beforeHiddenRender = renders;
      probe.setTime(2000000);
      queuedRotation(); queuedClock(); probe.step(); probe.step(30_000); await tick();
      check(text(".summary-pt-card.front") === beforeHide && renders === beforeHiddenRender, "queued hidden rotation cannot update cards");
      check(text(".live-clock-probe") === "1000000", "queued hidden clock cannot publish");
      surfaceVisible = true; await tick();
      check(probe.timerCount() === 1 && probe.timerCount(30_000) === 1, "restored surface creates one timer of each kind");
      check(text(".live-clock-probe") === "2000000", "restored surface refreshes current time without waiting");
      check(text(".summary-pt-card.front") === beforeHide, "restored surface keeps current point");
      queuedRotation(); queuedClock(); await tick();
      check(text(".summary-pt-card.front") === beforeHide, "retired timer cannot rotate the restored surface");
      covered = true; await tick();
      check(probe.timerCount() === 0 && probe.timerCount(30_000) === 1, "detail overlay stops only covered rail rotation");
      covered = false; await tick();
      check(probe.timerCount() === 1, "closing detail overlay resumes current point rotation");
      for (let i = 1; i <= 24; i++) {
        probe.step();
        await tick();
        check(text(".summary-pt-card.front")?.startsWith(`要点 ${i % 12} `), `rotation ${i} reaches original headline`);
        check(document.querySelectorAll(".summary-pt-card").length === 3, `rotation ${i} keeps three cards`);
      }
      probe.step(); await tick();
      entries = [...entries]; await tick();
      check(text(".summary-pt-card.front")?.startsWith("要点 1 "), "equivalent entries preserve rotation position");
      const equivalentTimer = probe.timerIds().join(",");
      entries = [...entries]; await tick();
      check(probe.timerIds().join(",") === equivalentTimer, "equivalent entries retain their existing rotation timer");
      query('.term-stack-arrow[aria-label="次"]').click(); await tick();
      check(front === 1 && text(".term-card.active .term-card-term") === "術語 1", "next term uses original index");
      query('.term-stack-arrow[aria-label="前"]').click(); await tick();
      check(front === 0, "previous term returns to original front");
      query('.term-stack-arrow[aria-label="前"]').click(); await tick();
      check(front === 11 && text(".term-stack-counter") === "12/12", "previous wraps to final term");
      query('.term-card.peek').click(); await tick();
      check(front === 0 && text(".term-card.active .term-card-term") === "術語 0", "back card click preserves original index at wrap");
      query('.term-card.active').click();
      query('.summary-stack').dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      check(opens === 2, "term click and summary keyboard retain detail actions");
      entries = [chunk("新要点", 4)]; await tick();
      check(text(".summary-pt-card.front")?.startsWith("新要点 0 "), "new summary resets to its first point");
      probe.step(); await tick();
      check(text(".summary-pt-card.front")?.startsWith("新要点 1 "), "new summary keeps rotation");
      entries = []; terms = []; await tick();
      check(!document.querySelector(".right-rail"), "cleared content unmounts the rail");
      check(probe.timerCount() === 0, "cleared content removes rotation timer");
      entries = [chunk("再開", 5)]; await tick();
      check(document.querySelectorAll(".summary-pt-card").length === 3, "remount restores three cards");
      check(probe.timerCount() === 1, "remount creates one timer");
      clockSnapshot = { ...clockSnapshot, active: false }; await tick();
      check(probe.timerCount(30_000) === 0, "inactive recording stops display clock");
      probe.setTime(3000000); clockSnapshot = { ...clockSnapshot, active: true }; await tick();
      check(probe.timerCount(30_000) === 1 && text(".live-clock-probe") === "3000000", "active recording restarts clock with fresh time");

      const transcript = query(".live-follow-probe");
      const atBottom = () => Math.abs(transcript.scrollHeight - transcript.clientHeight - transcript.scrollTop) <= 1;
      check(probe.frameCount() === 1, "bound transcript has one initial follow frame");
      probe.flushFrames();
      check(atBottom(), "real transcript scroll reaches bottom");
      for (let i = 21; i <= 140; i++) {
        followSnapshot = { ...followSnapshot, transcript_line_count: i };
        await tick();
      }
      check(probe.frameCount() === 1, "120 transcript DOM updates coalesce into one frame");
      check(transcript.textContent?.includes("Transcript 139"), "frame follows latest rendered transcript");
      probe.flushFrames(); check(atBottom(), "coalesced frame follows latest DOM height");

      followSnapshot = { ...followSnapshot, transcript_line_count: 141 }; await tick();
      const manualFrame = probe.frameCallbacks()[0];
      followAuto = false; await tick(); transcript.scrollTop = 0;
      check(probe.frameCount() === 0, "manual scrolling cancels pending follow");
      manualFrame(); await tick();
      check(transcript.scrollTop === 0, "queued frame cannot override manual position");
      followAuto = true; await tick();
      manualFrame();
      check(probe.frameCount() === 1 && transcript.scrollTop === 0, "retired manual frame cannot consume resumed follow");
      probe.flushFrames(); check(atBottom(), "auto-follow resumes at unchanged line count");

      followSnapshot = { ...followSnapshot, transcript_line_count: 142 }; await tick();
      const hiddenFrame = probe.frameCallbacks()[0];
      surfaceVisible = false; await tick(); transcript.scrollTop = 0;
      check(probe.frameCount() === 0, "hidden transcript cancels pending frame");
      hiddenFrame();
      check(transcript.scrollTop === 0, "queued hidden frame does not scroll");
      surfaceVisible = true; await tick();
      check(probe.frameCount() === 1, "returning to transcript schedules one catch-up frame");
      probe.flushFrames(); check(atBottom(), "visible catch-up reaches latest transcript");

      followSnapshot = { ...followSnapshot, transcript_line_count: 143 }; await tick();
      covered = true; await tick(); transcript.scrollTop = 0;
      check(probe.frameCount() === 0, "covering detail page stops transcript follow");
      covered = false; await tick(); probe.flushFrames();
      check(atBottom(), "closing detail page resumes transcript follow");

      followSnapshot = { ...followSnapshot, transcript_line_count: 144 }; await tick();
      followListening = false; await tick(); transcript.scrollTop = 0;
      check(probe.frameCount() === 0, "paused recording cancels follow frame");
      followListening = true; await tick(); probe.flushFrames();
      check(atBottom(), "recording resume catches up at unchanged count");

      followSnapshot = { ...followSnapshot, transcript_line_count: 145 }; await tick();
      const oldRecordingFrame = probe.frameCallbacks()[0];
      followSnapshot = { ...followSnapshot, session_id: "recording-b" }; await tick(); transcript.scrollTop = 0;
      oldRecordingFrame();
      check(probe.frameCount() === 1 && transcript.scrollTop === 0, "old recording frame cannot consume new recording's frame");
      probe.flushFrames(); check(atBottom(), "replacement recording follows its own transcript");
      followSnapshot = { ...followSnapshot, active: false }; await tick();
      check(probe.frameCount() === 0, "inactive record does not follow archived transcript");
      followSnapshot = { ...followSnapshot, active: true }; await tick();
      check(probe.frameCount() === 1, "one pending frame awaits disposal");
      await probe.finish({ checks, error: null });
    } catch (error) {
      await probe.finish({ checks, error: String(error) });
    }
  }
</script>

<main>
  <p>Isolated LIVE deck verification — transcript update {transcriptTick}</p>
  <DisplayClock snapshot={clockSnapshot} visible={surfaceVisible} />
  <TranscriptFollow snapshot={followSnapshot} visible={surfaceVisible} {covered} listening={followListening} following={followAuto} />
  <LiveRightRail
    visible={surfaceVisible && !covered}
    summaryEntries={entries} activeSummaryIdx={0} summarySegmentCount={entries.length}
    renderMd={renderer} onOpenSummaryDetail={() => opens++} onSelectSegment={noop}
    onOpenOverall={noop} summarizing={false} summaryStatusLabel="" previewLayout={null}
    activeSummaryTerms={terms} termCardIdx={front}
    termFloatLabels={{ title: "用語注釈", source: "引用", externalSource: "出典", previous: "前", next: "次", boardTitle: "知識整理", empty: "", externalNode: "", collapse: "", expand: "", selectAll: "", deselectAll: "" }}
    onOpenWhiteboard={noop} onSelectTermCard={(i) => front = i}
    onTermCardPrev={() => front = (front - 1 + terms.length) % terms.length}
    onTermCardNext={() => front = (front + 1) % terms.length}
  />
</main>
