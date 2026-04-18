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
} from "./stores";
import type {
  ScheduleResponse,
  LunaTodoItem,
  LunaNotification,
  KgcCourseRow,
  LunaCourseRow,
} from "./types";
import type { WeatherData, KwicPortalHome, MailMessage } from "./api";

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
      { id: "n1", title: "春学期の履修登録確認について", date: todayStr(), category: "教務" },
      { id: "n2", title: "学生証再発行の手続き変更", date: futureDate(-1), category: "学生生活" },
      { id: "n3", title: "図書館の開館時間延長のお知らせ", date: futureDate(-2), category: "施設" },
      { id: "n4", title: "奨学金の申請締切について", date: futureDate(-3), category: "奨学金" },
      { id: "n5", title: "夏季休業中の事務室開室日程", date: futureDate(-5), category: "教務" },
    ],
  };
}

export function demoLunaTodo(): LunaTodoItem[] {
  return [
    { course_name: "アルゴリズムとデータ構造", content_type: "レポート", content_name: "第7回 レポート課題", url: "#", deadline: futureISO(3, 23, 59), status: "未提出", feedback: "" },
    { course_name: "オペレーティングシステム", content_type: "小テスト", content_name: "プロセス管理 確認テスト", url: "#", deadline: futureISO(5, 23, 59), status: "未提出", feedback: "" },
    { course_name: "データベース概論", content_type: "レポート", content_name: "SQL演習課題 第3回", url: "#", deadline: futureISO(7, 23, 59), status: "未提出", feedback: "" },
    { course_name: "Academic English III", content_type: "課題", content_name: "Essay Draft Submission", url: "#", deadline: futureISO(2, 17, 0), status: "未提出", feedback: "" },
    { course_name: "ソフトウェア工学", content_type: "レポート", content_name: "UML設計課題", url: "#", deadline: futureISO(-1, 23, 59), status: "提出済", feedback: "合格" },
  ];
}

export function demoLunaUpdates(): LunaNotification[] {
  return [
    { date: todayStr(), course_info: "アルゴリズムとデータ構造", module: "お知らせ", content: "第8回の授業資料をアップロードしました", url: "#", idnumber: "L001" },
    { date: futureDate(-1), course_info: "オペレーティングシステム", module: "レポート", content: "中間テストの範囲を公開しました", url: "#", idnumber: "L002" },
    { date: futureDate(-2), course_info: "データベース概論", module: "お知らせ", content: "来週の授業は演習室で行います", url: "#", idnumber: "L003" },
    { date: futureDate(-3), course_info: "Academic English III", module: "課題", content: "Essay topic has been updated", url: "#", idnumber: "L005" },
  ];
}

export function demoKwicHome(): KwicPortalHome {
  return {
    sections: [
      {
        title: "重要なお知らせ",
        items: [
          { id: "k1", title: "2026年度 春学期の時間割変更について", date: todayStr(), category: "教務", url: "#", important: true, information_type: "1", person_category_cd: "1", category_cd: "1" },
          { id: "k2", title: "新型コロナウイルス感染症対策の変更", date: futureDate(-2), category: "健康", url: "#", important: true, information_type: "1", person_category_cd: "1", category_cd: "2" },
        ],
      },
      {
        title: "一般",
        items: [
          { id: "k3", title: "キャリアセンター 就職ガイダンス開催", date: futureDate(-1), category: "キャリア", url: "#", important: false, information_type: "1", person_category_cd: "1", category_cd: "3" },
          { id: "k4", title: "学食メニューリニューアルのお知らせ", date: futureDate(-4), category: "施設", url: "#", important: false, information_type: "1", person_category_cd: "1", category_cd: "4" },
        ],
      },
    ],
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
        study_hints: [
          "Outline your argument structure: introduction, 3 body paragraphs, conclusion",
          "Use academic vocabulary and formal tone throughout",
          "Proofread for grammar, spelling, and citation format",
          "Submit a draft and revise based on peer feedback",
        ],
        estimated_minutes: 120,
      },
      {
        task_name: "第7回 レポート課題",
        course_name: "アルゴリズムとデータ構造",
        deadline: deadlines[1],
        urgency: "soon" as const,
        background: "グラフ理論の基礎とその応用。BFS/DFS の動作原理と計算量を理解し、最短経路問題（ダイクストラ法）の擬似コードを書けることが求められます。",
        study_hints: [
          "教科書 第8章のグラフアルゴリズムの節を復習する",
          "BFS と DFS の違いを図示して整理する",
          "ダイクストラ法を小さなグラフで手計算してみる",
          "擬似コードを書いて計算量 O((V+E)logV) を確認する",
        ],
        estimated_minutes: 90,
      },
      {
        task_name: "プロセス管理 確認テスト",
        course_name: "オペレーティングシステム",
        deadline: deadlines[2],
        urgency: "soon" as const,
        background: "プロセスの状態遷移、スケジューリングアルゴリズム（FCFS, SJF, Round Robin, Priority）、デッドロックの検出と回避について出題されます。",
        study_hints: [
          "プロセスの 5 状態遷移図を描いて各遷移条件を確認",
          "各スケジューリングアルゴリズムのガントチャートを練習",
          "デッドロックの 4 条件（相互排他・保持待ち・横取り不可・循環待ち）を暗記",
        ],
        estimated_minutes: 60,
      },
      {
        task_name: "SQL演習課題 第3回",
        course_name: "データベース概論",
        deadline: deadlines[3],
        urgency: "normal" as const,
        background: "JOIN 操作、サブクエリ、集約関数を組み合わせた複合クエリの演習です。正規化理論（第 1〜第 3 正規形）の理解も問われます。",
        study_hints: [
          "INNER JOIN / LEFT JOIN / CROSS JOIN の違いをサンプルデータで確認",
          "GROUP BY + HAVING の使い分けを練習",
          "サブクエリを JOIN に書き換える練習をする",
          "正規化の手順を ER 図とともに整理する",
        ],
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
  writeCache("student_profile", demoStudent);
  writeCache("favorites", { entries: [], total_count: 0, current_page: 1, total_pages: 0 });
}
