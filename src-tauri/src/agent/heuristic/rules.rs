use serde_json::{json, Value};

// ─────────────────────── Heuristic Planner ───────────────────────
//
// Table-driven keyword matching for unambiguous intents.  Falls through to the
// model when no rule matches.  This avoids a model round-trip for the most
// common queries and is cheaper than 20+ if-else branches.

pub(in crate::agent::heuristic) struct HeuristicRule {
    pub(in crate::agent::heuristic) keywords: &'static [&'static str],
    /// Extra keywords that must ALSO match (empty = no extra requirement).
    pub(in crate::agent::heuristic) requires: &'static [&'static str],
    pub(in crate::agent::heuristic) tool: &'static str,
    pub(in crate::agent::heuristic) args: fn() -> Value,
}

pub(in crate::agent::heuristic) const HEURISTIC_RULES: &[HeuristicRule] = &[
    HeuristicRule {
        keywords: &["天気", "weather", "天气"],
        requires: &[],
        tool: "get_weather",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "今天怎么过",
            "今日どう",
            "今日のまとめ",
            "今日の予定",
            "今日のブリーフ",
            "todaysummary",
            "todaybrief",
            "今天有什么安排",
            "一日の流れ",
        ],
        requires: &[],
        tool: "get_today_brief",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["今日の授業", "今天的课", "todayclasses", "todayclass"],
        requires: &[],
        tool: "list_today_classes",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["成績", "grade", "成绩", "単位", "学分"],
        requires: &[],
        tool: "get_grades",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["履修", "registration", "选课"],
        requires: &[],
        tool: "get_registration",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["休講", "停课", "cancelledclass"],
        requires: &[],
        tool: "get_cancellations",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["補講", "makeupclass", "补课"],
        requires: &[],
        tool: "get_makeup_classes",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["教室変更", "roomchange", "换教室"],
        requires: &[],
        tool: "get_room_changes",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["試験時間割", "examtimetable", "考试时间", "考试安排"],
        requires: &[],
        tool: "get_exam_timetable",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["週間サマリー", "weeklysummary", "周总结", "这周总结"],
        requires: &[],
        tool: "get_weekly_summary",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "学生情報",
            "学籍番号",
            "studentprofile",
            "学部",
            "学科",
            "个人资料",
        ],
        requires: &[],
        tool: "get_student_profile",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["お気に入りシラバス", "bookmarksyllabus", "收藏课程"],
        requires: &[],
        tool: "list_syllabus_favorites",
        args: || json!({ "limit": 10 }),
    },
    // Schedule with week offset
    HeuristicRule {
        keywords: &["来週", "nextweek", "下周"],
        requires: &["授業", "课程", "時間割", "课表", "时间", "schedule"],
        tool: "list_week_classes",
        args: || json!({ "offset": 1 }),
    },
    HeuristicRule {
        keywords: &["今週", "thisweek", "本周", "这周"],
        requires: &["授業", "课程", "時間割", "课表", "时间", "schedule"],
        tool: "list_week_classes",
        args: || json!({ "offset": 0 }),
    },
    // Mail
    HeuristicRule {
        keywords: &[
            "メールアドレス",
            "メールアカウント",
            "mail address",
            "邮箱账号",
        ],
        requires: &[],
        tool: "get_mail_profile",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["メール", "mail", "邮箱", "收件箱", "受信"],
        requires: &[],
        tool: "list_recent_mail",
        args: || json!({ "limit": 10 }),
    },
    HeuristicRule {
        keywords: &["お知らせ", "通知", "notification", "公告"],
        requires: &[],
        tool: "list_recent_notifications",
        args: || json!({ "limit": 10 }),
    },
    HeuristicRule {
        keywords: &[
            "pdf",
            "docx",
            "ファイル",
            "附件",
            "添付",
            "ダウンロード",
            "文件",
            "笔记",
            "ノート",
            "live",
            "ライブ",
        ],
        requires: &[],
        tool: "list_downloaded_files",
        args: || json!({ "limit": 10 }),
    },
    HeuristicRule {
        keywords: &[
            "ブラウザ",
            "webview",
            "网页",
            "网页内容",
            "ページ",
            "url",
            "リンク先",
            "website",
            "webpage",
        ],
        requires: &[],
        tool: "list_browser_windows",
        args: || json!({}),
    },
    // Tasks
    HeuristicRule {
        keywords: &[
            "レポート",
            "課題",
            "未提出",
            "report",
            "assignment",
            "作业",
            "报告",
        ],
        requires: &[],
        tool: "list_luna_todos",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &["締め切り", "期限", "deadline", "截止", "いつまで", "due"],
        requires: &[],
        tool: "get_upcoming_deadlines",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "学習ガイド",
            "勉強計画",
            "studyplan",
            "学习计划",
            "やるべきこと",
            "怎么学",
            "どう取り組む",
            "アドバイス",
            "建议",
            "todo分析",
        ],
        requires: &[],
        tool: "get_todo_guide",
        args: || json!({}),
    },
    HeuristicRule {
        keywords: &[
            "最新化",
            "再同期",
            "强制刷新",
            "refreshdata",
            "更新して",
            "同步一下",
            "重新获取",
            "最新取得",
        ],
        requires: &[],
        tool: "refresh_data",
        args: || json!({}),
    },
    // Google Calendar — list only (create/edit/delete require model to extract args)
    HeuristicRule {
        keywords: &[
            "カレンダー一覧",
            "登録したイベント",
            "登録済みイベント",
            "calendarlist",
            "日历列表",
            "已添加的日历",
            "日历事件列表",
            "listcalendar",
        ],
        requires: &[],
        tool: "list_google_calendar_events",
        args: || json!({}),
    },
];
