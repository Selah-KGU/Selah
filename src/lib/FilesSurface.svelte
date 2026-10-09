<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyAuxiliaryTheme, syncAuxiliaryTheme } from "./auxiliarySurfaceTheme";
  import { acquireResourceGroup, ResourceScope } from "./resourceScope";
  import { LatestViewRead } from "./latestViewRead";
  import { createCacheSyncQueue } from "./cacheSyncQueue";
  import { FilePreviewController, filePreviewRequest, type DownloadPreview, type FilePreviewRequest } from "./filePreviewController";

  interface DownloadRecord {
    id: string;
    filename: string;
    path: string;
    course_name: string;
    source: string;
    size_bytes: number;
    downloaded_at: number;
    file_exists: boolean;
    subfolder?: string;
  }
  interface DuplicateItem extends DownloadRecord {
    is_recommended: boolean;
  }
  interface DuplicateGroup {
    content_hash: string;
    size_bytes: number;
    items: DuplicateItem[];
  }
  interface BulkResult {
    deleted_count?: number;
    failed_count?: number;
    errors?: string[];
  }
  function bulkResultMessage(result: BulkResult, summary: string): string {
    const errors = result.errors?.slice(0, 2).join(" / ");
    return errors ? summary + "（" + errors + "）" : summary;
  }
  interface ControlEvent {
    owner?: string;
    target?: string;
    tabId?: string;
    action?: string;
    payload?: unknown;
  }
  type FileGroup = { label: string; items: DownloadRecord[] };
  type FilterKind = "course" | "type" | "source";

  const FILE_TYPE_MAP: Record<string, { color: string; label: string }> = {
    pdf: { color: "#e74c3c", label: "PDF" },
    doc: { color: "#2b5797", label: "DOC" },
    docx: { color: "#2b5797", label: "DOC" },
    xls: { color: "#217346", label: "XLS" },
    xlsx: { color: "#217346", label: "XLS" },
    ppt: { color: "#d04423", label: "PPT" },
    pptx: { color: "#d04423", label: "PPT" },
    zip: { color: "#f39c12", label: "ZIP" },
    "7z": { color: "#f39c12", label: "7Z" },
    rar: { color: "#f39c12", label: "RAR" },
    png: { color: "#8e44ad", label: "PNG" },
    jpg: { color: "#8e44ad", label: "JPG" },
    jpeg: { color: "#8e44ad", label: "JPG" },
    gif: { color: "#8e44ad", label: "GIF" },
    mp4: { color: "#e67e22", label: "MP4" },
    mp3: { color: "#e67e22", label: "MP3" },
    txt: { color: "#7f8c8d", label: "TXT" },
    csv: { color: "#27ae60", label: "CSV" },
    md: { color: "#0a84ff", label: "MD" },
    html: { color: "#e44d26", label: "HTML" },
    htm: { color: "#e44d26", label: "HTML" },
  };
  const SOURCE_LABELS: Record<string, string> = {
    luna: "Luna LMS",
    mail: "メール",
    kwic: "KWIC",
    live: "Liveノート",
    scan: "スキャン",
  };
  const SORT_KEY_LABELS: Record<string, string> = {
    date: "日付",
    name: "名前",
    size: "サイズ",
    type: "形式",
  };
  const PERIOD_LABELS: Record<string, string> = {
    today: "今日",
    week: "今週",
    month: "今月",
    all: "全期間",
  };
  const PERIOD_ORDER: Period[] = ["all", "today", "week", "month"];
  const SORT_KEY_ORDER: SortKey[] = ["date", "name", "size", "type"];
  const PREVIEWABLE = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "md", "markdown", "txt", "csv", "json", "log"]);

  function readParam(name: string): string {
    try {
      const search = new URLSearchParams(window.location.search);
      const fromSearch = search.get(name);
      if (fromSearch) return fromSearch;
      const h = (location.hash || "").replace(/^#/, "");
      return new URLSearchParams(h).get(name) || "";
    } catch {
      return "";
    }
  }
  function readFocusCourse(): string {
    return readParam("course");
  }

  // Tab/owner labels let the surface push its controls into the document-tabs
  // second control row (mirrors the markdown reader pattern).
  const tabTarget = readParam("tabLabel");
  const tabOwner = readParam("ownerLabel") || "document-tabs";
  function getExt(filename: string): string {
    const parts = filename.split(".");
    return parts.length > 1 ? (parts.pop() || "").toLowerCase() : "";
  }
  function getTypeInfo(filename: string): { color: string; label: string } {
    const ext = getExt(filename);
    return FILE_TYPE_MAP[ext] || { color: "#95a5a6", label: ext.toUpperCase() || "FILE" };
  }
  function canPreviewFile(filename: string): boolean {
    return PREVIEWABLE.has(getExt(filename));
  }
  function formatSize(bytes: number): string {
    if (!bytes) return "";
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
    return (bytes / (1024 * 1024)).toFixed(1) + " MB";
  }
  function formatTime(ts: number): string {
    if (!ts) return "";
    const d = new Date(ts);
    const now = new Date();
    const diff = Math.floor((now.getTime() - ts) / 1000);
    if (diff < 60) return "たった今";
    if (diff < 3600) return Math.floor(diff / 60) + "分前";
    if (diff < 86400) return Math.floor(diff / 3600) + "時間前";
    const y = d.getFullYear(), m = d.getMonth() + 1, dd = d.getDate();
    const h = String(d.getHours()).padStart(2, "0"), mi = String(d.getMinutes()).padStart(2, "0");
    if (y === now.getFullYear()) return m + "/" + dd + " " + h + ":" + mi;
    return y + "/" + m + "/" + dd;
  }
  function courseKeyOf(r: DownloadRecord): string {
    return (r.course_name || "").trim() || "未分類";
  }

  // ── Persistence ──
  type ViewMode = "list" | "icons";
  type SortKey = "date" | "name" | "size" | "type";
  type SortDir = "asc" | "desc";
  type Period = "today" | "week" | "month" | "all";

  let period = $state<Period>("all");
  let hideMissing = $state(true);
  let sortKey = $state<SortKey>("date");
  let sortDir = $state<SortDir>("desc");
  let viewMode = $state<ViewMode>("list");

  function loadPrefs(): void {
    try {
      const raw = localStorage.getItem("downloads-prefs");
      if (!raw) return;
      const p = JSON.parse(raw);
      if (p.period) period = p.period;
      if (typeof p.hideMissing === "boolean") hideMissing = p.hideMissing;
      if (p.sortKey) sortKey = p.sortKey;
      if (p.sortDir) sortDir = p.sortDir;
      if (p.viewMode === "list" || p.viewMode === "icons") viewMode = p.viewMode;
    } catch {
      // ignore corrupt prefs
    }
  }
  function savePrefs(): void {
    try {
      localStorage.setItem("downloads-prefs", JSON.stringify({ period, hideMissing, sortKey, sortDir, viewMode }));
    } catch {
      // ignore storage failures
    }
  }

  // ── State ──
  let allRecords = $state<DownloadRecord[]>([]);
  let loading = $state(true);
  let courseFilters = $state<Set<string>>(new Set());
  let typeFilters = $state<Set<string>>(new Set());
  let sourceFilters = $state<Set<string>>(new Set());
  let searchQuery = $state("");
  let selectedIds = $state<Set<string>>(new Set());
  let bulkDeleteArmed = $state(false);
  let statusMessage = $state("");
  let openMenu = $state<"period" | "sort" | null>(null);
  let menuX = $state(0);

  const resources = new ResourceScope();
  let mutating = $state(false);
  let reloadAfterMutation = false;
  let themeVersion = 0;
  const refreshTheme = createCacheSyncQueue(async () => {
    const version = ++themeVersion;
    await syncAuxiliaryTheme(() => resources.active && version === themeVersion);
  });
  let pendingFocusCourse = readFocusCourse();

  function setForKind(kind: FilterKind): Set<string> {
    if (kind === "course") return courseFilters;
    if (kind === "type") return typeFilters;
    return sourceFilters;
  }
  function reassignKind(kind: FilterKind, next: Set<string>): void {
    if (kind === "course") courseFilters = next;
    else if (kind === "type") typeFilters = next;
    else sourceFilters = next;
  }
  function toggleFilter(kind: FilterKind, key: string): void {
    const next = new Set(setForKind(kind));
    if (next.has(key)) next.delete(key);
    else next.add(key);
    reassignKind(kind, next);
    statusMessage = "";
  }
  function clearKind(kind: FilterKind): void {
    reassignKind(kind, new Set());
    statusMessage = "";
  }
  function clearAllFilters(): void {
    courseFilters = new Set();
    typeFilters = new Set();
    sourceFilters = new Set();
    statusMessage = "";
  }
  let hasActiveFilters = $derived(courseFilters.size > 0 || typeFilters.size > 0 || sourceFilters.size > 0);

  // ── Sidebar counts ──
  function countBy(records: DownloadRecord[], keyFn: (r: DownloadRecord) => string): Record<string, number> {
    const out: Record<string, number> = {};
    for (const r of records) {
      const k = keyFn(r);
      out[k] = (out[k] || 0) + 1;
    }
    return out;
  }
  let courseCounts = $derived(countBy(allRecords, courseKeyOf));
  let typeCounts = $derived(countBy(allRecords, (r) => getExt(r.filename) || "other"));
  let sourceCounts = $derived(countBy(allRecords, (r) => r.source || "other"));

  function typeLabel(ext: string): string {
    return (FILE_TYPE_MAP[ext] || { label: ext.toUpperCase() || "OTHER" }).label;
  }
  function sourceLabel(src: string): string {
    return SOURCE_LABELS[src] || src;
  }
  function sortedKeys(counts: Record<string, number>, labelFn: (k: string) => string): string[] {
    return Object.keys(counts).sort((a, b) => (labelFn(a) || "").localeCompare(labelFn(b) || "", "ja"));
  }
  let courseKeys = $derived(sortedKeys(courseCounts, (k) => k));
  let typeKeys = $derived(sortedKeys(typeCounts, typeLabel));
  let sourceKeys = $derived(sortedKeys(sourceCounts, sourceLabel));

  // ── Filtering / sorting ──
  function periodCutoff(): number {
    if (period === "all") return 0;
    const now = new Date();
    if (period === "today") return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    if (period === "week") return new Date(now.getFullYear(), now.getMonth(), now.getDate() - 6).getTime();
    if (period === "month") return new Date(now.getFullYear(), now.getMonth(), 1).getTime();
    return 0;
  }
  function sortRecords(records: DownloadRecord[]): DownloadRecord[] {
    const dir = sortDir === "asc" ? 1 : -1;
    return records.sort((a, b) => {
      switch (sortKey) {
        case "name":
          return dir * (a.filename || "").localeCompare(b.filename || "", "ja", { numeric: true });
        case "size":
          return dir * ((a.size_bytes || 0) - (b.size_bytes || 0));
        case "type": {
          const av = getExt(a.filename) || "", bv = getExt(b.filename) || "";
          if (av === bv) return (a.filename || "").localeCompare(b.filename || "", "ja");
          return dir * av.localeCompare(bv);
        }
        case "date":
        default:
          return dir * (Number(a.downloaded_at || 0) - Number(b.downloaded_at || 0));
      }
    });
  }
  let filteredRecords = $derived.by<DownloadRecord[]>(() => {
    const cutoff = periodCutoff();
    const q = searchQuery;
    const out = allRecords.filter((r) => {
      if (hideMissing && r.file_exists === false) return false;
      if (courseFilters.size > 0 && !courseFilters.has(courseKeyOf(r))) return false;
      if (typeFilters.size > 0 && !typeFilters.has(getExt(r.filename) || "other")) return false;
      if (sourceFilters.size > 0 && !sourceFilters.has(r.source || "other")) return false;
      if (cutoff > 0) {
        const ts = Number(r.downloaded_at || 0);
        if (!ts || ts < cutoff) return false;
      }
      if (q) {
        const haystack = ((r.filename || "") + " " + (r.course_name || "") + " " + sourceLabel(r.source || "")).toLowerCase();
        if (haystack.indexOf(q) === -1) return false;
      }
      return true;
    });
    return sortRecords(out);
  });

  // ── Grouping ──
  function groupByDate(records: DownloadRecord[]): FileGroup[] {
    const now = new Date();
    const todayStr = now.toDateString();
    const yesterday = new Date(now.getTime() - 86400000).toDateString();
    const buckets = { today: [] as DownloadRecord[], yesterday: [] as DownloadRecord[], thisWeek: [] as DownloadRecord[], thisMonth: [] as DownloadRecord[], older: [] as DownloadRecord[] };
    for (const r of records) {
      const ts = Number(r.downloaded_at || 0);
      const d = new Date(ts);
      const ds = d.toDateString();
      if (ds === todayStr) buckets.today.push(r);
      else if (ds === yesterday) buckets.yesterday.push(r);
      else if (now.getTime() - ts < 7 * 86400000) buckets.thisWeek.push(r);
      else if (now.getFullYear() === d.getFullYear() && now.getMonth() === d.getMonth()) buckets.thisMonth.push(r);
      else buckets.older.push(r);
    }
    const result: FileGroup[] = [];
    if (buckets.today.length) result.push({ label: "今日", items: buckets.today });
    if (buckets.yesterday.length) result.push({ label: "昨日", items: buckets.yesterday });
    if (buckets.thisWeek.length) result.push({ label: "今週", items: buckets.thisWeek });
    if (buckets.thisMonth.length) result.push({ label: "今月", items: buckets.thisMonth });
    if (buckets.older.length) result.push({ label: "それ以前", items: buckets.older });
    return result;
  }
  function groupByCourse(records: DownloadRecord[]): FileGroup[] {
    const map = new Map<string, DownloadRecord[]>();
    for (const r of records) {
      const c = courseKeyOf(r);
      if (!map.has(c)) map.set(c, []);
      map.get(c)!.push(r);
    }
    return Array.from(map, ([label, items]) => ({ label, items }));
  }
  function groupByType(records: DownloadRecord[]): FileGroup[] {
    const map = new Map<string, DownloadRecord[]>();
    for (const r of records) {
      const ext = getExt(r.filename) || "other";
      if (!map.has(ext)) map.set(ext, []);
      map.get(ext)!.push(r);
    }
    return Array.from(map, ([ext, items]) => ({ label: typeLabel(ext), items }));
  }
  // Mirror the organizer's on-disk structure: within a single course, bucket by
  // the theme subfolder (第NN回 / 課題 / 教材 …) the file was filed into. Files
  // still loose at the course root fall into a trailing "未整理" group.
  const ROOT_GROUP = "未整理";
  function groupBySubfolder(records: DownloadRecord[]): FileGroup[] {
    const map = new Map<string, DownloadRecord[]>();
    for (const r of records) {
      const key = (r.subfolder || "").trim() || ROOT_GROUP;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(r);
    }
    return Array.from(map, ([label, items]) => ({ label, items })).sort((a, b) => {
      if (a.label === ROOT_GROUP) return 1;
      if (b.label === ROOT_GROUP) return -1;
      return a.label.localeCompare(b.label, "ja", { numeric: true });
    });
  }
  let groups = $derived.by<FileGroup[]>(() => {
    const records = filteredRecords;
    const oneCourse = courseFilters.size === 1;
    if (oneCourse) {
      // Adapt to the structure: theme-folder view when the course is organized,
      // else keep the by-type grouping for an unorganized (flat) course.
      const hasThemes = records.some((r) => (r.subfolder || "").trim());
      return hasThemes ? groupBySubfolder(records) : groupByType(records);
    }
    if (sortKey === "date") return groupByDate(records);
    return groupByCourse(records);
  });

  // ── Selection ──
  let selectedRecords = $derived(allRecords.filter((r) => selectedIds.has(r.id)));
  let selectedExisting = $derived(selectedRecords.filter((r) => r.file_exists !== false && r.path));
  function toggleSelect(id: string, checked: boolean): void {
    const next = new Set(selectedIds);
    if (checked) next.add(id);
    else next.delete(id);
    selectedIds = next;
    bulkDeleteArmed = false;
  }
  function selectAllVisible(): void {
    const next = new Set(selectedIds);
    for (const r of filteredRecords) next.add(r.id);
    selectedIds = next;
    bulkDeleteArmed = false;
  }
  function clearSelection(): void {
    selectedIds = new Set();
    bulkDeleteArmed = false;
  }
  function pruneSelection(): void {
    const known = new Set(allRecords.map((r) => r.id));
    let changed = false;
    const next = new Set(selectedIds);
    for (const id of next) {
      if (!known.has(id)) {
        next.delete(id);
        changed = true;
      }
    }
    if (changed) selectedIds = next;
  }

  // ── Status ──
  let baseStatus = $derived.by(() => {
    if (loading) return "読み込み中…";
    const parts = [filteredRecords.length + "件のファイル"];
    const missing = filteredRecords.filter((r) => r.file_exists === false).length;
    if (missing > 0) parts.push(missing + "件欠落");
    return parts.join(" · ");
  });
  let statusSize = $derived.by(() => {
    const total = filteredRecords.reduce((sum, r) => sum + (r.size_bytes || 0), 0);
    return total ? "合計 " + formatSize(total) : "";
  });
  let sortDirLabel = $derived(
    sortDir === "asc"
      ? sortKey === "date" ? "古い順" : sortKey === "size" ? "小さい順" : "昇順"
      : sortKey === "date" ? "新しい順" : sortKey === "size" ? "大きい順" : "降順",
  );

  // ── Data loading ──
  function applyRecords(records: DownloadRecord[]): void {
    if (!resources.active) return;
    allRecords = records;
    reconcilePreviewRecords(records);
    loading = false;
    pruneSelection();
    applyPendingFocus();
  }
  const recordsRead = new LatestViewRead(resources,
    () => invoke<DownloadRecord[]>("list_downloads"),
    applyRecords,
    (e) => { loading = false; statusMessage = `読み込み失敗: ${String(e)}`; });
  const refreshRecords = createCacheSyncQueue(async () => {
    if (mutating) { reloadAfterMutation = true; return; }
    await recordsRead.refresh().catch(() => {});
  });
  function loadDownloads(): Promise<void> {
    if (!resources.active) return Promise.resolve();
    if (mutating) { reloadAfterMutation = true; return Promise.resolve(); }
    return refreshRecords(["downloads"]);
  }
  async function mutate(work: () => Promise<void>): Promise<void> {
    if (!resources.active || mutating) return;
    mutating = true;
    recordsRead.invalidate();
    try { await work(); }
    finally {
      recordsRead.invalidate();
      if (resources.active) {
        mutating = false;
        loading = false;
        if (reloadAfterMutation) {
          reloadAfterMutation = false;
          void loadDownloads();
        }
        if (reloadDuplicatesAfterMutation && dupModalOpen) {
          reloadDuplicatesAfterMutation = false;
          void refreshDuplicates();
        }
      }
    }
  }
  function removeSubmittedSelection(ids: Iterable<string>): void {
    const next = new Set(selectedIds);
    for (const id of ids) next.delete(id);
    selectedIds = next;
  }
  function applyPendingFocus(): void {
    if (!pendingFocusCourse) return;
    const key = pendingFocusCourse.trim();
    pendingFocusCourse = "";
    if (!key) return;
    const exact = allRecords.some((r) => (r.course_name || "").trim() === key);
    if (exact) {
      courseFilters = new Set([key]);
      return;
    }
    const found = allRecords.find((r) => (r.course_name || "").trim().toLowerCase() === key.toLowerCase());
    courseFilters = new Set([found ? found.course_name : key]);
  }

  // ── Actions ──
  async function openFile(path: string): Promise<void> {
    if (!resources.active) return;
    try {
      await invoke("open_downloaded_file", { path });
    } catch (e) {
      console.error("Failed to open file:", e);
    }
  }
  async function revealFile(path: string): Promise<void> {
    if (!resources.active) return;
    try {
      await invoke("luna_reveal_file", { path });
    } catch (e) {
      console.error("Failed to reveal file:", e);
    }
  }
  async function removeRecord(id: string, deletedFilename?: string): Promise<void> {
    await mutate(async () => {
      try {
        await invoke("remove_download_record", { id });
        if (!resources.active) return;
        applyRecords(allRecords.filter((r) => r.id !== id));
        removeSubmittedSelection([id]);
        if (deletedFilename) statusMessage = "「" + deletedFilename + "」を履歴から削除しました";
      } catch (e) {
        if (resources.active) statusMessage = "履歴削除に失敗しました: " + e;
      }
    });
  }
  // A click pins the existing double-click hint; a double click submits removal.
  let deleteHintId = $state<string | null>(null);
  let cancelDeleteHint = () => {};
  function pinDeleteHint(id: string): void {
    if (!resources.active || mutating) return;
    deleteHintId = id;
    cancelDeleteHint();
    cancelDeleteHint = resources.schedule(() => { deleteHintId = null; }, 2500);
  }
  async function deleteEntry(r: DownloadRecord): Promise<void> {
    if (!resources.active || mutating) return;
    const id = r.id, path = r.path, filename = r.filename;
    deleteHintId = null;
    cancelDeleteHint();
    if (r.file_exists === false || !path) {
      await removeRecord(id, filename);
      return;
    }
    await mutate(async () => {
      try {
        const result = await invoke<BulkResult>("delete_downloaded_files", { paths: [path] });
        if (!resources.active) return;
        await loadDownloads();
        removeSubmittedSelection([id]);
        statusMessage = bulkResultMessage(result, (result.failed_count ?? 0) > 0
          ? "「" + filename + "」の削除に失敗しました"
          : "「" + filename + "」を削除しました");
      } catch (e) {
        if (resources.active) statusMessage = "削除に失敗しました: " + e;
      }
    });
  }
  async function deleteSelectedRecords(): Promise<void> {
    const ids = Array.from(selectedIds);
    if (!ids.length) return;
    await mutate(async () => {
      try {
        await invoke("remove_download_records", { ids });
        if (!resources.active) return;
        const submitted = new Set(ids);
        applyRecords(allRecords.filter((r) => !submitted.has(r.id)));
        removeSubmittedSelection(ids);
        bulkDeleteArmed = false;
        statusMessage = ids.length + "件を履歴から削除しました";
      } catch (e) {
        if (resources.active) statusMessage = "履歴削除に失敗しました: " + e;
      }
    });
  }
  let sharing = $state(false);
  async function shareSelectedFiles(): Promise<void> {
    if (!resources.active || sharing || mutating) return;
    const paths = selectedExisting.map((r) => r.path);
    if (!paths.length) return;
    sharing = true;
    try {
      await invoke("share_downloaded_files_native", { paths });
      if (resources.active) statusMessage = paths.length + "件を共有に送信しました";
    } catch (e) {
      if (resources.active) statusMessage = "共有に失敗しました: " + e;
    } finally {
      if (resources.active) sharing = false;
    }
  }
  let deletingFiles = $state(false);
  async function deleteSelectedFiles(): Promise<void> {
    if (!resources.active || mutating) return;
    const selected = selectedExisting;
    const paths = selected.map((r) => r.path), ids = selected.map((r) => r.id);
    if (!paths.length) return;
    if (!bulkDeleteArmed) {
      bulkDeleteArmed = true;
      statusMessage = paths.length + "件のファイルを削除します";
      return;
    }
    bulkDeleteArmed = false;
    await mutate(async () => {
      deletingFiles = true;
      try {
        const result = await invoke<BulkResult>("delete_downloaded_files", { paths });
        if (!resources.active) return;
        await loadDownloads();
        removeSubmittedSelection(ids);
        const deleted = result.deleted_count ?? 0, failed = result.failed_count ?? 0;
        statusMessage = bulkResultMessage(result, failed > 0 ? deleted + "件削除、" + failed + "件失敗" : deleted + "件のファイルを削除しました");
      } catch (e) {
        if (resources.active) statusMessage = "ファイル削除に失敗しました: " + e;
      } finally {
        if (resources.active) deletingFiles = false;
      }
    });
  }
  async function clearHistory(): Promise<void> {
    await mutate(async () => {
      try {
        await invoke("clear_download_history");
        if (resources.active) applyRecords([]);
      } catch (e) {
        if (resources.active) statusMessage = "履歴削除に失敗しました: " + e;
      }
    });
  }
  let scanning = $state(false);
  async function scanDir(): Promise<void> {
    await mutate(async () => {
      scanning = true;
      try {
        const before = allRecords.length;
        const records = await invoke<DownloadRecord[]>("scan_download_dir");
        if (!resources.active) return;
        applyRecords(records);
        const added = records.length - before;
        statusMessage = added > 0 ? added + "件の新しいファイルを検出しました" : "新しいファイルは見つかりませんでした";
      } catch (e) {
        if (resources.active) statusMessage = "スキャンに失敗しました: " + e;
      } finally {
        if (resources.active) scanning = false;
      }
    });
  }

  // ── Duplicate manager ──
  let dupModalOpen = $state(false);
  let dupScanning = $state(false);
  let duplicateGroups = $state<DuplicateGroup[]>([]);
  let dupCleanupArmed = $state(false);
  let dupCleaning = $state(false);
  let dupError = $state("");
  let dupFootOverride = $state("");
  let duplicateVersion = 0;
  let reloadDuplicatesAfterMutation = false;
  const duplicatesRead = new LatestViewRead(resources,
    () => invoke<DuplicateGroup[]>("scan_duplicate_downloads"),
    (groups) => { if (dupModalOpen) duplicateGroups = groups; },
    (e) => { if (dupModalOpen) dupError = String(e); });
  const queueDuplicates = createCacheSyncQueue(async () => {
    if (!resources.active || !dupModalOpen) return;
    if (mutating) { reloadDuplicatesAfterMutation = true; return; }
    dupScanning = true;
    try { await duplicatesRead.refresh().catch(() => {}); }
    finally { if (resources.active) dupScanning = false; }
  });
  function refreshDuplicates(): Promise<void> {
    if (!resources.active || !dupModalOpen) return Promise.resolve();
    return queueDuplicates(["duplicates"]);
  }

  function isDuplicateKeep(item: DuplicateItem): boolean {
    return item.is_recommended === true;
  }
  function duplicateFileExists(item: DuplicateItem): boolean {
    return item.file_exists !== false;
  }
  let dupCleanupPaths = $derived.by<string[]>(() => {
    const paths: string[] = [];
    for (const g of duplicateGroups) {
      for (const item of g.items || []) {
        if (!isDuplicateKeep(item) && duplicateFileExists(item)) paths.push(item.path);
      }
    }
    return paths;
  });
  let dupWasteBytes = $derived.by(() =>
    duplicateGroups.reduce((sum, g) => {
      const count = (g.items || []).filter((item) => !isDuplicateKeep(item) && duplicateFileExists(item)).length;
      return sum + Number(g.size_bytes || 0) * count;
    }, 0),
  );
  function openDuplicateModal(): void {
    dupModalOpen = true;
  }
  function closeDuplicateModal(): void {
    duplicateVersion += 1;
    duplicatesRead.invalidate();
    dupModalOpen = false;
    dupScanning = false;
    dupCleanupArmed = false;
    reloadDuplicatesAfterMutation = false;
  }
  function onWindowKeydown(e: KeyboardEvent): void {
    if (e.key !== "Escape") return;
    if (dupModalOpen) closeDuplicateModal();
    else if (openMenu) openMenu = null;
  }
  async function scanDuplicates(): Promise<void> {
    if (!resources.active) return;
    dupCleanupArmed = false;
    dupError = "";
    dupFootOverride = "";
    dupModalOpen = true;
    await refreshDuplicates();
  }
  async function cleanupDuplicates(): Promise<void> {
    if (!resources.active || mutating || dupScanning) return;
    const paths = [...dupCleanupPaths];
    if (!paths.length) return;
    if (!dupCleanupArmed) {
      dupCleanupArmed = true;
      dupFootOverride = paths.length + "件を削除します。各グループの「保留」は残ります";
      return;
    }
    const version = duplicateVersion;
    dupCleanupArmed = false;
    duplicatesRead.invalidate();
    await mutate(async () => {
      dupCleaning = true;
      try {
        const result = await invoke<BulkResult>("cleanup_duplicate_downloads", { paths });
        if (!resources.active) return;
        await loadDownloads();
        await refreshDuplicates();
        const deleted = result.deleted_count ?? 0, failed = result.failed_count ?? 0;
        statusMessage = bulkResultMessage(result, failed > 0 ? deleted + "件削除、" + failed + "件失敗" : deleted + "件の重複ファイルを削除しました");
        if (dupModalOpen && version === duplicateVersion) {
          dupFootOverride = result.errors?.slice(0, 2).join(" / ") || "";
        }
      } catch (e) {
        if (resources.active && dupModalOpen && version === duplicateVersion) dupFootOverride = String(e);
      } finally {
        if (resources.active) dupCleaning = false;
      }
    });
  }
  let dupFootText = $derived.by(() => {
    if (dupScanning) return "スキャン中…";
    if (dupError) return dupError;
    if (dupFootOverride) return dupFootOverride;
    if (!duplicateGroups.length) return "整理不要";
    return dupCleanupPaths.length + "件を削除候補として選択中";
  });
  let dupSummary = $derived.by(() => {
    if (dupScanning) return "内容ハッシュを計算しています";
    if (dupError) return "重複スキャンに失敗しました";
    if (!duplicateGroups.length) return "内容が重複しているファイルはありません";
    return duplicateGroups.length + "組の重複 · 削減候補 " + formatSize(dupWasteBytes);
  });

  // ── Lazy preview loading (icons view) ──
  let fileListEl = $state<HTMLElement | null>(null);
  let previewMap = $state<Record<string, DownloadPreview | null>>({});
  const previewController = new FilePreviewController(
    path => invoke<DownloadPreview | null>("get_download_preview", { path }),
    (key, preview) => {
      if (!resources.active) return;
      if (preview === undefined) delete previewMap[key];
      else previewMap[key] = preview;
    },
  );
  let previewObserver: IntersectionObserver | null = null;
  type PreviewTarget = { request: FilePreviewRequest; visible: boolean; invalidated: boolean; since: number; release: () => void };
  const previewTargets = new Map<HTMLElement, PreviewTarget>();

  function reconcilePreviewRecords(records: DownloadRecord[]): void {
    const keys = new Set(records.map(r => filePreviewRequest(r).key));
    previewController.prune(keys);
    for (const target of previewTargets.values()) {
      if (keys.has(target.request.key)) continue;
      target.release(); target.release = () => {};
      target.visible = false; target.invalidated = true;
    }
  }
  function previewEntries(entries: IntersectionObserverEntry[]): void {
    if (!resources.active) return;
    for (const entry of entries) {
      const consumer = previewTargets.get(entry.target as HTMLElement);
      if (!consumer || (typeof entry.time === "number" && entry.time < consumer.since)) continue;
      const visible = entry.isIntersecting && entry.target.isConnected && viewMode === "icons";
      if (visible === consumer.visible) continue;
      consumer.visible = visible;
      consumer.release();
      consumer.release = visible ? previewController.retain(consumer.request) : () => {};
    }
  }
  function flushPreviewEntries(except: HTMLElement): void {
    // Drain queued observations before reusing a node for a new file version.
    const queued = previewObserver?.takeRecords();
    if (queued?.length) previewEntries(queued.filter(entry => entry.target !== except));
  }
  function lazyPreview(node: HTMLElement, request: FilePreviewRequest) {
    let current: PreviewTarget | undefined;
    function setup(next: FilePreviewRequest): void {
      if (!resources.active || (current?.request.key === next.key && current.request.path === next.path && !current.invalidated)) return;
      flushPreviewEntries(node);
      previewObserver?.unobserve(node);
      current?.release();
      current = { request: next, visible: false, invalidated: false, since: performance.now(), release: () => {} };
      previewTargets.set(node, current);
      node.dataset.previewPath = next.path;
      if (!next.path) return;
      if (!previewObserver) {
        previewObserver = new IntersectionObserver(resources.guard(previewEntries), { root: fileListEl, rootMargin: "160px" });
      }
      // Keep observing after loading so scrolling/filtering releases the image.
      previewObserver.observe(node);
    }
    setup(request);
    return {
      update: setup,
      destroy() {
        flushPreviewEntries(node);
        previewObserver?.unobserve(node);
        current?.release();
        previewTargets.delete(node);
      },
    };
  }

  // ── Event wiring ──
  function setPeriod(p: Period): void {
    period = p;
    statusMessage = "";
    savePrefs();
  }
  function toggleMissing(): void {
    hideMissing = !hideMissing;
    statusMessage = "";
    savePrefs();
  }
  function setViewMode(mode: ViewMode): void {
    if (!resources.active || viewMode === mode) return;
    viewMode = mode;
    if (mode !== "icons") {
      for (const target of previewTargets.values()) {
        target.release(); target.release = () => {}; target.visible = false;
      }
      previewController.clearConsumers();
    } else {
      // A list → icons change in one tick can reuse the same DOM nodes. Re-arm
      // observation so those nodes receive an initial visible notification.
      previewObserver?.takeRecords();
      for (const [node, target] of previewTargets) {
        target.since = performance.now();
        previewObserver?.unobserve(node);
        previewObserver?.observe(node);
      }
    }
    savePrefs();
  }
  function setSortKey(key: SortKey): void {
    sortKey = key;
    statusMessage = "";
    savePrefs();
  }
  function setSortDir(dir: SortDir): void {
    sortDir = dir;
    statusMessage = "";
    savePrefs();
  }
  // ── Document-tabs control row (second row) ──
  function toggleViewMode(): void {
    setViewMode(viewMode === "icons" ? "list" : "icons");
  }
  const MENU_WIDTH = 176;
  function anchorMenu(payload: unknown): void {
    const x = (payload as { x?: number } | null)?.x;
    if (typeof x !== "number") return;
    // Align the popup's left edge with the trigger button, clamped to the
    // viewport so it never spills off the right edge.
    menuX = Math.max(8, Math.min(x, window.innerWidth - MENU_WIDTH - 8));
  }
  function selectPeriod(p: Period): void {
    setPeriod(p);
    openMenu = null;
  }
  // The tab strip is a separate 88px webview, so a dropdown there would be
  // clipped — the toolbar buttons instead toggle a popup rendered inside this
  // (full-size) surface webview.
  function buildControls(): Record<string, unknown>[] {
    return [
      { id: "view", label: viewMode === "icons" ? "アイコン" : "リスト", action: "files.toggleView", icon: viewMode === "icons" ? "square.grid.2x2" : "sidebar", group: "view" },
      { id: "period", label: "期間", value: PERIOD_LABELS[period], action: "files.togglePeriodMenu", icon: "calendar", active: openMenu === "period" || period !== "all", group: "view" },
      { id: "missing", label: hideMissing ? "欠落を表示" : "欠落を非表示", action: "files.toggleMissing", icon: "exclamationmark.triangle", active: !hideMissing, group: "view" },
      { id: "sort", label: "並び替え", value: SORT_KEY_LABELS[sortKey], action: "files.toggleSortMenu", icon: "arrow.triangle.swap", active: openMenu === "sort", group: "sort" },
      { id: "scan", label: scanning ? "スキャン中" : "スキャン", action: "files.scan", icon: "folder.open", disabled: scanning || mutating, group: "file" },
      { id: "dup", label: "重複整理", action: "files.duplicates", icon: "doc", group: "file" },
      { id: "clear", label: "履歴クリア", action: "files.clearHistory", icon: "trash", disabled: mutating || allRecords.length === 0, group: "file" },
    ];
  }
  function pushControls(): void {
    if (!resources.active || !tabTarget) return;
    invoke("document_tabs_set_controls", { owner: tabOwner, target: tabTarget, controls: buildControls() }).catch(() => {});
  }
  function isCurrentControlEvent(control: ControlEvent | undefined): boolean {
    return control?.owner === tabOwner && control?.target === tabTarget;
  }
  function handleControl(event: { payload: ControlEvent }): void {
    const control = event.payload;
    if (!isCurrentControlEvent(control)) return;
    const action = control.action;
    // Any toolbar action other than opening a menu dismisses an open popup.
    if (action !== "files.togglePeriodMenu" && action !== "files.toggleSortMenu") openMenu = null;
    switch (action) {
      case "files.search":
        searchQuery = String(control.payload ?? "").toLowerCase().trim();
        statusMessage = "";
        break;
      case "files.toggleView": toggleViewMode(); break;
      case "files.togglePeriodMenu":
        anchorMenu(control.payload);
        openMenu = openMenu === "period" ? null : "period";
        break;
      case "files.toggleMissing": toggleMissing(); break;
      case "files.toggleSortMenu":
        anchorMenu(control.payload);
        openMenu = openMenu === "sort" ? null : "sort";
        break;
      case "files.scan": void scanDir(); break;
      case "files.duplicates": void scanDuplicates(); break;
      case "files.clearHistory": void clearHistory(); break;
    }
  }

  $effect(() => {
    // Re-push the control row whenever any reflected state changes.
    void [viewMode, period, hideMissing, sortKey, sortDir, sortDirLabel, scanning, mutating, allRecords.length, openMenu];
    pushControls();
  });

  $effect(() => {
    // Keep selection consistent whenever the record set changes.
    allRecords;
    pruneSelection();
  });

  async function initializeFiles(): Promise<void> {
    await acquireResourceGroup(resources, [
      group => listen<string>("theme-changed", group.guard((event) => {
        themeVersion += 1;
        applyAuxiliaryTheme(event.payload);
      })),
      group => listen("app-theme-changed", group.guard(() => { void refreshTheme(["theme"]); })),
      group => listen<string>("focus-course", group.guard((event) => {
        const name = String(event.payload || "").trim();
        if (!name) { pendingFocusCourse = ""; courseFilters = new Set(); return; }
        pendingFocusCourse = name;
        // Keep the request pending while the first list is still loading.
        if (!loading) applyPendingFocus();
      })),
      group => listen<ControlEvent>("document-tab-control", group.guard(handleControl), { target: tabTarget }),
    ]);
    if (!resources.active) return;
    pushControls();
    void refreshTheme(["theme"]);
    await loadDownloads();
  }
  onMount(() => {
    document.documentElement.setAttribute("data-aux-surface", "files");
    document.body.setAttribute("data-aux-surface", "files");
    loadPrefs();
    void initializeFiles().catch((e) => {
      if (resources.active) { loading = false; statusMessage = `初期化失敗: ${String(e)}`; }
    });
  });
  onDestroy(() => {
    resources.dispose();
    previewObserver?.disconnect();
    previewObserver = null;
    for (const target of previewTargets.values()) target.release();
    previewTargets.clear();
    previewController.dispose();
    previewMap = {};
    document.documentElement.removeAttribute("data-aux-surface");
    document.body.removeAttribute("data-aux-surface");
    if (tabTarget) invoke("document_tabs_set_controls", { owner: tabOwner, target: tabTarget, controls: [] }).catch(() => {});
  });
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="files-root">
  <div class="layout">
    <aside class="sidebar">
      <div class="sidebar-section">
        <div class="sidebar-label">表示</div>
        <button class="nav-item" class:selected={!hasActiveFilters} onclick={clearAllFilters}>
          <span class="nav-check"><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
          <span class="nav-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><path d="M8 12l4 4 4-4" /><line x1="12" y1="8" x2="12" y2="16" /></svg></span>
          <span class="nav-label">すべて</span>
          <span class="nav-count">{allRecords.length}</span>
        </button>
      </div>

      <div class="sidebar-section">
        <div class="sidebar-label">
          <span>コース</span>
          {#if courseFilters.size > 0}<button class="sidebar-clear" onclick={() => clearKind("course")}>クリア</button>{/if}
        </div>
        {#each courseKeys as key (key)}
          <button class="nav-item" class:selected={courseFilters.has(key)} onclick={() => toggleFilter("course", key)}>
            <span class="nav-check"><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
            <span class="nav-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z" /></svg></span>
            <span class="nav-label" title={key}>{key}</span>
            <span class="nav-count">{courseCounts[key]}</span>
          </button>
        {/each}
      </div>

      <div class="sidebar-section">
        <div class="sidebar-label">
          <span>ファイル形式</span>
          {#if typeFilters.size > 0}<button class="sidebar-clear" onclick={() => clearKind("type")}>クリア</button>{/if}
        </div>
        {#each typeKeys as key (key)}
          <button class="nav-item" class:selected={typeFilters.has(key)} onclick={() => toggleFilter("type", key)}>
            <span class="nav-check"><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
            <span class="nav-icon" style="color:{(FILE_TYPE_MAP[key] || { color: '#95a5a6' }).color}"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /></svg></span>
            <span class="nav-label" title={typeLabel(key)}>{typeLabel(key)}</span>
            <span class="nav-count">{typeCounts[key]}</span>
          </button>
        {/each}
      </div>

      <div class="sidebar-section">
        <div class="sidebar-label">
          <span>ソース</span>
          {#if sourceFilters.size > 0}<button class="sidebar-clear" onclick={() => clearKind("source")}>クリア</button>{/if}
        </div>
        {#each sourceKeys as key (key)}
          <button class="nav-item" class:selected={sourceFilters.has(key)} onclick={() => toggleFilter("source", key)}>
            <span class="nav-check"><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
            <span class="nav-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="2" y1="12" x2="22" y2="12" /><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" /></svg></span>
            <span class="nav-label" title={sourceLabel(key)}>{sourceLabel(key)}</span>
            <span class="nav-count">{sourceCounts[key]}</span>
          </button>
        {/each}
      </div>
    </aside>

    <div class="main">
      {#if hasActiveFilters}
        <div class="filter-bar">
          <span class="filter-bar-label">絞込</span>
          <div class="filter-pills">
            {#each Array.from(courseFilters) as v (v)}
              <span class="filter-pill"><span class="pill-kind">コース</span><span>{v}</span><button class="filter-pill-x" title="解除" onclick={() => toggleFilter("course", v)}><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg></button></span>
            {/each}
            {#each Array.from(typeFilters) as v (v)}
              <span class="filter-pill"><span class="pill-kind">形式</span><span>{typeLabel(v)}</span><button class="filter-pill-x" title="解除" onclick={() => toggleFilter("type", v)}><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg></button></span>
            {/each}
            {#each Array.from(sourceFilters) as v (v)}
              <span class="filter-pill"><span class="pill-kind">ソース</span><span>{sourceLabel(v)}</span><button class="filter-pill-x" title="解除" onclick={() => toggleFilter("source", v)}><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg></button></span>
            {/each}
          </div>
          <button class="filter-clear-all" onclick={clearAllFilters}>すべて解除</button>
        </div>
      {/if}

      {#if selectedRecords.length > 0}
        <div class="selection-bar">
          <span class="selection-summary">{selectedRecords.length}件選択中</span>
          <div class="selection-actions">
            <button class="toolbar-btn" onclick={selectAllVisible}>表示中を選択</button>
            <button class="toolbar-btn" onclick={clearSelection}>選択解除</button>
            <button class="toolbar-btn" disabled={selectedExisting.length === 0 || sharing || mutating} onclick={shareSelectedFiles}>{sharing ? "共有中…" : "共有"}</button>
            <button class="toolbar-btn" disabled={mutating} onclick={deleteSelectedRecords}>履歴から削除</button>
            <button class="toolbar-btn danger" disabled={selectedExisting.length === 0 || deletingFiles || mutating} onclick={deleteSelectedFiles}>
              {deletingFiles ? "削除中…" : bulkDeleteArmed ? "もう一度押して削除" : "ファイルを削除"}
            </button>
          </div>
        </div>
      {/if}

      <div class="file-list" bind:this={fileListEl}>
        {#if loading}
          <div class="files-skeleton" aria-busy="true" aria-label="読み込み中">
            {#each [6, 4] as rowCount, g (g)}
              <div class="fsk-group">
                <div class="fsk fsk-group-header"></div>
                {#each Array(rowCount) as _, i (i)}
                  <div class="fsk-row">
                    <div class="fsk fsk-icon"></div>
                    <div class="fsk-row-text">
                      <div class="fsk fsk-name" style={`width:${72 - ((i * 11) % 34)}%`}></div>
                      <div class="fsk fsk-meta"></div>
                    </div>
                  </div>
                {/each}
              </div>
            {/each}
          </div>
        {:else if filteredRecords.length === 0}
          {#if allRecords.length === 0}
            <div class="empty-state">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><path d="M8 12l4 4 4-4" /><line x1="12" y1="8" x2="12" y2="16" /></svg>
              <div class="empty-state-text">ダウンロード履歴はありません</div>
            </div>
          {:else}
            <div class="empty-state">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
              <div class="empty-state-text">条件に一致するファイルがありません</div>
            </div>
          {/if}
        {:else}
          {#each groups as group (group.label)}
            <div class="group">
              <div class="group-header">
                <span class="group-header-name" title={group.label}>{group.label}</span>
                <span class="group-count">{group.items.length}件</span>
              </div>
              <div class="group-body" class:file-grid={viewMode === "icons"}>
                {#each group.items as r (r.id)}
                  {@const info = getTypeInfo(r.filename)}
                  {@const source = sourceLabel(r.source || "")}
                  {@const missing = r.file_exists === false}
                  {#if viewMode === "icons"}
                    <div class="file-card" class:missing data-id={r.id}>
                      <label class="file-select" title="選択"><input type="checkbox" checked={selectedIds.has(r.id)} onclick={(e) => e.stopPropagation()} onchange={(e) => toggleSelect(r.id, e.currentTarget.checked)} /></label>
                      {#if missing || !canPreviewFile(r.filename)}
                        <div class="file-card-icon" style="background:{info.color}">{info.label}</div>
                      {:else}
                        {@const request = filePreviewRequest(r)}
                        {@const preview = previewMap[request.key]}
                        <div class="file-card-preview" class:pending={preview === undefined} use:lazyPreview={request}>
                          {#if preview === undefined || preview === null}
                            <div class="file-card-preview fallback" style="background:{info.color}">{info.label}</div>
                          {:else if preview.kind === "image" && (preview.data_url)}
                            <img src={preview.data_url} alt="" />
                          {:else if preview.kind === "text" && preview.text}
                            <div class="file-card-preview-text">{preview.text}</div>
                          {:else}
                            <div class="file-card-preview fallback" style="background:{info.color}">{info.label}</div>
                          {/if}
                        </div>
                      {/if}
                      {#if missing}
                        <div class="file-card-name" title={r.filename}>{r.filename}</div>
                      {:else}
                        <div class="file-card-name" role="button" tabindex="0" title={"クリックして開く: " + r.filename} onclick={() => openFile(r.path)} onkeydown={(e) => { if (e.key === "Enter") openFile(r.path); }}>{r.filename}</div>
                      {/if}
                      <div class="file-card-meta">
                        {#if missing}<span class="missing-badge">なし</span>{:else}<span>{formatSize(r.size_bytes)}</span>{/if}
                        {#if source}<span>·</span><span>{source}</span>{/if}
                      </div>
                      <div class="file-card-actions">
                        {#if !missing}
                          <button class="action-btn" title="Finderで表示" onclick={() => revealFile(r.path)}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg></button>
                        {/if}
                        <span class="action-tip" class:pinned={deleteHintId === r.id}><button class="action-btn remove" aria-label="削除" disabled={mutating} onclick={() => pinDeleteHint(r.id)} ondblclick={() => deleteEntry(r)}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg></button><span class="action-tip-bubble">ダブルクリックで削除</span></span>
                      </div>
                    </div>
                  {:else}
                    <div class="file-row" class:missing data-id={r.id}>
                      <label class="file-select" title="選択"><input type="checkbox" checked={selectedIds.has(r.id)} onclick={(e) => e.stopPropagation()} onchange={(e) => toggleSelect(r.id, e.currentTarget.checked)} /></label>
                      <div class="file-icon" style="background:{info.color}">{info.label}</div>
                      <div class="file-info">
                        {#if missing}
                          <div class="file-name">{r.filename}</div>
                        {:else}
                          <div class="file-name" role="button" tabindex="0" title={"クリックして開く: " + r.filename} onclick={() => openFile(r.path)} onkeydown={(e) => { if (e.key === "Enter") openFile(r.path); }}>{r.filename}</div>
                        {/if}
                        <div class="file-meta">
                          {#if missing}<span class="missing-badge">ファイルなし</span>{/if}
                          {#if r.course_name}<span>{r.course_name}</span><span>·</span>{/if}
                          <span>{formatSize(r.size_bytes)}</span>
                          <span>·</span><span>{formatTime(r.downloaded_at)}</span>
                          {#if source}<span>·</span><span class="file-meta-tag">{source}</span>{/if}
                        </div>
                      </div>
                      <div class="file-actions">
                        {#if !missing}
                          <button class="action-btn" title="Finderで表示" onclick={() => revealFile(r.path)}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg></button>
                        {/if}
                        <span class="action-tip" class:pinned={deleteHintId === r.id}><button class="action-btn remove" aria-label="削除" disabled={mutating} onclick={() => pinDeleteHint(r.id)} ondblclick={() => deleteEntry(r)}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg></button><span class="action-tip-bubble">ダブルクリックで削除</span></span>
                      </div>
                    </div>
                  {/if}
                {/each}
              </div>
            </div>
          {/each}
        {/if}
      </div>

      <div class="status-bar">
        <span>{statusMessage || baseStatus}</span>
        <span>{statusSize}</span>
      </div>
    </div>
  </div>

  {#if openMenu}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="menu-backdrop" role="presentation" onclick={() => (openMenu = null)}></div>
    <div class="popmenu" role="menu" tabindex="-1" style="left:{menuX}px;">
      {#if openMenu === "period"}
        {#each PERIOD_ORDER as p (p)}
          <button class="popmenu-item" class:active={period === p} type="button" role="menuitemradio" aria-checked={period === p} onclick={() => selectPeriod(p)}>
            <span class="popmenu-check"><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
            <span>{PERIOD_LABELS[p]}</span>
          </button>
        {/each}
      {:else}
        <div class="popmenu-label">並び順</div>
        {#each SORT_KEY_ORDER as k (k)}
          <button class="popmenu-item" class:active={sortKey === k} type="button" role="menuitemradio" aria-checked={sortKey === k} onclick={() => setSortKey(k)}>
            <span class="popmenu-check"><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
            <span>{SORT_KEY_LABELS[k]}</span>
          </button>
        {/each}
        <div class="popmenu-divider"></div>
        {#each [["desc", "降順"], ["asc", "昇順"]] as [dir, label] (dir)}
          <button class="popmenu-item" class:active={sortDir === dir} type="button" role="menuitemradio" aria-checked={sortDir === dir} onclick={() => setSortDir(dir as SortDir)}>
            <span class="popmenu-check"><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg></span>
            <span>{label}</span>
          </button>
        {/each}
      {/if}
    </div>
  {/if}

  {#if dupModalOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="modal-backdrop open" role="presentation" onclick={(e) => { if (e.currentTarget === e.target) closeDuplicateModal(); }}>
      <div class="duplicate-modal" role="dialog" aria-modal="true" aria-label="重複ファイル整理">
        <div class="modal-head">
          <div class="modal-head-info">
            <div class="modal-title">重複ファイル整理</div>
            <div class="modal-subtitle">{dupSummary}</div>
          </div>
          <button class="modal-close" type="button" aria-label="閉じる" title="閉じる" onclick={closeDuplicateModal}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
          </button>
        </div>
        <div class="modal-body">
          {#if dupScanning}
            <div class="duplicate-empty"><div class="spinner"></div><strong>重複を確認中</strong><span>同じサイズのファイルだけを読み込んで照合します</span></div>
          {:else if dupError}
            <div class="duplicate-empty"><strong>スキャンできませんでした</strong><span>{dupError}</span></div>
          {:else if !duplicateGroups.length}
            <div class="duplicate-empty"><strong>重複ファイルは見つかりませんでした</strong><span>ファイル名が違う場合も内容ハッシュで確認済みです</span></div>
          {:else}
            {#each duplicateGroups as g, idx (g.content_hash + idx)}
              <div class="duplicate-group">
                <div class="duplicate-group-head"><span>重複 {idx + 1} · {formatSize(g.size_bytes)}</span><span title={g.content_hash}>{g.content_hash.slice(0, 12)}</span></div>
                {#each g.items as item (item.id)}
                  {@const itemSource = sourceLabel(item.source || "")}
                  <div class="duplicate-item">
                    <div class="duplicate-info">
                      <div class="duplicate-name" title={item.filename}>{item.filename}</div>
                      <div class="duplicate-path" title={item.path}>{item.path}</div>
                    </div>
                    <div class="duplicate-tags">
                      {#if isDuplicateKeep(item)}<span class="duplicate-tag keep">保留</span>{:else}<span class="duplicate-tag">削除候補</span>{/if}
                      {#if item.course_name}<span class="duplicate-tag">{item.course_name}</span>{/if}
                      {#if itemSource}<span class="duplicate-tag">{itemSource}</span>{/if}
                    </div>
                  </div>
                {/each}
              </div>
            {/each}
          {/if}
        </div>
        <div class="modal-foot">
          <span>{dupFootText}</span>
          <div class="modal-action-row">
            <button class="toolbar-btn" onclick={scanDuplicates}>再スキャン</button>
            <button class="toolbar-btn danger" disabled={dupCleanupPaths.length === 0 || dupCleaning || dupScanning || mutating} onclick={cleanupDuplicates}>
              {dupCleaning ? "削除中…" : dupCleanupArmed ? "もう一度押して削除" : "推奨以外を削除"}
            </button>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .files-root {
    --bg-primary: #ffffff;
    --bg-secondary: #f8f8fa;
    --bg-input: rgba(255, 255, 255, 0.9);
    --text-primary: #1d1d1f;
    --text-secondary: #6e6e73;
    --text-tertiary: #aeaeb2;
    --border: rgba(0, 0, 0, 0.08);
    --border-strong: rgba(0, 0, 0, 0.12);
    --accent: #002855;
    --blue: #007aff;
    --red: #ff3b30;
    --bg-hover: rgba(0, 0, 0, 0.04);
    --bg-active: rgba(0, 40, 85, 0.08);
    --shadow-card: 0 0.5px 1px rgba(0, 0, 0, 0.04), 0 1px 3px rgba(0, 0, 0, 0.04);
    color-scheme: light;
    position: relative;
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", "Noto Sans JP", sans-serif;
    font-size: 13px;
    line-height: 1.47;
    color: var(--text-primary);
    background: var(--bg-primary);
    -webkit-font-smoothing: antialiased;
    -webkit-user-select: none;
    user-select: none;
  }
  @media (prefers-color-scheme: dark) {
    :global(html:not([data-theme="light"])) .files-root {
      --bg-primary: #1c1c1e;
      --bg-secondary: #2c2c2e;
      --bg-input: rgba(58, 58, 60, 0.55);
      --text-primary: #f5f5f7;
      --text-secondary: #98989d;
      --text-tertiary: #636366;
      --border: rgba(255, 255, 255, 0.08);
      --border-strong: rgba(255, 255, 255, 0.12);
      --accent: #4a90d9;
      --blue: #0a84ff;
      --red: #ff453a;
      --bg-hover: rgba(255, 255, 255, 0.06);
      --bg-active: rgba(74, 144, 217, 0.12);
      --shadow-card: 0 0.5px 1px rgba(0, 0, 0, 0.2), 0 1px 3px rgba(0, 0, 0, 0.15);
      color-scheme: dark;
    }
  }
  :global(html[data-theme="dark"]) .files-root,
  :global(body[data-theme="dark"]) .files-root {
    --bg-primary: #1c1c1e;
    --bg-secondary: #2c2c2e;
    --bg-input: rgba(58, 58, 60, 0.55);
    --text-primary: #f5f5f7;
    --text-secondary: #98989d;
    --text-tertiary: #636366;
    --border: rgba(255, 255, 255, 0.08);
    --border-strong: rgba(255, 255, 255, 0.12);
    --accent: #4a90d9;
    --blue: #0a84ff;
    --red: #ff453a;
    --bg-hover: rgba(255, 255, 255, 0.06);
    --bg-active: rgba(74, 144, 217, 0.12);
    --shadow-card: 0 0.5px 1px rgba(0, 0, 0, 0.2), 0 1px 3px rgba(0, 0, 0, 0.15);
    color-scheme: dark;
  }

  :global(html[data-aux-surface="files"]),
  :global(body[data-aux-surface="files"]),
  :global(body[data-aux-surface="files"] #app) {
    height: 100%;
    margin: 0;
    background: var(--bg-primary);
  }

  /* ── Sidebar ── */
  .layout { display: flex; flex: 1; overflow: hidden; }
  .sidebar {
    width: 200px; flex-shrink: 0; padding: 12px 8px;
    border-right: 0.5px solid var(--border); display: flex; flex-direction: column; gap: 1px;
    background: var(--bg-primary); overflow-y: auto;
  }
  .sidebar-section { margin-bottom: 12px; }
  .sidebar-label {
    display: flex; align-items: center; justify-content: space-between;
    font-size: 10px; font-weight: 600; color: var(--text-tertiary);
    text-transform: uppercase; letter-spacing: 0.5px;
    padding: 6px 10px 4px; user-select: none;
  }
  .sidebar-clear {
    font-size: 9px; color: var(--text-tertiary); cursor: pointer;
    text-transform: none; letter-spacing: 0; padding: 1px 5px; border-radius: 4px;
    background: none; border: none; font-family: inherit;
    transition: color 0.12s, background 0.12s;
  }
  .sidebar-clear:hover { color: var(--accent); background: var(--bg-hover); }
  .nav-item {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 10px; border-radius: 8px; cursor: pointer;
    font-size: 12px; color: var(--text-secondary);
    transition: background 0.12s, color 0.12s;
    -webkit-app-region: no-drag; border: none; background: none;
    width: 100%; text-align: left;
  }
  .nav-item:hover { background: var(--bg-hover); color: var(--text-primary); }
  .nav-item.selected { background: var(--bg-active); color: var(--accent); font-weight: 600; }
  .nav-check {
    width: 14px; height: 14px; flex-shrink: 0; border-radius: 4px;
    border: 1.5px solid var(--border-strong);
    display: flex; align-items: center; justify-content: center;
    transition: background 0.12s, border-color 0.12s;
  }
  .nav-item.selected .nav-check { background: var(--accent); border-color: var(--accent); }
  .nav-item.selected .nav-check svg { color: #fff; }
  .nav-item:not(.selected) .nav-check svg { display: none; }
  .nav-icon {
    width: 14px; height: 14px; display: flex; align-items: center; justify-content: center;
    flex-shrink: 0; opacity: 0.7;
  }
  .nav-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nav-count {
    margin-left: auto; font-size: 10px; font-weight: 600;
    color: var(--text-tertiary); min-width: 18px; text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .nav-item.selected .nav-count { color: var(--accent); }

  /* ── Main content ── */
  .main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }

  /* ── Toolbar ── */
  .toolbar-btn {
    display: flex; align-items: center; justify-content: center;
    height: 30px; padding: 0 10px; gap: 6px; border-radius: 8px;
    font-size: 11px; font-weight: 500;
    color: var(--text-secondary); background: var(--bg-secondary);
    border: 1px solid var(--border); cursor: pointer;
    transition: background 0.12s, color 0.12s; white-space: nowrap;
    font-family: inherit; flex: 0 0 auto;
  }
  .toolbar-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .toolbar-btn:disabled { opacity: 0.55; cursor: default; }
  .toolbar-btn.danger { color: var(--red); }
  .toolbar-btn.danger:hover { background: color-mix(in srgb, var(--red) 8%, transparent); }

  @media (max-width: 700px) {
    .sidebar { width: 168px; }
    .toolbar-btn { padding: 0 8px; }
  }

  /* Active filter bar */
  .filter-bar {
    display: flex; align-items: center; gap: 6px;
    padding: 8px 16px; border-bottom: 0.5px solid var(--border);
    background: var(--bg-secondary); flex-shrink: 0; flex-wrap: wrap;
  }
  .filter-bar-label {
    font-size: 10px; font-weight: 600; color: var(--text-tertiary);
    text-transform: uppercase; letter-spacing: 0.5px; margin-right: 2px;
  }
  .filter-pills { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; flex: 1; }
  .filter-pill {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 3px 4px 3px 8px; border-radius: 12px;
    font-size: 11px; color: var(--accent);
    background: var(--bg-active); border: 1px solid color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .pill-kind { opacity: 0.7; font-size: 9px; text-transform: uppercase; letter-spacing: 0.4px; }
  .filter-pill-x {
    display: inline-flex; align-items: center; justify-content: center;
    width: 16px; height: 16px; padding: 0; border-radius: 50%;
    color: var(--accent); background: none; border: none; cursor: pointer;
    transition: background 0.12s;
  }
  .filter-pill-x svg { display: block; }
  .filter-pill-x:hover { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .filter-clear-all {
    margin-left: auto; font-size: 10px; color: var(--text-tertiary);
    background: none; border: none; cursor: pointer; font-family: inherit;
    padding: 2px 6px; border-radius: 4px;
  }
  .filter-clear-all:hover { color: var(--red); background: var(--bg-hover); }

  /* Selection actions */
  .selection-bar {
    display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
    padding: 8px 16px; border-bottom: 0.5px solid var(--border);
    background: color-mix(in srgb, var(--accent) 7%, var(--bg-primary));
    flex-shrink: 0;
  }
  .selection-summary { font-size: 11px; font-weight: 600; color: var(--accent); margin-right: auto; }
  .selection-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }

  /* ── File list ── */
  .file-list { flex: 1; overflow-y: auto; padding: 8px 12px; }
  .group { margin-bottom: 16px; }
  .group-body.file-grid { margin-top: 4px; }
  .group-header {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 8px; margin-bottom: 4px;
    font-size: 11px; font-weight: 600; color: var(--text-secondary);
    border-bottom: 1px solid var(--border);
  }
  .group-header-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .group-count { font-size: 10px; color: var(--text-tertiary); font-weight: 500; }

  .file-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(136px, 1fr)); gap: 10px; }

  .file-row {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 8px; border-radius: 8px;
    transition: background 0.1s; cursor: default;
  }
  .file-row:hover { background: var(--bg-hover); }

  .file-card {
    position: relative; display: flex; flex-direction: column; align-items: center; gap: 8px;
    min-height: 150px; padding: 12px 8px 10px;
    border: 1px solid transparent; border-radius: 8px;
    background: transparent; transition: background 0.12s, border-color 0.12s;
  }
  .file-card:hover { background: var(--bg-hover); border-color: var(--border); }
  .file-card.missing { opacity: 0.5; }
  .file-card .file-select { position: absolute; top: 7px; left: 7px; opacity: 0; transition: opacity 0.12s; }
  .file-card:hover .file-select,
  .file-card:has(input:checked) .file-select { opacity: 1; }
  .file-card-icon {
    width: 54px; height: 62px; margin-top: 4px;
    display: flex; align-items: center; justify-content: center;
    border-radius: 8px; color: #fff; font-size: 11px; font-weight: 800;
    text-transform: uppercase; letter-spacing: 0.3px;
    box-shadow: inset 0 -10px 18px rgba(0, 0, 0, 0.08);
  }
  .file-card-preview {
    width: 82px; height: 68px; margin-top: 2px;
    display: flex; align-items: center; justify-content: center;
    border-radius: 8px; overflow: hidden;
    background: var(--bg-secondary); border: 1px solid var(--border);
    color: var(--text-tertiary); font-size: 10px;
  }
  .file-card-preview img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .file-card-preview-text {
    width: 100%; height: 100%; padding: 6px;
    font-size: 8.5px; line-height: 1.25;
    color: var(--text-secondary); text-align: left;
    white-space: pre-wrap; overflow: hidden;
    background: var(--bg-primary);
  }
  .file-card-preview.pending { background: color-mix(in srgb, var(--accent) 5%, var(--bg-secondary)); }
  .file-card-preview.fallback {
    width: 54px; height: 62px;
    border: none; color: #fff; font-size: 11px; font-weight: 800;
    text-transform: uppercase; letter-spacing: 0.3px;
    box-shadow: inset 0 -10px 18px rgba(0, 0, 0, 0.08);
  }
  .file-card-name {
    width: 100%; min-height: 34px; display: -webkit-box;
    -webkit-line-clamp: 2; -webkit-box-orient: vertical; line-clamp: 2;
    overflow: hidden; text-align: center; font-size: 11px; font-weight: 600;
    color: var(--text-primary); cursor: pointer; line-height: 1.35;
    overflow-wrap: anywhere;
  }
  .file-card-name:hover { color: var(--accent); }
  .file-card-meta {
    width: 100%; display: flex; align-items: center; justify-content: center; gap: 4px;
    font-size: 9px; color: var(--text-tertiary); white-space: nowrap; overflow: hidden;
  }
  .file-card-meta span { overflow: hidden; text-overflow: ellipsis; }
  .file-card-actions {
    display: flex; align-items: center; justify-content: center; gap: 2px;
    opacity: 0; transition: opacity 0.12s;
  }
  .file-card:hover .file-card-actions { opacity: 1; }

  .file-icon {
    width: 32px; height: 32px; border-radius: 8px;
    display: flex; align-items: center; justify-content: center;
    font-size: 10px; font-weight: 700; color: #fff; flex-shrink: 0;
    text-transform: uppercase; letter-spacing: 0.3px;
  }
  .file-select {
    width: 20px; height: 20px; flex-shrink: 0;
    display: flex; align-items: center; justify-content: center;
  }
  .file-select input { width: 15px; height: 15px; accent-color: var(--accent); cursor: pointer; }
  .file-info { flex: 1; min-width: 0; }
  .file-name {
    font-size: 12px; font-weight: 500; color: var(--text-primary);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    cursor: pointer; transition: color 0.15s;
  }
  .file-name:hover { color: var(--accent); }
  .file-meta {
    display: flex; align-items: center; gap: 8px;
    font-size: 10px; color: var(--text-tertiary); margin-top: 1px;
  }
  .file-meta-tag {
    padding: 1px 5px; border-radius: 4px; font-size: 9px; font-weight: 600;
    background: var(--bg-secondary); color: var(--text-secondary); text-transform: uppercase;
  }
  .file-actions {
    display: flex; align-items: center; gap: 2px;
    opacity: 0; transition: opacity 0.15s; flex-shrink: 0;
  }
  .file-row:hover .file-actions { opacity: 1; }
  .action-btn {
    display: flex; align-items: center; justify-content: center;
    width: 28px; height: 28px; border-radius: 6px;
    color: var(--text-tertiary); background: none; border: none;
    cursor: pointer; padding: 0; transition: color 0.15s, background 0.15s;
  }
  .action-btn:hover { color: var(--text-primary); background: var(--bg-secondary); }
  .action-btn:disabled { opacity: 0.55; cursor: default; pointer-events: none; }
  .action-btn.remove:hover { color: var(--red); background: color-mix(in srgb, var(--red) 8%, transparent); }

  .action-tip { position: relative; display: inline-flex; }
  .action-tip-bubble {
    position: absolute; bottom: calc(100% + 5px); right: 0;
    white-space: nowrap; padding: 3px 7px; border-radius: 6px;
    font-size: 10px; font-weight: 600; line-height: 1.2;
    background: var(--text-primary); color: var(--bg-primary);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
    opacity: 0; pointer-events: none; transition: opacity 0.12s; z-index: 6;
  }
  .action-tip:hover .action-tip-bubble,
  .action-tip.pinned .action-tip-bubble { opacity: 1; }
  .action-tip.pinned .action-btn.remove { color: var(--red); background: color-mix(in srgb, var(--red) 10%, transparent); }
  .file-row:has(.action-tip.pinned) .file-actions,
  .file-card:has(.action-tip.pinned) .file-card-actions { opacity: 1; }

  .file-row.missing { opacity: 0.5; }
  .file-row.missing .file-name { text-decoration: line-through; cursor: default; }
  .file-row.missing .file-name:hover { color: var(--text-primary); }
  .missing-badge {
    font-size: 9px; font-weight: 600; color: var(--red);
    background: color-mix(in srgb, var(--red) 10%, transparent);
    padding: 1px 5px; border-radius: 4px; white-space: nowrap;
  }

  .empty-state {
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    height: 100%; gap: 8px; color: var(--text-tertiary);
  }
  .empty-state svg { opacity: 0.3; }
  .empty-state-text { font-size: 13px; }

  .spinner {
    width: 24px; height: 24px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%; animation: spin 0.6s linear infinite;
  }
  @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }

  /* ── Loading skeleton (shaped like grouped file rows) ── */
  .files-skeleton {
    --fsk-base: color-mix(in srgb, var(--text-primary) 7%, transparent);
    --fsk-hi: color-mix(in srgb, var(--text-primary) 14%, transparent);
    padding-top: 2px;
    animation: fsk-fade-in 0.25s ease both;
  }
  .fsk-group { margin-bottom: 16px; }
  .fsk {
    background: linear-gradient(90deg, var(--fsk-base) 25%, var(--fsk-hi) 37%, var(--fsk-base) 63%);
    background-size: 400% 100%;
    animation: fsk-shimmer 1.4s ease-in-out infinite;
  }
  .fsk-group-header { height: 11px; width: 120px; border-radius: 5px; margin: 6px 8px 10px; }
  .fsk-row { display: flex; align-items: center; gap: 10px; padding: 8px; }
  .fsk-icon { width: 32px; height: 32px; border-radius: 8px; flex-shrink: 0; }
  .fsk-row-text { display: flex; flex-direction: column; gap: 7px; flex: 1; min-width: 0; }
  .fsk-name { height: 12px; border-radius: 5px; }
  .fsk-meta { height: 9px; width: 38%; border-radius: 5px; }

  @keyframes fsk-shimmer {
    0% { background-position: 100% 0; }
    100% { background-position: 0 0; }
  }
  @keyframes fsk-fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .fsk { animation: none; background: var(--fsk-base); }
    .files-skeleton { animation: none; }
  }

  /* ── Status bar ── */
  .status-bar {
    display: flex; align-items: center; justify-content: space-between;
    padding: 6px 16px; border-top: 0.5px solid var(--border);
    font-size: 10px; color: var(--text-tertiary); flex-shrink: 0;
    background: var(--bg-primary);
  }

  /* ── Toolbar popups (period / sort) ── */
  .menu-backdrop { position: absolute; inset: 0; z-index: 140; }
  .popmenu {
    position: absolute; top: 6px; z-index: 150;
    width: 176px; padding: 5px;
    background: var(--bg-primary); border: 1px solid var(--border-strong);
    border-radius: 10px; box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
  }
  .popmenu-label {
    padding: 5px 10px 3px; font-size: 10px; font-weight: 600;
    color: var(--text-tertiary); text-transform: uppercase; letter-spacing: 0.5px;
  }
  .popmenu-item {
    display: flex; align-items: center; gap: 7px; width: 100%;
    padding: 7px 10px; border: none; background: none; border-radius: 7px;
    font-size: 12.5px; color: var(--text-primary); cursor: pointer;
    text-align: left; font-family: inherit;
  }
  .popmenu-item:hover { background: var(--bg-hover); }
  .popmenu-item.active { background: var(--bg-active); color: var(--accent); font-weight: 600; }
  .popmenu-check { width: 12px; flex-shrink: 0; opacity: 0; display: inline-flex; }
  .popmenu-item.active .popmenu-check { opacity: 1; }
  .popmenu-check svg { display: block; }
  .popmenu-divider { height: 1px; background: var(--border); margin: 4px 2px; }

  /* ── Duplicate manager ── */
  .modal-backdrop {
    position: fixed; inset: 0; z-index: 200;
    display: flex; align-items: center; justify-content: center;
    background: rgba(0, 0, 0, 0.28); padding: 24px;
  }
  .duplicate-modal {
    width: min(760px, 100%); max-height: min(680px, 90vh);
    display: flex; flex-direction: column; overflow: hidden;
    background: var(--bg-primary); border: 1px solid var(--border-strong);
    border-radius: 10px; box-shadow: 0 18px 50px rgba(0, 0, 0, 0.22);
  }
  .modal-head {
    display: flex; align-items: center; justify-content: space-between; gap: 12px;
    padding: 14px 16px; border-bottom: 0.5px solid var(--border);
  }
  .modal-head-info { min-width: 0; }
  .modal-title { font-size: 14px; font-weight: 700; color: var(--text-primary); }
  .modal-subtitle { margin-top: 2px; font-size: 11px; color: var(--text-tertiary); }
  .modal-close {
    flex-shrink: 0; padding: 0;
    width: 28px; height: 28px; display: flex; align-items: center; justify-content: center;
    border: 1px solid var(--border); border-radius: 6px; background: var(--bg-secondary);
    color: var(--text-secondary); cursor: pointer;
  }
  .modal-close svg { display: block; }
  .modal-close:hover { background: var(--bg-hover); color: var(--text-primary); }
  .modal-body { padding: 10px 12px; overflow-y: auto; flex: 1; }
  .modal-foot {
    display: flex; align-items: center; justify-content: space-between; gap: 10px;
    padding: 12px 16px; border-top: 0.5px solid var(--border);
  }
  .duplicate-group {
    border: 1px solid var(--border); border-radius: 8px;
    overflow: hidden; margin-bottom: 10px; background: var(--bg-primary);
  }
  .duplicate-group-head {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 8px 10px; background: var(--bg-secondary);
    font-size: 11px; color: var(--text-secondary); font-weight: 600;
  }
  .duplicate-item {
    display: grid; grid-template-columns: 1fr auto; gap: 8px;
    padding: 8px 10px; border-top: 0.5px solid var(--border);
  }
  .duplicate-info { min-width: 0; }
  .duplicate-name {
    min-width: 0; font-size: 12px; font-weight: 500;
    color: var(--text-primary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .duplicate-path {
    min-width: 0; margin-top: 2px; font-size: 10px;
    color: var(--text-tertiary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .duplicate-tags { display: flex; align-items: center; gap: 5px; }
  .duplicate-tag {
    padding: 1px 5px; border-radius: 4px; font-size: 9px; font-weight: 600;
    background: var(--bg-secondary); color: var(--text-secondary); white-space: nowrap;
  }
  .duplicate-tag.keep { background: var(--bg-active); color: var(--accent); }
  .duplicate-empty {
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    min-height: 180px; gap: 8px; color: var(--text-tertiary); text-align: center;
  }
  .duplicate-empty strong { color: var(--text-secondary); font-size: 13px; }
  .modal-action-row { display: flex; align-items: center; gap: 8px; }
</style>
