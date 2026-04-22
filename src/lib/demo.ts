/**
 * Demo mode — provides realistic test data for all major views.
 * Activated by tapping the logo on the login page 7 times in quick succession.
 */
import { writable, get } from "svelte/store";
import {
  authState,
  lunaAuthState,
  kwicAuthState,
  mailAuthState,
  type GradesData,
  type CancellationsData,
  type MakeupData,
  type RoomChangesData,
  type RegistrationData,
  type ExamTimetableData,
  type NotificationsData,
  type StudentInfo,
  type SyllabusSearchParams,
  type SyllabusSearchResult,
  type SyllabusEntry,
} from "./stores";
import type {
  ScheduleResponse,
  LunaTodoItem,
  LunaNotification,
  KgcCourseRow,
  LunaCourseRow,
} from "./types";
import type {
  WeatherData,
  KwicPortalHome,
  KwicSubportalData,
  KwicPortalNotification,
  MailMessage,
  MailAttachment,
} from "./api";

// ---- Store ----

export const demoMode = writable(false);

const DEMO_KEY = "selah-demo-mode";

export function isDemoMode(): boolean {
  return get(demoMode);
}

export function activateDemo() {
  demoMode.set(true);
  try { localStorage.setItem(DEMO_KEY, "1"); } catch {}

  authState.set({
    authenticated: true,
    username: "demo_user",
    displayName: "関学 太郎",
    studentId: "12345678",
    faculty: "理工学部",
    department: "情報科学科",
    loading: false,
    error: "",
  });
  lunaAuthState.set({ authenticated: true });
  kwicAuthState.set({ authenticated: true });
  mailAuthState.set({ authenticated: true, email: "taro@kwansei.ac.jp", displayName: "関学 太郎" });

  try { localStorage.setItem("selah-ever-auth", "1"); } catch {}
}

export function deactivateDemo() {
  demoMode.set(false);
  try { localStorage.removeItem(DEMO_KEY); } catch {}
}

export function restoreDemo(): boolean {
  try {
    if (localStorage.getItem(DEMO_KEY) === "1") {
      activateDemo();
      return true;
    }
  } catch {}
  return false;
}

// ---- Shared student ----

const demoStudent: StudentInfo = {
  student_id: "12345678",
  name: "関学 太郎",
  name_en: "KWANGAKU Taro",
  student_type: "学部学生",
  affiliation_type: "正規",
  status: "在学",
  class: "3年",
  faculty: "理工学部",
  department: "情報科学科",
  major: "",
  address: "兵庫県西宮市上ケ原一番町1-155",
};

// ---- Helpers ----

function todayStr(): string {
  return new Date().toISOString().slice(0, 10);
}

function futureDate(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  return d.toISOString().slice(0, 10);
}

function futureISO(days: number, hour = 9, min = 0): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  d.setHours(hour, min, 0, 0);
  return d.toISOString();
}

const TERM_LABELS: Record<string, string> = {
  "01": "通年",
  "02": "春学期",
  "03": "秋学期",
  "04": "春学期前半",
  "05": "春学期後半",
  "06": "秋学期前半",
  "07": "秋学期後半",
  "20": "通年集中",
  "21": "春学期集中",
  "22": "秋学期集中",
};

const CAMPUS_LABELS: Record<string, string> = {
  "1": "西宮上ケ原",
  "2": "神戸三田",
  "3": "大阪梅田",
  "5": "西宮聖和",
  "6": "オンライン",
  "7": "東京丸の内",
  "8": "西宮北口",
};

const DEPARTMENT_LABELS: Record<string, string> = {
  "28": "理工学部",
  "36": "理学部",
  "37": "工学部",
  "38": "生命環境学部",
  "39": "建築学部",
  "42": "共通教育センター",
  "45": "言語教育研究センター",
};

const demoSyllabusCatalog: SyllabusEntry[] = [
  {
    academic_year: "2026",
    department: "理工学部",
    class_code: "CS301",
    course_title: "アルゴリズムとデータ構造",
    instructor: "田中 一郎",
    term: "春学期",
    day_period: "月2",
    campus: "神戸三田",
    credits: "2",
    bookmarked: true,
    refer_index: "syllabus-cs301",
    register_index: "syllabus-cs301",
  },
  {
    academic_year: "2026",
    department: "理工学部",
    class_code: "CS302",
    course_title: "オペレーティングシステム",
    instructor: "佐藤 花子",
    term: "春学期",
    day_period: "火3",
    campus: "神戸三田",
    credits: "2",
    bookmarked: false,
    refer_index: "syllabus-cs302",
    register_index: "syllabus-cs302",
  },
  {
    academic_year: "2026",
    department: "理工学部",
    class_code: "CS330",
    course_title: "ヒューマンコンピュータインタラクション",
    instructor: "中村 佳奈",
    term: "春学期",
    day_period: "火5",
    campus: "神戸三田",
    credits: "2",
    bookmarked: true,
    refer_index: "syllabus-cs330",
    register_index: "syllabus-cs330",
  },
  {
    academic_year: "2026",
    department: "理工学部",
    class_code: "CS360",
    course_title: "情報理論",
    instructor: "木村 恒一",
    term: "春学期",
    day_period: "金3",
    campus: "神戸三田",
    credits: "2",
    bookmarked: false,
    refer_index: "syllabus-cs360",
    register_index: "syllabus-cs360",
  },
  {
    academic_year: "2026",
    department: "共通教育センター",
    class_code: "GE210",
    course_title: "データサイエンス実践",
    instructor: "藤井 直人",
    term: "春学期",
    day_period: "木4",
    campus: "オンライン",
    credits: "2",
    bookmarked: false,
    refer_index: "syllabus-ge210",
    register_index: "syllabus-ge210",
  },
];

const demoSyllabusBookmarks = new Set(
  demoSyllabusCatalog.filter((entry) => entry.bookmarked).map((entry) => entry.class_code),
);

function cloneSyllabusEntry(entry: SyllabusEntry): SyllabusEntry {
  return {
    ...entry,
    bookmarked: demoSyllabusBookmarks.has(entry.class_code),
  };
}

function buildSyllabusResult(entries: SyllabusEntry[]): SyllabusSearchResult {
  return {
    entries: entries.map(cloneSyllabusEntry),
    total_count: entries.length,
    current_page: 1,
    total_pages: entries.length > 0 ? 1 : 0,
  };
}

function includesFolded(value: string, needle: string): boolean {
  return value.toLowerCase().includes(needle.toLowerCase());
}

// ---- Demo Data Generators ----

export function demoScheduleData(): ScheduleResponse {
  const kgcCurrent: KgcCourseRow[] = [
    { id: 1, kgc_code: "CS301", name: "アルゴリズムとデータ構造", day: 1, period: 2, room: "III-201", detail_path: "/course/1", is_cancelled: false, is_makeup: false, is_room_changed: false, week_label: "" },
    { id: 2, kgc_code: "CS302", name: "オペレーティングシステム", day: 2, period: 3, room: "III-301", detail_path: "/course/2", is_cancelled: false, is_makeup: false, is_room_changed: false, week_label: "" },
    { id: 3, kgc_code: "CS303", name: "データベース概論", day: 3, period: 1, room: "III-102", detail_path: "/course/3", is_cancelled: false, is_makeup: false, is_room_changed: false, week_label: "" },
    { id: 4, kgc_code: "MA201", name: "線形代数学 II", day: 4, period: 2, room: "I-301", detail_path: "/course/4", is_cancelled: false, is_makeup: false, is_room_changed: false, week_label: "" },
    { id: 5, kgc_code: "EN101", name: "Academic English III", day: 5, period: 1, room: "IV-103", detail_path: "/course/5", is_cancelled: false, is_makeup: false, is_room_changed: false, week_label: "" },
    { id: 6, kgc_code: "CS304", name: "ソフトウェア工学", day: 1, period: 4, room: "III-201", detail_path: "/course/6", is_cancelled: false, is_makeup: false, is_room_changed: false, week_label: "" },
    { id: 7, kgc_code: "CS305", name: "コンピュータネットワーク", day: 3, period: 3, room: "III-401", detail_path: "/course/7", is_cancelled: true, is_makeup: false, is_room_changed: false, week_label: "" },
  ];

  const lunaCourses: LunaCourseRow[] = [
    { id: 1, luna_id: "L001", name: "アルゴリズムとデータ構造", teacher: "田中 一郎", day: 1, period: 2 },
    { id: 2, luna_id: "L002", name: "オペレーティングシステム", teacher: "佐藤 花子", day: 2, period: 3 },
    { id: 3, luna_id: "L003", name: "データベース概論", teacher: "鈴木 三郎", day: 3, period: 1 },
    { id: 4, luna_id: "L004", name: "線形代数学 II", teacher: "高橋 四郎", day: 4, period: 2 },
    { id: 5, luna_id: "L005", name: "Academic English III", teacher: "J. Smith", day: 5, period: 1 },
    { id: 6, luna_id: "L006", name: "ソフトウェア工学", teacher: "山田 五郎", day: 1, period: 4 },
    { id: 7, luna_id: "L007", name: "コンピュータネットワーク", teacher: "伊藤 六介", day: 3, period: 3 },
  ];

  return {
    raw: {
      kgc_entries_current: kgcCurrent,
      kgc_entries_next: kgcCurrent.map(c => ({ ...c, is_cancelled: false })),
      luna_courses: lunaCourses,
      session_plans: [
        ["L001", [{ session_num: 8, topic: "グラフアルゴリズム", delivery_mode: "対面", study_outside: "教科書 第8章を読むこと" }]],
        ["L002", [{ session_num: 6, topic: "プロセス管理", delivery_mode: "対面", study_outside: "復習レポート提出" }]],
      ],
      luna_counts: [
        ["L001", { announcements: 3, new_announcements: 1, reports: 2, exams: 0, discussions: 1 }],
        ["L002", { announcements: 5, new_announcements: 0, reports: 1, exams: 1, discussions: 0 }],
      ],
      luna_activities: [
        { luna_id: "L001", activity_type: "report", title: "第7回 レポート課題", period: futureDate(3), status: "未提出" },
        { luna_id: "L002", activity_type: "exam", title: "中間テスト", period: futureDate(7), status: "未受験" },
      ],
      kgc_course_details: [
        { kgc_code: "CS301", fields: [["担当教員", "田中 一郎"], ["単位", "2"]], delivery_mode: "対面" },
        { kgc_code: "CS302", fields: [["担当教員", "佐藤 花子"], ["単位", "2"]], delivery_mode: "対面" },
      ],
      current_week_label: "第8週",
      next_week_label: "第9週",
      luna_communities: [],
    },
    ai_result: null,
    ai_stale: false,
    snapshot_updated_at: Date.now(),
    luna_communities: [],
    luna_year_options: [{ value: "2026", label: "2026", selected: true }],
    luna_term_options: [{ value: "spring", label: "春学期", selected: true }],
    luna_year: "2026",
    luna_term: "spring",
  };
}

export function demoGrades(): GradesData {
  return {
    student: demoStudent,
    curriculum: [
      { category: "専門必修", level: 3, required_credits: "40", enrolled_acquired_credits: "30", enrolled_credits: "36", earned_credits: "30", is_deficit: false },
      { category: "専門選択", level: 3, required_credits: "20", enrolled_acquired_credits: "16", enrolled_credits: "22", earned_credits: "16", is_deficit: false },
      { category: "共通教育", level: 3, required_credits: "24", enrolled_acquired_credits: "24", enrolled_credits: "24", earned_credits: "24", is_deficit: false },
      { category: "外国語", level: 3, required_credits: "8", enrolled_acquired_credits: "6", enrolled_credits: "8", earned_credits: "6", is_deficit: false },
      { category: "自由選択", level: 3, required_credits: "10", enrolled_acquired_credits: "8", enrolled_credits: "10", earned_credits: "8", is_deficit: false },
    ],
  };
}

export function demoCancellations(): CancellationsData {
  return {
    student: demoStudent,
    entries: [
      { date: futureDate(2), period: "3限", campus: "三田", department: "理工学部", course_code: "CS305", year: "2026", course_name: "コンピュータネットワーク", instructor: "伊藤 六介", room: "III-401", comment: "学会出張のため" },
    ],
  };
}

export function demoMakeup(): MakeupData {
  return {
    student: demoStudent,
    entries: [
      { date: futureDate(5), period: "5限", campus: "三田", department: "理工学部", course_code: "CS305", year: "2026", course_name: "コンピュータネットワーク", instructor: "伊藤 六介", room: "III-401", comment: "休講分の補講" },
    ],
  };
}

export function demoRoomChanges(): RoomChangesData {
  return {
    student: demoStudent,
    entries: [
      { date: futureDate(1), department: "理工学部", course_code: "CS301", year: "2026", course_name: "アルゴリズムとデータ構造", room: "III-301 → III-102", instructor: "田中 一郎", schedule: "月曜 2限", comment: "教室変更" },
    ],
  };
}

export function demoRegistration(): RegistrationData {
  return {
    student: demoStudent,
    credit_summary: [
      { semester: "2026年度 春学期", enrolled: "22", limit: "26" },
    ],
    courses: [
      { period: "2", day: "月", semester: "春", course_name: "アルゴリズムとデータ構造", course_code: "CS301", instructor: "田中 一郎", campus: "三田", credits: "2", room: "III-201", status: "登録済" },
      { period: "3", day: "火", semester: "春", course_name: "オペレーティングシステム", course_code: "CS302", instructor: "佐藤 花子", campus: "三田", credits: "2", room: "III-301", status: "登録済" },
      { period: "1", day: "水", semester: "春", course_name: "データベース概論", course_code: "CS303", instructor: "鈴木 三郎", campus: "三田", credits: "2", room: "III-102", status: "登録済" },
      { period: "2", day: "木", semester: "春", course_name: "線形代数学 II", course_code: "MA201", instructor: "高橋 四郎", campus: "三田", credits: "2", room: "I-301", status: "登録済" },
      { period: "1", day: "金", semester: "春", course_name: "Academic English III", course_code: "EN101", instructor: "J. Smith", campus: "三田", credits: "2", room: "IV-103", status: "登録済" },
      { period: "4", day: "月", semester: "春", course_name: "ソフトウェア工学", course_code: "CS304", instructor: "山田 五郎", campus: "三田", credits: "2", room: "III-201", status: "登録済" },
      { period: "3", day: "水", semester: "春", course_name: "コンピュータネットワーク", course_code: "CS305", instructor: "伊藤 六介", campus: "三田", credits: "2", room: "III-401", status: "登録済" },
    ],
    year_semester: "2026年度 春学期",
    last_applied: todayStr(),
    language_options: [{ name: "日本語", value: "ja" }],
  };
}

export function demoExams(): ExamTimetableData {
  return {
    student: demoStudent,
    entries: [
      { day: futureDate(14), period: 2, course_name: "アルゴリズムとデータ構造", room: "III-201" },
      { day: futureDate(15), period: 3, course_name: "オペレーティングシステム", room: "III-301" },
      { day: futureDate(16), period: 1, course_name: "データベース概論", room: "III-102" },
    ],
  };
}

export function demoNotifications(): NotificationsData {
  return {
    entries: [
      { id: "春学期の履修登録確認について|" + todayStr(), title: "春学期の履修登録確認について", date: todayStr(), category: "教務" },
      { id: "第8回 アルゴリズム演習の提出について|" + futureDate(-1), title: "第8回 アルゴリズム演習の提出について", date: futureDate(-1), category: "授業" },
      { id: "学生証再発行の手続き変更|" + futureDate(-2), title: "学生証再発行の手続き変更", date: futureDate(-2), category: "学生生活" },
      { id: "奨学金の申請締切について|" + futureDate(-3), title: "奨学金の申請締切について", date: futureDate(-3), category: "奨学金" },
      { id: "夏季休業中の事務室開室日程|" + futureDate(-5), title: "夏季休業中の事務室開室日程", date: futureDate(-5), category: "教務" },
    ],
  };
}

export function demoLunaTodo(): LunaTodoItem[] {
  return [
    { course_name: "アルゴリズムとデータ構造", content_type: "レポート", content_name: "第7回 レポート課題", url: "/mod/assign/view.php?id=demo-report-1", deadline: futureISO(3, 23, 59), status: "未提出", feedback: "" },
    { course_name: "オペレーティングシステム", content_type: "小テスト", content_name: "プロセス管理 確認テスト", url: "/mod/quiz/view.php?id=demo-quiz-1", deadline: futureISO(5, 23, 59), status: "未提出", feedback: "" },
    { course_name: "データベース概論", content_type: "レポート", content_name: "SQL演習課題 第3回", url: "/mod/assign/view.php?id=demo-report-2", deadline: futureISO(7, 23, 59), status: "未提出", feedback: "" },
    { course_name: "Academic English III", content_type: "課題", content_name: "Essay Draft Submission", url: "/mod/assign/view.php?id=demo-essay-1", deadline: futureISO(2, 17, 0), status: "未提出", feedback: "" },
    { course_name: "ソフトウェア工学", content_type: "レポート", content_name: "UML設計課題", url: "/mod/assign/view.php?id=demo-report-3", deadline: futureISO(-1, 23, 59), status: "提出済", feedback: "合格" },
  ];
}

export function demoLunaUpdates(): LunaNotification[] {
  return [
    { date: todayStr(), course_info: "アルゴリズムとデータ構造", module: "お知らせ", content: "第8回の授業資料をアップロードしました", url: "/mod/forum/discuss.php?d=demo-forum-1", idnumber: "L001" },
    { date: futureDate(-1), course_info: "オペレーティングシステム", module: "レポート", content: "中間テストの範囲を公開しました", url: "/mod/quiz/view.php?id=demo-quiz-1", idnumber: "L002" },
    { date: futureDate(-2), course_info: "データベース概論", module: "お知らせ", content: "来週の授業は演習室で行います", url: "/mod/forum/discuss.php?d=demo-forum-2", idnumber: "L003" },
    { date: futureDate(-3), course_info: "Academic English III", module: "課題", content: "Essay topic has been updated", url: "/mod/assign/view.php?id=demo-essay-1", idnumber: "L005" },
  ];
}

export function demoKwicHome(): KwicPortalHome {
  return {
    sections: [
      {
        title: "メインリンク",
        items: [
          { id: "kwic-main-1", title: "学生支援ポータル", date: "", category: "", url: "/portal/subportal?tagCd=1", important: false, information_type: "", person_category_cd: "", category_cd: "" },
          { id: "kwic-main-2", title: "教学サポート", date: "", category: "", url: "/portal/subportal?tagCd=2", important: false, information_type: "", person_category_cd: "", category_cd: "" },
          { id: "kwic-main-3", title: "キャリア・就職", date: "", category: "", url: "/portal/subportal?tagCd=3", important: false, information_type: "", person_category_cd: "", category_cd: "" },
          { id: "kwic-main-4", title: "ICTサポート", date: "", category: "", url: "/portal/subportal?tagCd=4", important: false, information_type: "", person_category_cd: "", category_cd: "" },
        ],
      },
      {
        title: "注目コンテンツ",
        items: [
          { id: "kwic-feature-1", title: "履修登録の事前確認ガイド", date: todayStr(), category: "教務", url: "https://kwansei.example/guide", important: false, information_type: "", person_category_cd: "", category_cd: "" },
          { id: "kwic-feature-2", title: "春学期サポート窓口一覧", date: futureDate(-1), category: "学生生活", url: "https://kwansei.example/support", important: false, information_type: "", person_category_cd: "", category_cd: "" },
        ],
      },
      {
        title: "呼出し・重要なお知らせ",
        items: [
          { id: "k1", title: "2026年度 春学期の時間割変更について", date: todayStr(), category: "教務", url: "", important: true, information_type: "1", person_category_cd: "1", category_cd: "1" },
          { id: "k2", title: "学生定期健康診断の事前回答", date: futureDate(-2), category: "保健館", url: "", important: true, information_type: "1", person_category_cd: "1", category_cd: "2" },
        ],
      },
      {
        title: "学部・研究科からのお知らせ",
        items: [
          { id: "k3", title: "理工学部オリエンテーション補足資料", date: futureDate(-1), category: "理工学部", url: "", important: false, information_type: "1", person_category_cd: "1", category_cd: "3" },
          { id: "k4", title: "実験レポート提出ルールの更新", date: futureDate(-4), category: "理工学部", url: "", important: false, information_type: "1", person_category_cd: "1", category_cd: "4" },
        ],
      },
      {
        title: "その他",
        items: [
          { id: "k5", title: "キャリアセンター 就職ガイダンス開催", date: futureDate(-1), category: "キャリア", url: "", important: false, information_type: "1", person_category_cd: "1", category_cd: "5" },
          { id: "k6", title: "図書館の開館時間延長のお知らせ", date: futureDate(-4), category: "施設", url: "", important: false, information_type: "1", person_category_cd: "1", category_cd: "6" },
        ],
      },
    ],
  };
}

export function demoKwicSubportal(tagCd: string): KwicSubportalData {
  const data: Record<string, KwicSubportalData> = {
    "1": {
      title: "学生支援ポータル",
      links: [
        { title: "奨学金案内", url: "https://example.com/scholarship", icon_url: "", description: "申請スケジュールと募集要項" },
        { title: "学生相談室", url: "https://example.com/counseling", icon_url: "", description: "相談窓口と利用方法" },
      ],
      notifications: [
        { id: "sub-1", title: "奨学金説明会の録画公開", date: futureDate(-1), category: "奨学金", important: false, information_type: "1", person_category_cd: "1", category_cd: "10" },
      ],
    },
    "2": {
      title: "教学サポート",
      links: [
        { title: "履修登録 FAQ", url: "https://example.com/faq", icon_url: "", description: "履修・時間割まわりのよくある質問" },
        { title: "シラバス検索", url: "https://example.com/syllabus", icon_url: "", description: "授業情報の検索" },
      ],
      notifications: [
        { id: "sub-2", title: "履修登録修正期間の案内", date: todayStr(), category: "教務", important: true, information_type: "1", person_category_cd: "1", category_cd: "11" },
      ],
    },
    "3": {
      title: "キャリア・就職",
      links: [
        { title: "インターン情報", url: "https://example.com/intern", icon_url: "", description: "募集一覧と締切" },
      ],
      notifications: [],
    },
    "4": {
      title: "ICTサポート",
      links: [
        { title: "アカウント設定", url: "https://example.com/account", icon_url: "", description: "大学アカウントの設定" },
      ],
      notifications: [],
    },
  };
  return data[tagCd] ?? { title: "サブポータル", links: [], notifications: [] };
}

export function demoKwicDetail(n: Pick<KwicPortalNotification, "id" | "title">) {
  const bodies: Record<string, string> = {
    k1: "<p>来週から一部科目の教室が変更されます。時間割の最新表示を確認してください。</p>",
    k2: "<p>健康診断の事前問診は 4 月 25 日までに回答してください。未回答者は当日の受付に時間がかかります。</p>",
    k3: "<p>理工学部向けの補足資料を掲載しました。実験科目の初回ガイダンスに関する説明を含みます。</p>",
    k4: "<p>レポート提出時のファイル命名規則と締切時刻の扱いを更新しました。</p>",
    k5: "<p>就職ガイダンスは今週金曜 16:30 から実施します。参加方法は事前登録制です。</p>",
    k6: "<p>試験期間中は図書館の開館時間を 21:00 まで延長します。</p>",
  };
  return {
    title: n.title,
    date: futureDate(-1),
    sender: "KWIC ポータル",
    body_html: bodies[n.id] ?? "<p>演示用の詳細本文です。</p>",
    attachments: [],
  };
}

export function demoWeather(): WeatherData {
  return {
    temperature: 21.5,
    weatherCode: 1,
    humidity: 55,
    windSpeed: 8.2,
    tomorrow: { tempMax: 24.0, tempMin: 15.0, weatherCode: 2 },
  };
}

export function demoMailInbox(): MailMessage[] {
  return [
    { id: "m1", subject: "【重要】春学期成績評価方法の変更", bodyPreview: "学生の皆さんへ。春学期の成績評価方法について、以下の通り変更がありますのでお知らせします...", from: { emailAddress: { name: "教務課", address: "kyomu@kwansei.ac.jp" } }, receivedDateTime: futureISO(0, 10, 30), isRead: false, hasAttachments: false },
    { id: "m2", subject: "アルゴリズム 第7回レポートについて", bodyPreview: "田中です。第7回のレポート課題について、提出期限を3日延長します。詳細は授業で...", from: { emailAddress: { name: "田中 一郎", address: "tanaka@kwansei.ac.jp" } }, receivedDateTime: futureISO(-1, 14, 15), isRead: false, hasAttachments: true },
    { id: "m3", subject: "Re: 研究室訪問の件", bodyPreview: "関学さん、来週の火曜日15時でしたらお時間取れます。研究室は III-501 です...", from: { emailAddress: { name: "山本 教授", address: "yamamoto@kwansei.ac.jp" } }, receivedDateTime: futureISO(-2, 9, 45), isRead: true, hasAttachments: false },
    { id: "m4", subject: "図書館システムメンテナンスのお知らせ", bodyPreview: "4月20日（日）9:00-17:00の間、図書館システムのメンテナンスを実施します...", from: { emailAddress: { name: "図書館", address: "library@kwansei.ac.jp" } }, receivedDateTime: futureISO(-3, 11, 0), isRead: true, hasAttachments: false },
    { id: "m5", subject: "サークル新歓イベントの案内", bodyPreview: "プログラミングサークルの新歓イベントを開催します。日時: 4月25日 18:00...", from: { emailAddress: { name: "プログラミングサークル", address: "progcircle@kwansei.ac.jp" } }, receivedDateTime: futureISO(-5, 16, 20), isRead: true, hasAttachments: false },
  ];
}

export function demoMailAttachments(messageId: string): MailAttachment[] {
  if (messageId !== "m2") return [];
  return [
    { id: "mail-attachment-1", name: "report-guideline.pdf", contentType: "application/pdf", size: 235520 },
  ];
}

export function demoStudentProfile(): StudentInfo {
  return { ...demoStudent };
}

export function demoLunaDetail(path: string) {
  const courseName =
    path.includes("demo-quiz-1") ? "オペレーティングシステム" :
    path.includes("demo-report-2") ? "データベース概論" :
    path.includes("demo-essay-1") ? "Academic English III" :
    "アルゴリズムとデータ構造";

  return {
    title:
      path.includes("demo-quiz-1") ? "プロセス管理 確認テスト" :
      path.includes("demo-report-2") ? "SQL演習課題 第3回" :
      path.includes("demo-essay-1") ? "Essay Draft Submission" :
      path.includes("demo-forum-2") ? "来週の授業は演習室で行います" :
      "第7回 レポート課題",
    course_name: courseName,
    sections: [
      {
        heading: "概要",
        body: "演示用の Luna 詳細です。授業の要点、提出条件、注意事項をここで確認できます。",
      },
      {
        heading: "ポイント",
        body:
          path.includes("demo-quiz-1")
            ? "プロセス管理、スケジューリング、排他制御の理解を確認します。"
            : "締切前に形式と提出先を見直しておくと安心です。",
      },
    ],
    attachments: [],
    meta: {
      course: courseName,
      updated_at: todayStr(),
    },
  };
}

export function demoLunaPage(path: string): string {
  return `<!doctype html>
<html lang="ja">
  <head>
    <meta charset="utf-8" />
    <title>Luna Demo - ${path}</title>
  </head>
  <body>
    <h1>Luna Demo</h1>
    <p>パス: ${path}</p>
    <a href="/course/view.php?id=demo-course-1">Course</a>
    <a href="/mod/assign/view.php?id=demo-report-1">Assignment</a>
  </body>
</html>`;
}

export function demoSearchSyllabus(params: SyllabusSearchParams): SyllabusSearchResult {
  const termLabel = TERM_LABELS[params.term] ?? "";
  const campusLabel = CAMPUS_LABELS[params.campus] ?? "";
  const departmentLabel = DEPARTMENT_LABELS[params.department] ?? "";
  const yearFrom = parseInt(params.year_from || "0", 10);
  const yearTo = parseInt(params.year_to || "9999", 10);

  const filtered = demoSyllabusCatalog.filter((entry) => {
    const year = parseInt(entry.academic_year || "0", 10);
    if (Number.isFinite(year) && (year < yearFrom || year > yearTo)) return false;
    if (termLabel && !entry.term.includes(termLabel)) return false;
    if (campusLabel && !entry.campus.includes(campusLabel)) return false;
    if (departmentLabel && !entry.department.includes(departmentLabel)) return false;
    if (params.day_period && !entry.day_period.includes(params.day_period.replace(/^([A-F])/, (_m, g1) => ["A", "B", "C", "D", "E", "F"].includes(g1) ? ["月", "火", "水", "木", "金", "土"][["A", "B", "C", "D", "E", "F"].indexOf(g1)] : g1))) return false;
    if (params.class_code && !includesFolded(entry.class_code, params.class_code)) return false;
    if (params.keyword && !includesFolded(`${entry.course_title} ${entry.department}`, params.keyword)) return false;
    if (params.instructor && !includesFolded(entry.instructor, params.instructor)) return false;
    return true;
  });

  return buildSyllabusResult(filtered);
}

export function demoSyllabusFavorites(): SyllabusSearchResult {
  return buildSyllabusResult(
    demoSyllabusCatalog.filter((entry) => demoSyllabusBookmarks.has(entry.class_code)),
  );
}

export function demoToggleSyllabusBookmark(classCode: string): boolean {
  if (demoSyllabusBookmarks.has(classCode)) demoSyllabusBookmarks.delete(classCode);
  else demoSyllabusBookmarks.add(classCode);
  return demoSyllabusBookmarks.has(classCode);
}

export function demoFetchPage(path: string): string {
  return `<!doctype html>
<html lang="ja">
  <head>
    <meta charset="utf-8" />
    <title>Selah Demo Page</title>
  </head>
  <body>
    <h1>Selah Demo</h1>
    <p>このページは演示モード用の簡易 HTML です。</p>
    <p>path: ${path}</p>
    <table class="output">
      <tr><th>タイトル</th><th>掲示日</th><th>分類</th></tr>
      <tr><td>春学期の履修登録確認について</td><td>${todayStr()}</td><td>教務</td></tr>
    </table>
  </body>
</html>`;
}

// ---- Demo AI data generators ----

export function demoAiTodoAnalysis(): import("./types").AiTodoAnalysis {
  const deadlines = [futureDate(2), futureDate(3), futureDate(5), futureDate(7)];
  return {
    task_guides: [
      {
        task_name: "Essay Draft Submission",
        course_name: "Academic English III",
        deadline: deadlines[0],
        urgency: "critical" as const,
        background: "Academic essay writing requires a clear thesis statement, evidence-based arguments, and proper citation format (APA/MLA). Review the course rubric before submission.",
        live_note_summary: "In class, the instructor emphasized thesis clarity, paragraph unity, and linking evidence back to the claim.",
        study_hints: [
          "Outline your argument structure: introduction, 3 body paragraphs, conclusion",
          "Use academic vocabulary and formal tone throughout",
          "Proofread for grammar, spelling, and citation format",
          "Submit a draft and revise based on peer feedback",
        ],
        ready_to_use_label: "Starter Outline",
        ready_to_use: "Introduction: define the topic and state your thesis.\nBody 1: first supporting argument with one source.\nBody 2: counterpoint or second source.\nConclusion: restate the thesis and implication.",
        estimated_minutes: 120,
      },
      {
        task_name: "第7回 レポート課題",
        course_name: "アルゴリズムとデータ構造",
        deadline: deadlines[1],
        urgency: "soon" as const,
        background: "グラフ理論の基礎とその応用。BFS/DFS の動作原理と計算量を理解し、最短経路問題（ダイクストラ法）の擬似コードを書けることが求められます。",
        live_note_summary: "授業では BFS/DFS の探索順序と、ダイクストラ法で更新表をどう追うかを例題付きで確認しました。",
        study_hints: [
          "教科書 第8章のグラフアルゴリズムの節を復習する",
          "BFS と DFS の違いを図示して整理する",
          "ダイクストラ法を小さなグラフで手計算してみる",
          "擬似コードを書いて計算量 O((V+E)logV) を確認する",
        ],
        ready_to_use_label: "答案骨架",
        ready_to_use: "1. グラフ G=(V,E) を定義する。\n2. BFS/DFS の用途と違いを 1 文ずつ書く。\n3. ダイクストラ法の更新手順を番号付きで示す。\n4. 最後に計算量の根拠を書く。",
        estimated_minutes: 90,
      },
      {
        task_name: "プロセス管理 確認テスト",
        course_name: "オペレーティングシステム",
        deadline: deadlines[2],
        urgency: "soon" as const,
        background: "プロセスの状態遷移、スケジューリングアルゴリズム（FCFS, SJF, Round Robin, Priority）、デッドロックの検出と回避について出題されます。",
        live_note_summary: "",
        study_hints: [
          "プロセスの 5 状態遷移図を描いて各遷移条件を確認",
          "各スケジューリングアルゴリズムのガントチャートを練習",
          "デッドロックの 4 条件（相互排他・保持待ち・横取り不可・循環待ち）を暗記",
        ],
        ready_to_use_label: "复习清单",
        ready_to_use: "状態遷移図を自分で再現する。\nFCFS / SJF / RR の違いを 1 行ずつ説明する。\nデッドロックの 4 条件を空で言えるか確認する。",
        estimated_minutes: 60,
      },
      {
        task_name: "SQL演習課題 第3回",
        course_name: "データベース概論",
        deadline: deadlines[3],
        urgency: "normal" as const,
        background: "JOIN 操作、サブクエリ、集約関数を組み合わせた複合クエリの演習です。正規化理論（第 1〜第 3 正規形）の理解も問われます。",
        live_note_summary: "",
        study_hints: [
          "INNER JOIN / LEFT JOIN / CROSS JOIN の違いをサンプルデータで確認",
          "GROUP BY + HAVING の使い分けを練習",
          "サブクエリを JOIN に書き換える練習をする",
          "正規化の手順を ER 図とともに整理する",
        ],
        ready_to_use_label: "解题顺序",
        ready_to_use: "1. 必要な表と結合条件を書く。\n2. 集約が必要なら GROUP BY を先に決める。\n3. HAVING が必要か確認する。\n4. 最後にサブクエリでしか書けない条件があるか見る。",
        estimated_minutes: 75,
      },
    ],
    daily_plan: [
      {
        label: "今日",
        tasks: [
          "Essay Draft Submission（Academic English III）",
          "第7回 レポート課題の下調べ（アルゴリズムとデータ構造）",
        ],
        free_hours: 3,
      },
      {
        label: "明日",
        tasks: [
          "第7回 レポート課題を仕上げる（アルゴリズムとデータ構造）",
          "プロセス管理 確認テストの復習開始（オペレーティングシステム）",
        ],
        free_hours: 4,
      },
      {
        label: futureDate(2).slice(5).replace("-", "/"),
        tasks: [
          "プロセス管理のスケジューリング問題を解く（オペレーティングシステム）",
          "SQL演習課題 第3回に着手（データベース概論）",
        ],
        free_hours: 2,
      },
    ],
    advice:
      "今週は英語エッセイの締切が最も近いので、まず Academic English III のドラフトを優先しましょう。" +
      "アルゴリズムのレポートはグラフ理論の復習が鍵です。教科書第8章を読んでから取り組むと効率的です。" +
      "OS の確認テストまでにはまだ余裕があるので、毎日30分ずつ復習を進めるのがおすすめです。" +
      "SQL課題は締切が一番遠いですが、JOIN の練習は早めに始めておくと安心です。",
  };
}

export function demoAiScheduleResult(): import("./types").AiScheduleResult {
  return {
    current_week_label: "第8週",
    next_week_label: "第9週",
    current_week: [
      { day: 1, period: 2, course_name: "アルゴリズムとデータ構造", delivery_mode: "対面", room: "III-201", teacher: "田中 一郎", session_topic: "グラフアルゴリズム: BFS/DFS", is_cancelled: false, notifications: ["第8回の授業資料をアップロードしました"], assignments: ["第7回 レポート課題 (締切: " + futureDate(3) + ")"], exams: [] },
      { day: 1, period: 4, course_name: "ソフトウェア工学", delivery_mode: "対面", room: "III-201", teacher: "山田 五郎", session_topic: "テスト駆動開発とCI/CD", is_cancelled: false, notifications: [], assignments: [], exams: [] },
      { day: 2, period: 3, course_name: "オペレーティングシステム", delivery_mode: "対面", room: "III-301", teacher: "佐藤 花子", session_topic: "プロセス間通信とデッドロック", is_cancelled: false, notifications: ["中間テストの範囲を公開しました"], assignments: ["プロセス管理 確認テスト (締切: " + futureDate(5) + ")"], exams: [] },
      { day: 3, period: 1, course_name: "データベース概論", delivery_mode: "対面", room: "III-102", teacher: "鈴木 三郎", session_topic: "トランザクション処理", is_cancelled: false, notifications: ["来週の授業は演習室で行います"], assignments: ["SQL演習課題 第3回 (締切: " + futureDate(7) + ")"], exams: [] },
      { day: 3, period: 3, course_name: "コンピュータネットワーク", delivery_mode: "対面", room: "III-401", teacher: "伊藤 六介", session_topic: "（休講）", is_cancelled: true, notifications: [], assignments: [], exams: [] },
      { day: 4, period: 2, course_name: "線形代数学 II", delivery_mode: "対面", room: "I-301", teacher: "高橋 四郎", session_topic: "固有値と固有ベクトル", is_cancelled: false, notifications: [], assignments: [], exams: [] },
      { day: 5, period: 1, course_name: "Academic English III", delivery_mode: "対面", room: "IV-103", teacher: "J. Smith", session_topic: "Essay Workshop & Peer Review", is_cancelled: false, notifications: ["Essay topic has been updated"], assignments: ["Essay Draft Submission (締切: " + futureDate(2) + ")"], exams: [] },
    ],
    next_week: [
      { day: 1, period: 2, course_name: "アルゴリズムとデータ構造", delivery_mode: "対面", room: "III-201", teacher: "田中 一郎", session_topic: "最短経路アルゴリズム", is_cancelled: false, notifications: [], assignments: [], exams: [] },
      { day: 1, period: 4, course_name: "ソフトウェア工学", delivery_mode: "対面", room: "III-201", teacher: "山田 五郎", session_topic: "アジャイル開発手法", is_cancelled: false, notifications: [], assignments: [], exams: [] },
      { day: 2, period: 3, course_name: "オペレーティングシステム", delivery_mode: "対面", room: "III-301", teacher: "佐藤 花子", session_topic: "メモリ管理", is_cancelled: false, notifications: [], assignments: [], exams: [{ title: "中間テスト", date: futureDate(14) }].map(() => "中間テスト (" + futureDate(14) + ")") },
      { day: 3, period: 1, course_name: "データベース概論", delivery_mode: "対面", room: "III-102", teacher: "鈴木 三郎", session_topic: "正規化理論", is_cancelled: false, notifications: [], assignments: [], exams: [] },
      { day: 3, period: 3, course_name: "コンピュータネットワーク", delivery_mode: "対面", room: "III-401", teacher: "伊藤 六介", session_topic: "TCP/IP プロトコル（補講）", is_cancelled: false, notifications: [], assignments: [], exams: [] },
    ],
    weekly_summary:
      "今週はアルゴリズムのレポート提出と英語エッセイのドラフト提出が重なっています。水曜のネットワークは休講ですが、来週に補講があります。OS の中間テスト範囲が発表されたので、早めに復習を始めましょう。",
    cross_week_insights:
      "来週の OS 中間テストに向けて、今週中にプロセス管理の確認テストを済ませておくと良いでしょう。データベースの SQL 課題は来週の正規化理論の内容とも関連するので、並行して進めると理解が深まります。",
  };
}

export function demoAiNotifResult(): { summary: string; important: { title: string; reason: string; index: number }[]; suggestions: string[] } {
  return {
    summary:
      "今週は教務関連の通知が多く、特に春学期の履修登録確認が重要です。田中先生からレポート課題の期限延長のメールが届いています。来週の授業で教室変更があるので注意してください。",
    important: [
      { title: "春学期の履修登録確認について", reason: "履修登録の最終確認期限が近づいています", index: 1 },
      { title: "アルゴリズム 第7回レポートについて", reason: "提出期限が3日延長されました", index: 2 },
      { title: "2026年度 春学期の時間割変更について", reason: "来週から教室変更があります", index: 3 },
    ],
    suggestions: [
      "履修登録に不備がないか今日中に確認しましょう",
      "レポートの期限延長を活かしてグラフ理論の復習に時間を使いましょう",
      "来週の教室変更に備えて時間割メモを更新しておきましょう",
    ],
  };
}

// ---- Populate cache with all demo data ----

export function populateDemoCache() {
  // Directly write into the cache system via memory + localStorage
  const now = Date.now();
  const DISK_PREFIX = "selah_cache_";
  const DISK_CACHE_VERSION = 1;

  type Entry = { v: number; data: any; ts: number };

  function writeCache(key: string, data: any) {
    const entry: Entry = { v: DISK_CACHE_VERSION, data, ts: now };
    try { localStorage.setItem(DISK_PREFIX + key, JSON.stringify(entry)); } catch {}
  }

  writeCache("schedule_data", demoScheduleData());
  writeCache("grades", demoGrades());
  writeCache("cancellations", demoCancellations());
  writeCache("makeup", demoMakeup());
  writeCache("rooms", demoRoomChanges());
  writeCache("registration", demoRegistration());
  writeCache("exams", demoExams());
  writeCache("notifications", demoNotifications());
  writeCache("luna_todo", demoLunaTodo());
  writeCache("luna_updates", demoLunaUpdates());
  writeCache("kwic_home", demoKwicHome());
  writeCache("weather", demoWeather());
  writeCache("mail_inbox", demoMailInbox());
  writeCache("student_profile", demoStudentProfile());
  writeCache("favorites", demoSyllabusFavorites());
  writeCache("syllabus_favorites", demoSyllabusFavorites());
  writeCache("mail_profile", { displayName: demoStudent.name, mail: "taro@kwansei.ac.jp", userPrincipalName: "taro@kwansei.ac.jp" });
}
