<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { fetchTimetable, fetchTimetableWeek, fetchExamTimetable, fetchGrades, fetchRegistration, fetchSyllabusFavorites, getAiConfig, aiChat, lunaInvoke, openSyllabusDetail, gcalCheckSession, gcalSyncTimetable } from "../api";
  import { cachedFetch, onCacheUpdate, syllabusSearchState, lunaAuthState, gcalAuthState, aiRefreshRequested } from "../stores";
  import type { TimetableData, TimetableEntry, SyllabusEntry, ExamTimetableData, ExamEntry, AiChatMessage } from "../stores";
  import ViewLoader from "../ViewLoader.svelte";
  import StudentBar from "../StudentBar.svelte";
  import Icon from "../Icon.svelte";
  import type { LunaCourse, LunaCommunity } from "../types";

  // ── KG-Course weekly state ──
  let loading = $state(true);
  let navigating = $state(false);
  let error = $state("");
  let data = $state<TimetableData | null>(null);
  let showFavorites = $state(true);

  // ── Exam state ──
  let examData = $state<ExamTimetableData | null>(null);

  // ── Luna state ──
  interface SelectOption { value: string; label: string; selected: boolean; }
  interface LunaTimetable {
    year: string; term: string; year_label: string; term_label: string;
    year_options: SelectOption[]; term_options: SelectOption[];
    courses: LunaCourse[]; communities: LunaCommunity[];
  }

  let lunaLoading = $state(false);
  let lunaError = $state("");
  let lunaTimetable = $state<LunaTimetable | null>(null);

  // ── Constants ──
  const periods = [1, 2, 3, 4, 5, 6, 7];
  const days = ["月", "火", "水", "木", "金", "土"];
  const periodTimes: Record<number, string> = {
    1: "9:00~10:30", 2: "11:00~12:30", 3: "13:30~15:00",
    4: "15:10~16:40", 5: "16:50~18:20",
  };
  const fullWidthToNum: Record<string, number> = {
    "１": 1, "２": 2, "３": 3, "４": 4, "５": 5, "６": 6, "７": 7,
  };

  // ── Helpers ──
  function parseDayPeriod(dp: string): Array<{ day: string; period: number }> {
    const results: Array<{ day: string; period: number }> = [];
    for (const part of dp.split(/[・\/,、]/)) {
      const m = part.trim().match(/([月火水木金土])曜([１２３４５６７1-7])時限/);
      if (m) {
        const p = fullWidthToNum[m[2]] ?? parseInt(m[2]);
        if (p >= 1 && p <= 7) results.push({ day: m[1], period: p });
      }
    }
    return results;
  }

  function short(text: string): string {
    return text.replace(/／.+$/, "").replace(/\/.+$/, "");
  }

  // ── Favorites ──
  let favoritesMap = $derived.by(() => {
    const map = new Map<string, SyllabusEntry[]>();
    if (!showFavorites) return map;
    for (const fav of $syllabusSearchState.favorites?.entries ?? []) {
      for (const slot of parseDayPeriod(fav.day_period)) {
        const key = `${slot.day}-${slot.period}`;
        const arr = map.get(key) ?? [];
        arr.push(fav);
        map.set(key, arr);
      }
    }
    return map;
  });

  let hasFavoritesCache = $derived(($syllabusSearchState.favorites?.entries.length ?? 0) > 0);

  // ── Unified Cell ──
  interface UnifiedCellData {
    luna?: LunaCourse;
    kgc?: TimetableEntry;
    exam?: ExamEntry;
    favorites: SyllabusEntry[];
  }

  // ── Pre-computed cell map (avoids 42 array scans per render) ──
  let cellMap = $derived.by(() => {
    const map = new Map<string, UnifiedCellData>();
    for (const day of days) {
      const dayIdx = days.indexOf(day) + 1;
      for (const period of periods) {
        map.set(`${day}-${period}`, {
          luna: lunaTimetable?.courses.find(c => c.period === period && c.day === dayIdx),
          kgc: data?.entries.find(e => e.day === day && e.period === period),
          exam: examData?.entries.find(e => e.day === day && e.period === period),
          favorites: favoritesMap.get(`${day}-${period}`) ?? [],
        });
      }
    }
    return map;
  });

  function getUnifiedCell(day: string, period: number): UnifiedCellData {
    return cellMap.get(`${day}-${period}`) ?? { favorites: [] };
  }

  function cellIsEmpty(cell: UnifiedCellData): boolean {
    return !cell.luna && !cell.kgc && !cell.exam && cell.favorites.length === 0;
  }

  function cellItemCount(cell: UnifiedCellData): number {
    let n = cell.favorites.length;
    if (cell.luna || cell.kgc) n++;
    return n;
  }

  function cellDotColor(cell: UnifiedCellData): string {
    if (cell.kgc) {
      if (cell.kgc.is_cancelled)    return "#ff3b30";
      if (cell.kgc.is_makeup)       return "#34c759";
      if (cell.kgc.is_room_changed) return "#ff9500";
    }
    return "var(--accent)";
  }

  async function handleCellClick(cell: UnifiedCellData) {
    if (cell.luna && $lunaAuthState.authenticated) {
      await openLunaCourse(cell.luna.idnumber, cell.luna.name, cell.kgc?.detail_path);
    } else if (cell.kgc?.detail_path) {
      try {
        await invoke("open_detail_window", { path: cell.kgc.detail_path, courseName: cell.kgc.course_name });
      } catch (e: any) {
        console.error("Failed to open detail:", e);
      }
    }
  }

  async function openKgcDetail(entry: TimetableEntry, event: MouseEvent) {
    event.stopPropagation();
    if (!entry.detail_path) return;
    try {
      await invoke("open_detail_window", { path: entry.detail_path, courseName: entry.course_name });
    } catch (e: any) {
      console.error("Failed to open KGC detail:", e);
    }
  }

  async function openLunaCourse(idnumber: string, name: string, kgcPath?: string) {
    try {
      await invoke("luna_open_detail_window", {
        path: "", title: name, mode: "course", idnumber,
        kgcPath: kgcPath || null,
      });
    } catch (e: any) {
      console.error("Failed to open Luna course:", e);
    }
  }

  // ── Calendar sync ──
  let syncing = $state(false);
  let syncTooltip = $state("");
  let gcalTooltip = $state("");
  let autoSync = $state(localStorage.getItem("selah-auto-sync") === "true");

  const SYNC_HASH_KEY = "selah-sync-hash";

  type SyncEntry = { day: string; period: number; course_name: string; room: string; is_cancelled: boolean };

  function computeSyncHash(entries: SyncEntry[], weekLabel: string): string {
    const payload = weekLabel + "|" + entries
      .map(e => `${e.day}:${e.period}:${e.course_name}:${e.room}:${e.is_cancelled}`)
      .sort()
      .join(";");
    let h = 0;
    for (let i = 0; i < payload.length; i++) {
      h = ((h << 5) - h + payload.charCodeAt(i)) | 0;
    }
    return h.toString(36);
  }

  /** Mon-Thu: this week's Monday; Fri-Sun: next week's Monday */
  function getSyncTargetMonday(): string {
    const now = new Date();
    const dow = now.getDay(); // 0=Sun,1=Mon,...,5=Fri,6=Sat
    const monday = new Date(now);
    monday.setDate(now.getDate() - ((dow + 6) % 7)); // back to this Monday
    if (dow === 0 || dow >= 5) {
      monday.setDate(monday.getDate() + 7); // advance to next Monday
    }
    const y = monday.getFullYear();
    const m = String(monday.getMonth() + 1).padStart(2, "0");
    const d = String(monday.getDate()).padStart(2, "0");
    return `${y}/${m}/${d}`;
  }

  /** Get the sync target week's data, fetching if displayed week doesn't match */
  async function getSyncData(): Promise<{ entries: SyncEntry[]; weekLabel: string } | null> {
    const target = getSyncTargetMonday();
    if (data && data.week_label === target) {
      return {
        entries: data.entries.map(e => ({
          day: e.day, period: e.period, course_name: e.course_name,
          room: e.room, is_cancelled: e.is_cancelled,
        })),
        weekLabel: target,
      };
    }
    // Displayed week differs from sync target — fetch the correct week
    try {
      const current = await fetchTimetable();
      if (current.week_label === target) {
        return {
          entries: current.entries.map(e => ({
            day: e.day, period: e.period, course_name: e.course_name,
            room: e.room, is_cancelled: e.is_cancelled,
          })),
          weekLabel: target,
        };
      }
      // Navigate forward/backward to reach the target week (max 4 hops)
      let result = current;
      const targetDate = new Date(target.replace(/\//g, "-"));
      for (let i = 0; i < 4; i++) {
        const resultDate = new Date((result.week_label || "").replace(/\//g, "-"));
        if (result.week_label === target) break;
        const direction = targetDate > resultDate ? "next" : "prev";
        result = await fetchTimetableWeek(direction);
      }
      if (result.week_label === target) {
        return {
          entries: result.entries.map(e => ({
            day: e.day, period: e.period, course_name: e.course_name,
            room: e.room, is_cancelled: e.is_cancelled,
          })),
          weekLabel: target,
        };
      }
      // Fallback: use whatever we got
      return {
        entries: result.entries.map(e => ({
          day: e.day, period: e.period, course_name: e.course_name,
          room: e.room, is_cancelled: e.is_cancelled,
        })),
        weekLabel: result.week_label || "",
      };
    } catch {
      // If fetching fails, fall back to displayed data
      if (!data) return null;
      return {
        entries: data.entries.map(e => ({
          day: e.day, period: e.period, course_name: e.course_name,
          room: e.room, is_cancelled: e.is_cancelled,
        })),
        weekLabel: data.week_label || "",
      };
    }
  }

  function hasDataChanged(entries: SyncEntry[], weekLabel: string): { changed: boolean; hash: string } {
    const hash = computeSyncHash(entries, weekLabel);
    const lastHash = localStorage.getItem(SYNC_HASH_KEY) || "";
    return { changed: hash !== lastHash, hash };
  }

  function flashTooltip(target: "sys" | "gcal", msg: string, duration: number) {
    if (target === "sys") {
      syncTooltip = msg;
      setTimeout(() => { if (syncTooltip === msg) syncTooltip = ""; }, duration);
    } else {
      gcalTooltip = msg;
      setTimeout(() => { if (gcalTooltip === msg) gcalTooltip = ""; }, duration);
    }
  }

  async function syncCalendar(entries: SyncEntry[], weekLabel: string, silent = false) {
    if (syncing) return;
    syncing = true;
    try {
      const result = await invoke<string>("sync_calendar", { entries, weekLabel });
      if (!silent) flashTooltip("sys", result, 3000);
    } catch (e: any) {
      if (!silent) flashTooltip("sys", "同期失敗: " + (e?.message || String(e)), 5000);
    } finally { syncing = false; }
  }

  // ── Google Calendar sync ──
  let gcalSyncing = $state(false);
  let gcalAutoSync = $state(localStorage.getItem("selah-gcal-auto-sync") === "true");

  async function loadGcalStatus() {
    try {
      const status = await gcalCheckSession();
      gcalAuthState.set({
        authenticated: status.authenticated,
        calendarExists: status.calendar_exists,
        syncedEvents: status.synced_events,
      });
    } catch { /* ignore */ }
  }

  async function syncGoogleCalendar(entries: SyncEntry[], weekLabel: string, silent = false) {
    if (gcalSyncing || !$gcalAuthState.authenticated) return;
    gcalSyncing = true;
    try {
      const result = await gcalSyncTimetable(entries, weekLabel);
      if (!silent) flashTooltip("gcal", result, 3000);
      await loadGcalStatus();
    } catch (e: any) {
      if (!silent) flashTooltip("gcal", "Google同期失敗: " + (e?.message || String(e)), 5000);
    } finally { gcalSyncing = false; }
  }

  /** Sync calendars only if timetable data has changed since last sync */
  async function syncIfChanged(silent = false) {
    const syncData = await getSyncData();
    if (!syncData) return;
    const { entries, weekLabel } = syncData;
    const { changed, hash } = hasDataChanged(entries, weekLabel);
    if (!changed) {
      if (!silent) flashTooltip("sys", "変更なし - 同期不要", 2000);
      return;
    }
    const promises: Promise<void>[] = [];
    promises.push(syncCalendar(entries, weekLabel, silent));
    if ($gcalAuthState.authenticated) promises.push(syncGoogleCalendar(entries, weekLabel, silent));
    await Promise.allSettled(promises);
    localStorage.setItem(SYNC_HASH_KEY, hash);
  }

  async function afterDataLoaded() {
    if (autoSync || (gcalAutoSync && $gcalAuthState.authenticated)) {
      await syncIfChanged(true);
    }
  }

  // ── Week auto-selection ──
  // On initial load, if the server-default week is fully in the past, advance
  // to the current/future week. Manual navigation blocks SWR overwrites.
  let userNavigated = false;

  function isWeekFullyPast(d: TimetableData): boolean {
    const m = (d.week_label || "").match(/(\d{4})\/(\d{2})\/(\d{2})/);
    if (!m) return false;
    const weekEnd = new Date(+m[1], +m[2] - 1, +m[3]);
    weekEnd.setDate(weekEnd.getDate() + 6);
    weekEnd.setHours(23, 59, 59, 999);
    return new Date() > weekEnd;
  }

  /** Skip past fully-elapsed weeks (max 4 hops). */
  async function autoAdvanceToRelevantWeek(initial: TimetableData): Promise<TimetableData> {
    let current = initial;
    for (let i = 0; i < 4; i++) {
      if (!isWeekFullyPast(current)) return current;
      if (!current.form_fields) break;
      try {
        current = await fetchTimetableWeek("next");
      } catch {
        return current;
      }
    }
    return current;
  }

  // Track whether auto-advance moved us past the server default week
  let advancedPastDefault = false;

  // ── Data loading (all sources in parallel) ──
  // SWR: update UI when background refresh brings fresh data
  const unsubTimetable = onCacheUpdate<TimetableData>("timetable", (fresh) => {
    // Don't overwrite if user manually navigated or we auto-advanced past the default week
    if (userNavigated || advancedPastDefault) return;
    data = fresh;
    afterDataLoaded();
    updateTray();
  });
  const unsubExams = onCacheUpdate<ExamTimetableData>("exams", (fresh) => { examData = fresh; });
  const unsubLuna = onCacheUpdate<LunaTimetable>("luna_timetable", (fresh) => { lunaTimetable = fresh; });
  onDestroy(() => { unsubTimetable(); unsubExams(); unsubLuna(); });

  onMount(async () => {
    const kgcPromise = (async () => {
      try {
        const cached = await cachedFetch("timetable", fetchTimetable);
        if (cached) {
          const result = await autoAdvanceToRelevantWeek(cached);
          advancedPastDefault = result.week_label !== cached.week_label;
          data = result;
        }
        await afterDataLoaded();
      } catch (e: any) { error = e?.message || String(e); }
      finally { loading = false; }
    })();

    const lunaPromise = (async () => {
      if (!$lunaAuthState.authenticated) return;
      lunaLoading = true;
      try {
        lunaTimetable = await cachedFetch("luna_timetable", () => lunaInvoke<LunaTimetable>("luna_fetch_timetable", {}));
      } catch (e: any) { lunaError = String(e); }
      finally { lunaLoading = false; }
    })();

    const examPromise = (async () => {
      try { examData = await cachedFetch("exams", fetchExamTimetable); }
      catch { /* exam is supplementary */ }
    })();

    await Promise.allSettled([kgcPromise, lunaPromise, examPromise]);
    updateTray();
    loadGcalStatus();
  });

  function updateTray() {
    const entries = (data?.entries ?? []).map(e => ({
      day: e.day,
      period: e.period,
      course_name: e.course_name,
      room: e.room,
      is_cancelled: e.is_cancelled,
    }));
    invoke("update_tray", { entries }).catch(() => {});
  }

  async function navigateWeek(direction: "prev" | "next") {
    if (!data?.form_fields || navigating) return;
    userNavigated = true;
    navigating = true;
    try {
      data = await fetchTimetableWeek(direction);
      await afterDataLoaded();
    } catch (e: any) { error = e?.message || String(e); }
    finally { navigating = false; }
  }

  async function lunaSwitchTerm(year: string, term: string) {
    lunaLoading = true;
    try {
      lunaTimetable = await lunaInvoke<LunaTimetable>("luna_fetch_timetable", { year, term });
    } catch (e: any) { lunaError = String(e); }
    lunaLoading = false;
  }

  // ── AI Analysis ──
  const AI_CACHE_KEY = "selah_ai_analysis";
  const AI_CACHE_MAX_AGE = 24 * 60 * 60 * 1000; // 24h
  let aiAnalyzing = $state(false);
  let aiStatus = $state("");
  let aiCachedResult = $state(loadAiCache());
  // Watch for refresh requests from AI result window (via Dashboard)
  $effect(() => {
    if (get(aiRefreshRequested)) {
      aiRefreshRequested.set(false);
      runAiAnalysis(true);
    }
  });

  function loadAiCache(): string {
    try {
      const raw = localStorage.getItem(AI_CACHE_KEY);
      if (!raw) return "";
      const parsed = JSON.parse(raw);
      if (Date.now() - parsed.ts > AI_CACHE_MAX_AGE) {
        localStorage.removeItem(AI_CACHE_KEY);
        return "";
      }
      return parsed.result || "";
    } catch { return ""; }
  }

  function saveAiCache(result: string) {
    try {
      localStorage.setItem(AI_CACHE_KEY, JSON.stringify({ result, ts: Date.now() }));
    } catch { /* quota exceeded */ }
  }

  const LANGUAGE_NAMES: Record<string, string> = {
    ja: "日本語", zh: "中国語", en: "英語", ko: "韓国語",
  };

  async function openAiResultWindow(result: string, error?: string) {
    try {
      await invoke("open_ai_result_window", { result, error: error || null });
    } catch (e: any) {
      console.error("Failed to open AI result window:", e);
    }
  }

  async function buildAiData(): Promise<{ messages: AiChatMessage[] }> {
    const [gradesRes, regRes, favRes, aiConfig] = await Promise.allSettled([
      cachedFetch("grades", fetchGrades),
      cachedFetch("registration", fetchRegistration),
      cachedFetch("favorites", fetchSyllabusFavorites),
      getAiConfig(),
    ]);

    const grades = gradesRes.status === "fulfilled" ? gradesRes.value : null;
    const reg = regRes.status === "fulfilled" ? regRes.value : null;
    const favs = favRes.status === "fulfilled" ? favRes.value : null;
    const config = aiConfig.status === "fulfilled" ? aiConfig.value : null;

    const studentInfo = data?.student || grades?.student || reg?.student;
    const hasFavs = (favs?.entries.length ?? 0) > 0;

    // ── Build structured data sections ──
    let dataText = "";

    if (studentInfo) {
      dataText += `## 学生情報\n`;
      dataText += `氏名: ${studentInfo.name}\n学籍番号: ${studentInfo.student_id}\n`;
      dataText += `学部: ${studentInfo.faculty}\n学科: ${studentInfo.department}\n`;
      if (studentInfo.major) dataText += `専攻: ${studentInfo.major}\n`;
      dataText += `年次: ${studentInfo.status}\nクラス: ${studentInfo.class}\n\n`;
    }

    // Registered courses with structured table
    if (reg) {
      if (reg.credit_summary.length) {
        dataText += `## 単位概要\n`;
        dataText += `| 区分 | 履修単位 | 上限 | 残り |\n|---|---|---|---|\n`;
        for (const cs of reg.credit_summary) {
          const enrolled = parseInt(cs.enrolled) || 0;
          const limit = parseInt(cs.limit) || 0;
          const remaining = limit > 0 ? limit - enrolled : "-";
          dataText += `| ${cs.semester} | ${cs.enrolled} | ${cs.limit} | ${remaining} |\n`;
        }
        dataText += "\n";
      }

      dataText += `## 履修登録科目一覧\n`;
      dataText += `| 曜日 | 時限 | 科目名 | 学期 | 単位 | 教員 | キャンパス | 状態 |\n|---|---|---|---|---|---|---|---|\n`;
      // Compute per-day slot map for schedule density analysis
      const daySlots: Record<string, number[]> = {};
      let totalCredits = 0;
      for (const c of reg.courses) {
        const cr = parseInt(c.credits) || 0;
        totalCredits += cr;
        dataText += `| ${c.day} | ${c.period} | ${c.course_name} | ${c.semester} | ${c.credits} | ${c.instructor} | ${c.campus} | ${c.status || "-"} |\n`;
        if (c.day) {
          if (!daySlots[c.day]) daySlots[c.day] = [];
          const periodNum = parseInt(c.period);
          if (!isNaN(periodNum)) daySlots[c.day].push(periodNum);
        }
      }
      dataText += `\n合計登録単位: ${totalCredits}\n`;
      dataText += `登録科目数: ${reg.courses.length}\n\n`;

      // Schedule density summary
      dataText += `## 曜日別コマ数\n`;
      const allDays = ["月", "火", "水", "木", "金", "土"];
      for (const d of allDays) {
        const slots = daySlots[d] || [];
        if (slots.length > 0) {
          slots.sort((a, b) => a - b);
          const gaps = [];
          for (let i = 1; i < slots.length; i++) {
            if (slots[i] - slots[i - 1] > 1) {
              for (let g = slots[i - 1] + 1; g < slots[i]; g++) gaps.push(g);
            }
          }
          const gapInfo = gaps.length > 0 ? ` (空きコマ: ${gaps.join(",")}限)` : "";
          dataText += `- ${d}曜: ${slots.length}コマ [${slots.join(",")}限]${gapInfo}\n`;
        } else {
          dataText += `- ${d}曜: 休み\n`;
        }
      }
      dataText += "\n";
    }

    // Current week timetable (cancellations etc)
    if (data?.entries.length) {
      dataText += `## 今週の時間割 (${data.week_label})\n`;
      const cancelled = data.entries.filter(e => e.is_cancelled);
      const makeup = data.entries.filter(e => e.is_makeup);
      if (cancelled.length) {
        dataText += `休講: ${cancelled.map(e => `${e.day}${e.period}限 ${e.course_name}`).join(", ")}\n`;
      }
      if (makeup.length) {
        dataText += `補講: ${makeup.map(e => `${e.day}${e.period}限 ${e.course_name}`).join(", ")}\n`;
      }
      if (!cancelled.length && !makeup.length) {
        dataText += `変更なし（通常通り）\n`;
      }
      dataText += "\n";
    }

    // Curriculum requirements with gap analysis
    if (grades?.curriculum.length) {
      dataText += `## 卒業要件・単位取得状況\n`;
      dataText += `| 区分 | 必要単位 | 履修単位 | 修得単位 | 不足 |\n|---|---|---|---|---|\n`;
      for (const r of grades.curriculum) {
        const required = parseInt(r.required_credits) || 0;
        const earned = parseInt(r.earned_credits) || 0;
        const deficit = required > earned ? required - earned : 0;
        const deficitStr = deficit > 0 ? `⚠️ ${deficit}` : "✅ 0";
        dataText += `| ${r.category} | ${r.required_credits} | ${r.enrolled_credits} | ${r.earned_credits} | ${deficitStr} |\n`;
      }
      dataText += "\n";
    }

    // Favorites — detailed with conflict analysis
    if (hasFavs) {
      dataText += `## ブックマーク（お気に入り）科目\n`;
      dataText += `| 科目名 | 教員 | 学期 | 曜日時限 | キャンパス | 単位 |\n|---|---|---|---|---|---|\n`;
      for (const f of favs!.entries) {
        dataText += `| ${f.course_title} | ${f.instructor} | ${f.term} | ${f.day_period} | ${f.campus} | ${f.credits || "?"} |\n`;
      }
      dataText += "\n";

      // Check for time conflicts between favorites and registered courses
      if (reg?.courses.length) {
        const regSlots = new Set(reg.courses.map(c => `${c.day}${c.period}`));
        const conflicts: string[] = [];
        const fittable: string[] = [];
        for (const f of favs!.entries) {
          // day_period might be like "月1" or "火2,木4"
          const slots = f.day_period.match(/[月火水木金土]\d/g) || [];
          const hasConflict = slots.some(s => {
            // Check against registered day+period patterns
            return reg.courses.some(c => {
              const cSlots = `${c.day}${c.period}`.match(/[月火水木金土]\d/g) || [];
              return cSlots.some(cs => cs === s);
            });
          });
          if (hasConflict) {
            conflicts.push(`${f.course_title} (${f.day_period})`);
          } else {
            fittable.push(`${f.course_title} (${f.day_period}, ${f.credits || "?"}単位)`);
          }
        }
        if (conflicts.length) {
          dataText += `### 時間割衝突あり\n`;
          for (const c of conflicts) dataText += `- ❌ ${c}\n`;
          dataText += "\n";
        }
        if (fittable.length) {
          dataText += `### 追加可能（衝突なし）\n`;
          for (const f of fittable) dataText += `- ✅ ${f}\n`;
          dataText += "\n";
        }
      }
    }

    if (lunaTimetable?.courses.length) {
      dataText += `## Luna LMS 登録コース\n`;
      for (const c of lunaTimetable.courses) {
        dataText += `- ${c.name} (${c.teacher})\n`;
      }
      dataText += "\n";
    }

    const lang = config?.reply_language || "ja";
    const langName = LANGUAGE_NAMES[lang] || "日本語";

    // Build system prompt — adapt based on whether favorites exist
    const systemPrompt = hasFavs
      ? `あなたは関西学院大学の学生向け履修アドバイザーAIです。提供されたデータを基に、**専門的かつ実用的な履修分析レポート**を作成してください。

**重要: この学生はブックマーク（お気に入り）に科目を登録しています。これは履修を検討中の科目です。現在の時間割にこれらの科目を組み込んだ場合のシミュレーションを最優先で行ってください。**

以下の構成で分析してください:

## 📊 現状サマリー
- 総登録科目数、総単位数、週あたりコマ数
- 最も負担の重い曜日と最も軽い曜日
- 全休日の有無

## 🔖 お気に入り科目の追加シミュレーション
**ここが最も重要なセクションです。**
- 追加可能な科目（時間割衝突なし）を明確に列挙
- 各科目を追加した場合の単位数変化と週間スケジュールへの影響を具体的に分析
- 時間割衝突のある科目も報告し、どちらを優先すべきかアドバイス
- 追加後の理想的な時間割パターンを提案（曜日×時限の表形式が望ましい）
- 追加による単位上限への影響を確認

## 📈 単位充足度分析
- 卒業要件に対する現在の充足率を区分ごとに評価
- 不足単位がある分野を優先度順に指摘
- お気に入り科目の追加でどの区分が改善されるか

## ⏰ 時間割バランス分析
- 曜日ごとのコマ分布の偏り評価
- 空きコマの活用状況（連続授業 vs 分散型）
- 1限・5限の有無と生活リズムへの影響
- キャンパス移動が必要な場合の時間的余裕

## 💡 総合アドバイス
- お気に入り科目の中からおすすめの追加科目を優先順位付きで提案
- 来学期以降を見据えた中長期的なアドバイス
- 注意すべきリスク（単位上限超過、過負荷、特定曜日への集中等）

**分析ルール:**
- 提供データのみに基づく（推測や捏造はしない）
- データ不足のセクションは「データ不足のためスキップ」と明記
- 数値は具体的に記載（「多い」ではなく「6コマ」等）
- Markdownの表・箇条書きを活用して読みやすく構成

回答は必ず${langName}で記述してください。`
      : `あなたは関西学院大学の学生向け履修アドバイザーAIです。提供されたデータを基に、**専門的かつ実用的な履修分析レポート**を作成してください。

以下の構成で分析してください:

## 📊 現状サマリー
- 総登録科目数、総単位数、週あたりコマ数
- 最も負担の重い曜日と最も軽い曜日
- 全休日の有無

## 📈 単位充足度分析
- 卒業要件に対する現在の充足率を区分ごとに評価
- 不足単位がある分野を優先度順に指摘
- 今学期で取得見込みの単位数と、卒業までの残り

## ⏰ 時間割バランス分析
- 曜日ごとのコマ分布の偏り評価
- 空きコマの活用状況（連続授業 vs 分散型）
- 1限・5限の有無と生活リズムへの影響
- キャンパス移動が必要な場合の時間的余裕

## 📋 科目構成分析
- 必修・選択・自由科目のバランス
- 同一教員の科目の偏り
- 学期（春/秋/通年）の分布

## 💡 総合アドバイス
- 現在の履修プランの長所と短所
- 来学期以降を見据えた中長期的なアドバイス
- 注意すべきリスク（単位上限超過、過負荷、特定曜日への集中等）
- シラバスのお気に入り機能を活用して候補科目を検討することを推奨

**分析ルール:**
- 提供データのみに基づく（推測や捏造はしない）
- データ不足のセクションは「データ不足のためスキップ」と明記
- 数値は具体的に記載（「多い」ではなく「6コマ」等）
- Markdownの表・箇条書きを活用して読みやすく構成

回答は必ず${langName}で記述してください。`;

    return { messages: [
      { role: "system" as const, content: systemPrompt },
      {
        role: "user" as const,
        content: `以下の学生データに基づいて、履修状況の詳細分析レポートを作成してください。\n\n${dataText}`,
      },
    ]};
  }

  async function runAiAnalysis(forceRefresh = false) {
    if (aiCachedResult && !forceRefresh) {
      await openAiResultWindow(aiCachedResult);
      return;
    }
    aiAnalyzing = true;
    aiStatus = "データ収集中...";

    try {
      const { messages } = await buildAiData();
      aiStatus = "AI分析中...";
      const result = await aiChat(messages);
      aiCachedResult = result;
      saveAiCache(result);
      await openAiResultWindow(result);
    } catch (e: any) {
      await openAiResultWindow("", e?.message || String(e));
    } finally {
      aiAnalyzing = false;
      aiStatus = "";
    }
  }
</script>

<div class="view">
  <div class="title-row">
    <h2>時間割</h2>
    <div class="title-controls">
      {#if hasFavoritesCache}
        <button
          class="fav-toggle"
          class:active={showFavorites}
          onclick={() => showFavorites = !showFavorites}
        >
          <svg width="11" height="11" viewBox="0 0 16 16" fill="none">
            <path d="M4 2h8a1 1 0 0 1 1 1v11.5l-5-3-5 3V3a1 1 0 0 1 1-1z"
              fill={showFavorites ? "currentColor" : "none"}
              stroke="currentColor" stroke-width="1.4"
            />
          </svg>
          お気に入り
        </button>
      {/if}
      <button class="ai-btn" onclick={() => runAiAnalysis()} disabled={aiAnalyzing || (!data && !lunaTimetable)}>
        {#if aiAnalyzing}
          <span class="mini-spinner"></span>
          {aiStatus}
        {:else}
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none">
            <path d="M8 1l1.5 3.5L13 6l-3.5 1.5L8 11 6.5 7.5 3 6l3.5-1.5z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>
            <path d="M12 10l.75 1.75L14.5 12.5l-1.75.75L12 15l-.75-1.75-1.75-.75 1.75-.75z" stroke="currentColor" stroke-width="0.9" stroke-linejoin="round" fill="none"/>
          </svg>
          AI分析
        {/if}
      </button>
      {#if data}
        <div class="cal-controls">
          <button class="sync-btn" class:auto-active={autoSync || gcalAutoSync} onclick={() => syncIfChanged()} disabled={syncing || gcalSyncing}
            title={(syncing || gcalSyncing) ? "同期中..." : "カレンダーに同期"}>
            <span class="sync-icons">
              <svg width="12" height="12" viewBox="0 0 384 512" fill="currentColor" class="btn-icon apple">
                <path d="M318.7 268.7c-.2-36.7 16.4-64.4 50-84.8-18.8-26.9-47.2-41.7-84.7-44.6-35.5-2.8-74.3 20.7-88.5 20.7-15 0-49.4-19.7-76.4-19.7C63.3 141.2 4 184 4 273.5c0 26.2 4.8 53.3 14.4 81.2 12.8 36.7 59 126.7 107.2 125.2 25.2-.6 43-17.9 75.8-17.9 31.8 0 48.3 17.9 76.4 17.9 48.6-.7 90.4-82.5 102.6-119.3-65.2-30.7-61.7-90-61.7-91.9zm-56.6-164.2c27.3-32.4 24.8-61.9 24-72.5-24.1 1.4-52 16.4-67.9 34.9-17.5 19.8-27.8 44.3-25.6 71.9 26.1 2 49.9-11.4 69.5-34.3z"/>
              </svg>
              {#if $gcalAuthState.authenticated}
                <svg width="12" height="12" viewBox="0 0 24 24" class="btn-icon gcal">
                  <path class="g-blue" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z"/>
                  <path class="g-green" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
                  <path class="g-yellow" d="M5.84 14.09A6.97 6.97 0 015.48 12c0-.72.13-1.43.36-2.09V7.07H2.18A11.96 11.96 0 001 12c0 1.94.46 3.77 1.18 4.93l3.66-2.84z"/>
                  <path class="g-red" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
                </svg>
              {/if}
            </span>
            {#if syncing || gcalSyncing}
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" class="spin">
                <path d="M14 8A6 6 0 1 1 8 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
                <path d="M14 2v4h-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
              同期中...
            {:else}
              同期
            {/if}
            {#if autoSync || gcalAutoSync}<span class="auto-dot"></span>{/if}
          </button>
        </div>
      {/if}
    </div>
  </div>

  <!-- Sub-controls: week nav + Luna term selector -->
  <div class="sub-controls">
    {#if data}
      <div class="week-nav">
        <button class="nav-btn" onclick={() => navigateWeek("prev")} disabled={navigating}>
          <Icon name="chevron.left" size={13} />前週
        </button>
        <span class="week-label" class:navigating>{data.week_label || ""}</span>
        <button class="nav-btn" onclick={() => navigateWeek("next")} disabled={navigating}>
          次週<Icon name="chevron.right" size={13} />
        </button>
      </div>
    {/if}
    {#if $lunaAuthState.authenticated && lunaTimetable}
      <div class="select-group">
        <select class="apple-select" value={lunaTimetable.year}
          onchange={(e) => lunaSwitchTerm((e.target as HTMLSelectElement).value, lunaTimetable!.term)}>
          {#each lunaTimetable.year_options as opt}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
        <select class="apple-select" value={lunaTimetable.term}
          onchange={(e) => lunaSwitchTerm(lunaTimetable!.year, (e.target as HTMLSelectElement).value)}>
          {#each lunaTimetable.term_options as opt}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>
    {/if}
  </div>

  <div class="legend">
    <span class="legend-item"><span class="dot" style="background:var(--accent)"></span>授業</span>
    <span class="legend-item"><span class="dot" style="background:#ff3b30"></span>休講</span>
    <span class="legend-item"><span class="dot" style="background:#34c759"></span>補講</span>
    <span class="legend-item"><span class="dot" style="background:#ff9500"></span>教室変更</span>
    {#if hasFavoritesCache && showFavorites}
      <span class="legend-item"><span class="dot" style="background:#af52de"></span>お気に入り</span>
    {/if}
  </div>

  <ViewLoader {loading} {error} empty={!loading && !error && !data?.entries.length && !lunaTimetable?.courses.length} emptyMessage="登録されている授業はありません">
    {#if lunaLoading}
      <div class="luna-loading-hint">
        <span class="mini-spinner"></span>
        Luna 読み込み中...
      </div>
    {/if}

    <div class="grid-wrap">
      <div class="timetable" class:navigating>
        <div class="grid-header corner"></div>
        {#each days as day}
          <div class="grid-header">{day}</div>
        {/each}

        {#each periods as period}
          <div class="grid-header period-num">
            <span class="period-label">{period}</span>
            {#if periodTimes[period]}
              {@const [start, end] = periodTimes[period].split("~")}
              <span class="period-time">{start}</span>
              <span class="period-time">{end}</span>
            {/if}
          </div>

          {#each days as day}
            {@const cell = getUnifiedCell(day, period)}
            {@const empty = cellIsEmpty(cell)}
            {@const hasCourse = !!(cell.luna || cell.kgc)}
            {@const favOnly = !cell.luna && !cell.kgc && !cell.exam && cell.favorites.length > 0}

            {#if favOnly}
              <div class="cell" class:multi={cell.favorites.length > 1}>
                {#if cell.favorites.length > 1}
                  <div class="conflict-banner">重複 {cell.favorites.length}件</div>
                {/if}
                {#each cell.favorites as fav}
                  <button class="item fav-item" onclick={() => openSyllabusDetail(fav.class_code, fav.course_title).catch(console.error)}>
                    <div class="item-top">
                      <span class="course-name fav-title">{short(fav.course_title)}</span>
                      <span class="dot" style="background:#af52de;flex-shrink:0;margin-top:2px"></span>
                    </div>
                    <span class="course-room">{short(fav.instructor)}</span>
                  </button>
                {/each}
              </div>
            {:else if !empty}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              {@const totalItems = cellItemCount(cell)}
              <div
                class="cell course-cell"
                class:multi={totalItems > 1}
                class:entry-normal={hasCourse && !cell.kgc?.is_cancelled && !cell.kgc?.is_makeup && !cell.kgc?.is_room_changed}
                class:entry-cancelled={cell.kgc?.is_cancelled}
                class:entry-makeup={cell.kgc?.is_makeup}
                class:entry-changed={cell.kgc?.is_room_changed}
                class:exam-only={!hasCourse && !!cell.exam}
                onclick={() => handleCellClick(cell)}
              >
                {#if totalItems > 1}
                  <div class="conflict-banner">重複 {totalItems}件</div>
                {/if}
                <div class="item">
                  <div class="item-top">
                    <span class="course-name" class:struck={cell.kgc?.is_cancelled}>
                      {cell.luna?.name || cell.kgc?.course_name || cell.exam?.course_name || ""}
                    </span>
                    {#if hasCourse}
                      <span class="dot" style="background:{cellDotColor(cell)};flex-shrink:0;margin-top:2px"></span>
                    {/if}
                  </div>
                  {#if cell.luna?.teacher}
                    <span class="course-teacher">{cell.luna.teacher}</span>
                  {/if}
                  {#if cell.kgc?.room}
                    <span class="course-room">{cell.kgc.room}</span>
                  {/if}
                  {#if cell.exam && !hasCourse}
                    <span class="course-room">{cell.exam.room}</span>
                  {/if}
                  {#if cell.kgc?.is_cancelled || cell.kgc?.is_makeup || cell.kgc?.is_room_changed || cell.exam}
                    <div class="tags">
                      {#if cell.kgc?.is_cancelled}<span class="tag tag-cancel">休講</span>{/if}
                      {#if cell.kgc?.is_makeup}<span class="tag tag-makeup">補講</span>{/if}
                      {#if cell.kgc?.is_room_changed}<span class="tag tag-change">変更</span>{/if}
                      {#if cell.exam}<span class="tag tag-exam">試験</span>{/if}
                    </div>
                  {/if}
                </div>

                {#if cell.favorites.length > 0}
                  {#each cell.favorites as fav}
                    <button class="item fav-item fav-sub" onclick={(e) => { e.stopPropagation(); openSyllabusDetail(fav.class_code, fav.course_title).catch(console.error); }}>
                      <div class="item-top">
                        <span class="course-name fav-title">{short(fav.course_title)}</span>
                        <span class="dot" style="background:#af52de;flex-shrink:0;margin-top:2px"></span>
                      </div>
                      <span class="course-room">{short(fav.instructor)}</span>
                    </button>
                  {/each}
                {/if}
              </div>
            {:else}
              <div class="cell"></div>
            {/if}
          {/each}
        {/each}
      </div>
    </div>

    {#if lunaTimetable?.communities && lunaTimetable.communities.length > 0}
      <div class="comm-section">
        <h3>コミュニティ</h3>
        <div class="comm-chips">
          {#each lunaTimetable.communities as comm}
            <button class="comm-chip" onclick={() => openLunaCourse(comm.idnumber, comm.name)}>
              {comm.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </ViewLoader>
</div>

<style>
  /* ── Layout ── */
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  .title-row h2 { margin: 0; }
  .title-controls { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }

  /* ── Sub-controls: week nav + Luna term ── */
  .sub-controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 10px;
    flex-wrap: wrap;
  }

  /* ── Favourite toggle ── */
  .fav-toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 11px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    transition: all 0.15s ease;
  }
  .fav-toggle:hover { background: var(--bg-hover); color: var(--text-primary); }
  .fav-toggle.active {
    background: rgba(175, 82, 222, 0.1);
    border-color: rgba(175, 82, 222, 0.25);
    color: #af52de;
  }

  /* ── Calendar sync ── */
  .cal-controls {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .sync-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 5px 10px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-secondary);
    transition: all 0.15s ease;
  }
  .sync-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .sync-btn:disabled { opacity: 0.5; cursor: default; }
  .sync-btn .spin { animation: spin 0.8s linear infinite; }

  .btn-icon { flex-shrink: 0; }
  .sync-icons { display: flex; align-items: center; gap: 3px; }
  .btn-icon.gcal .g-blue { fill: #4285F4; }
  .btn-icon.gcal .g-green { fill: #34A853; }
  .btn-icon.gcal .g-yellow { fill: #FBBC05; }
  .btn-icon.gcal .g-red { fill: #EA4335; }

  .sync-btn.auto-active {
    border-color: var(--accent);
    color: var(--accent);
  }

  .auto-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent);
    flex-shrink: 0;
  }

  /* ── Week navigation ── */
  .week-nav { display: flex; align-items: center; gap: 4px; }
  .nav-btn {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 5px 11px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 500;
    font-family: inherit;
    color: var(--accent);
    background: var(--bg-card);
    border: 0.5px solid var(--border);
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .nav-btn:hover:not(:disabled) { background: var(--accent-light); }
  .nav-btn:active:not(:disabled) { transform: scale(0.97); }
  .nav-btn:disabled { opacity: 0.35; cursor: default; }
  .week-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    min-width: 190px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    transition: opacity 0.2s ease;
    padding: 0 6px;
  }
  .week-label.navigating { opacity: 0.35; }

  /* ── Luna term selector ── */
  .select-group { display: flex; gap: 6px; }
  .apple-select {
    padding: 5px 10px;
    border-radius: 8px;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    outline: none;
  }
  .apple-select:hover { background: var(--bg-hover); }

  /* ── Luna loading hint ── */
  .luna-loading-hint {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-tertiary);
    margin-bottom: 8px;
  }
  .mini-spinner {
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  /* ── Legend ── */
  .legend {
    display: flex;
    gap: 14px;
    justify-content: flex-end;
    align-items: center;
    margin-bottom: 8px;
    font-size: 11px;
    color: var(--text-tertiary);
  }
  .legend-item { display: flex; align-items: center; gap: 5px; }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  /* ── Grid ── */
  .grid-wrap {
    border-radius: 14px;
    overflow-x: auto;
    overflow-y: hidden;
    box-shadow: var(--shadow-md);
    animation: fade-in 0.3s ease both;
    -webkit-overflow-scrolling: touch;
  }
  .timetable {
    min-width: 480px;
    display: grid;
    grid-template-columns: 48px repeat(6, 1fr);
    gap: 1px;
    background: var(--border);
    transition: opacity 0.2s ease;
  }
  .timetable.navigating { opacity: 0.45; pointer-events: none; }

  /* ── Headers ── */
  .grid-header {
    background: var(--bg-secondary);
    padding: 9px 4px;
    text-align: center;
    font-weight: 600;
    font-size: 11.5px;
    color: var(--text-secondary);
    letter-spacing: 0.02em;
  }
  .corner { background: var(--bg-tertiary); }
  .period-num {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0;
    line-height: 1;
    padding: 4px 2px;
  }
  .period-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: 2px;
  }
  .period-time {
    font-size: 9px;
    font-weight: 400;
    color: var(--text-tertiary);
    opacity: 0.6;
    white-space: nowrap;
    line-height: 1.3;
  }

  /* ── Cell ── */
  .cell {
    background: var(--bg-card);
    display: flex;
    flex-direction: column;
    min-height: 68px;
    overflow: hidden;
    position: relative;
  }

  .course-cell {
    cursor: pointer;
    transition: filter 0.12s ease;
  }
  .course-cell:hover { filter: brightness(0.97); }
  .course-cell:active { filter: brightness(0.93); }

  .entry-normal   { background: color-mix(in srgb, var(--accent) 8%, var(--bg-card)); }
  .entry-cancelled { background: rgba(255, 59, 48, 0.06); }
  .entry-makeup   { background: rgba(52, 199, 89, 0.08); }
  .entry-changed  { background: rgba(255, 149, 0, 0.07); }
  .exam-only      { background: var(--kg-gold-light, rgba(200, 170, 80, 0.08)); }

  /* ── Item content ── */
  .item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px 7px 6px;
    flex: 1;
    min-height: 58px;
    text-align: left;
    width: 100%;
  }
  .item-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 4px;
  }
  .course-name {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    flex: 1;
    min-width: 0;
  }
  .struck { text-decoration: line-through; opacity: 0.5; }
  .fav-title { color: #af52de; }

  .course-teacher {
    font-size: 10px;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .course-room {
    font-size: 10px;
    color: var(--text-tertiary);
    word-break: break-all;
  }

  /* ── Tags ── */
  .tags { display: flex; gap: 3px; flex-wrap: wrap; margin-top: 2px; }
  .tag {
    font-size: 9px;
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 4px;
    line-height: 1.4;
    letter-spacing: 0.01em;
  }
  .tag-cancel { background: rgba(255, 59, 48, 0.12); color: #ff3b30; }
  .tag-makeup { background: rgba(52, 199, 89, 0.12); color: #28a745; }
  .tag-change { background: rgba(255, 149, 0, 0.12); color: #cc7700; }
  .tag-exam   { background: rgba(200, 170, 80, 0.15); color: #8a7530; }


  /* ── Conflict banner ── */
  .conflict-banner {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    background: #fff3e0;
    border-radius: 4px;
    font-size: 8.5px;
    font-weight: 500;
    color: #cc7700;
    opacity: 0.75;
    flex-shrink: 0;
  }
  .conflict-badge {
    font-size: 8px;
    line-height: 1;
  }
  .multi {
    border-color: rgba(255, 149, 0, 0.25) !important;
  }
  .fav-sub {
    border-top: 0.5px solid var(--border);
  }

  /* ── Favourite item ── */
  .fav-item {
    background: rgba(175, 82, 222, 0.06);
    padding: 7px 7px 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-height: 58px;
    border: none;
    border-radius: 0;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }
  .fav-item:hover {
    background: rgba(175, 82, 222, 0.12);
  }

  /* ── Communities ── */
  .comm-section { margin-top: 16px; }
  .comm-section h3 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0 0 8px 0;
  }
  .comm-chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .comm-chip {
    padding: 6px 14px;
    border-radius: 20px;
    border: 0.5px solid var(--border);
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s ease;
    box-shadow: var(--shadow-sm);
  }
  .comm-chip:hover { background: var(--bg-hover); box-shadow: var(--shadow-md); }

  /* ── Misc ── */
  .state-msg {
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
    padding: 40px 0;
  }

  /* ── AI Analysis Button ── */
  .ai-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 11px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 500;
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.12), rgba(0, 122, 255, 0.12));
    color: var(--text-primary);
    border: 0.5px solid rgba(175, 82, 222, 0.25);
    cursor: pointer;
    transition: all 0.2s;
  }
  .ai-btn:hover:not(:disabled) {
    background: linear-gradient(135deg, rgba(175, 82, 222, 0.22), rgba(0, 122, 255, 0.22));
    border-color: rgba(175, 82, 222, 0.4);
  }
  .ai-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
