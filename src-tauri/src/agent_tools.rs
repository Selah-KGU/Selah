//! Read-only tool implementations for the Selah agent.
//!
//! Each tool takes a JSON-encoded argument object (often empty `{}`) and
//! returns a JSON value.  Tools are intentionally few and semantically
//! narrow so a 2B model can reliably pick among them.

use serde_json::{json, Value};
use std::sync::LazyLock;
use tauri::Manager;

use crate::db::Database;

#[derive(Debug, Clone)]
pub(crate) struct ReusableCourseDownload {
    pub path: String,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReusableActivityDetail {
    pub list_fingerprint: String,
    pub source_fingerprint: String,
    pub checked_at: i64,
}

#[path = "agent_tools/academic.rs"]
mod academic;
#[path = "agent_tools/calendar.rs"]
mod calendar;
#[path = "agent_tools/files_browser.rs"]
mod files_browser;
#[path = "agent_tools/insights.rs"]
mod insights;
#[path = "agent_tools/mail_lookup.rs"]
mod mail_lookup;
#[path = "agent_tools/records.rs"]
mod records;

use academic::*;
use calendar::*;
use files_browser::*;
use insights::*;
use mail_lookup::*;
use records::*;

pub(crate) async fn fetch_luna_course_contents(
    app: &tauri::AppHandle,
    luna_id: &str,
) -> Result<crate::luna_parser::LunaCourseContents, String> {
    files_browser::fetch_luna_course_contents_for_download(app, luna_id).await
}

pub(crate) async fn download_luna_course_material(
    app: &tauri::AppHandle,
    luna_id: &str,
    filename: &str,
) -> Result<Option<Value>, String> {
    files_browser::download_course_material_from_contents(app, luna_id, filename).await
}

pub(crate) fn read_downloaded_text(path: &std::path::Path) -> Result<String, String> {
    files_browser::read_supported_download_file(path)
}

/// Full-document extraction (no PDF page cap) — used by the paper checker,
/// which must analyse the whole file rather than an agent-sized excerpt.
pub(crate) fn read_downloaded_text_full(path: &std::path::Path) -> Result<String, String> {
    files_browser::read_supported_download_file_full(path)
}

/// Embedded page images for a scanned PDF whose text layer is empty, so a
/// vision model can still read it. Errors for non-PDF or image-free files.
pub(crate) fn read_downloaded_images(
    path: &std::path::Path,
) -> Result<Vec<crate::ai::ImagePart>, String> {
    files_browser::extract_pdf_images(path)
}

/// Last resort for a PDF with no text layer and no embedded images (e.g. a
/// vector slide deck): rasterize its pages so a vision model can read them.
pub(crate) fn render_pdf_images(
    path: &std::path::Path,
) -> Result<Vec<crate::ai::ImagePart>, String> {
    files_browser::render_pdf_to_images(path)
}

pub(crate) async fn download_luna_activity_attachments(
    app: &tauri::AppHandle,
    luna_id: &str,
    contents: &crate::luna_parser::LunaCourseContents,
    kinds: &[&str],
    reusable_paths: &std::collections::HashMap<String, ReusableCourseDownload>,
    reusable_details: &std::collections::HashMap<String, ReusableActivityDetail>,
    detail_cache_ttl_secs: i64,
    now: i64,
    force_detail_fetch: bool,
) -> Result<Vec<Value>, String> {
    files_browser::download_all_luna_activity_attachments(
        app,
        luna_id,
        contents,
        kinds,
        reusable_paths,
        reusable_details,
        detail_cache_ttl_secs,
        now,
        force_detail_fetch,
    )
    .await
}

/// Maximum number of list items returned by any single tool.
const LIST_CAP: usize = 15;
/// Mail body truncation threshold (bytes).
const MAIL_BODY_CAP: usize = 4096;

// ─────────────────────── Tool Spec & Arg Schema ───────────────────────

/// Describes how to sanitize tool arguments before dispatch.
#[derive(Clone, Copy)]
enum ArgSchema {
    /// No arguments — always returns `{}`.
    Empty,
    /// Single integer arg with key, clamped to 0..=max.
    Int { key: &'static str, max: i64 },
    /// Single text arg with key, max_len.
    Text { key: &'static str, max_len: usize },
    /// Course code arg (alphanumeric, uppercased).
    CourseCode { key: &'static str },
    /// limit + optional keyword.
    LimitKeyword,
    /// Custom sanitizer (message_id with validation).
    MailMessageId,
    /// Downloaded file path (restricted to allowed roots).
    FilePath,
    /// Downloaded file path + body for safe text writes.
    FileWrite,
    /// Luna title + optional attachment name.
    TitleAttachment,
    /// Luna activity detail options (all optional, but should have title/luna_id).
    LunaActivityDetail,
    /// Luna explicit attachment download options.
    DownloadLunaAttachment,
    /// Luna course material explicit download by filename.
    DownloadCourseMaterial,
    /// Optional text arg, omitted when empty.
    OptionalText { key: &'static str, max_len: usize },
    /// URL arg.
    Url,
    /// A known Copilot page plus optional context.
    CopilotPage,
    /// URL + optional explicit filename for the saved file.
    DownloadUrl,
    /// Browser click action.
    BrowserClick,
    /// Browser viewport coordinate click.
    BrowserMouseClick,
    /// Browser viewport coordinate drag.
    BrowserMouseDrag,
    /// Browser fill action.
    BrowserFill,
    /// Browser select action.
    BrowserSelect,
    /// Browser key press action.
    BrowserPress,
    /// Browser scroll action.
    BrowserScroll,
    /// Browser wait action.
    BrowserWait,
    /// Screenshot of a window or target.
    ComputerScreenshot,
    /// System-level mouse click.
    ComputerMouseClick,
    /// System-level mouse drag.
    ComputerMouseDrag,
    /// System-level scroll wheel.
    ComputerScroll,
    /// Google Calendar single-event creation.
    CalendarEvent,
    /// Google Calendar event update (event_id required, rest optional).
    CalendarUpdate,
    /// Google Calendar event delete (event_id required).
    CalendarEventId,
}

struct ToolSpec {
    name: &'static str,
    category: &'static str,
    signature: &'static str,
    purpose: &'static str,
    schema: ArgSchema,
}

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "list_today_classes",
        category: "授業・時間割",
        signature: "list_today_classes()",
        purpose: "今日の授業一覧",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "list_week_classes",
        category: "授業・時間割",
        signature: "list_week_classes(offset: 0|1)",
        purpose: "今週または来週の時間割",
        schema: ArgSchema::Int {
            key: "offset",
            max: 1,
        },
    },
    ToolSpec {
        name: "search_courses",
        category: "授業・時間割",
        signature: "search_courses(query: string)",
        purpose: "科目名・科目コード・教員名から候補を探す",
        schema: ArgSchema::Text {
            key: "query",
            max_len: 80,
        },
    },
    ToolSpec {
        name: "get_course_context",
        category: "授業・時間割",
        signature: "get_course_context(query: string)",
        purpose: "科目の時間割・授業計画・教材・Luna活動をまとめて取得",
        schema: ArgSchema::Text {
            key: "query",
            max_len: 80,
        },
    },
    ToolSpec {
        name: "get_course_detail",
        category: "授業・時間割",
        signature: "get_course_detail(kgc_code: string)",
        purpose: "KGC科目コード指定で詳細・授業計画を取得",
        schema: ArgSchema::CourseCode { key: "kgc_code" },
    },
    ToolSpec {
        name: "get_cancellations",
        category: "授業・時間割",
        signature: "get_cancellations()",
        purpose: "休講情報一覧",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_makeup_classes",
        category: "授業・時間割",
        signature: "get_makeup_classes()",
        purpose: "補講情報一覧",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_room_changes",
        category: "授業・時間割",
        signature: "get_room_changes()",
        purpose: "教室変更情報一覧",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_exam_timetable",
        category: "授業・時間割",
        signature: "get_exam_timetable()",
        purpose: "試験時間割",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "list_luna_todos",
        category: "課題・成績・履修",
        signature: "list_luna_todos()",
        purpose: "Luna の未提出レポート・テスト一覧",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_grades",
        category: "課題・成績・履修",
        signature: "get_grades()",
        purpose: "成績・単位取得状況",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_registration",
        category: "課題・成績・履修",
        signature: "get_registration()",
        purpose: "履修登録科目一覧・単位集計",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "list_syllabus_favorites",
        category: "課題・成績・履修",
        signature: "list_syllabus_favorites(limit?: number, keyword?: string)",
        purpose: "お気に入りシラバス一覧",
        schema: ArgSchema::LimitKeyword,
    },
    ToolSpec {
        name: "list_recent_notifications",
        category: "お知らせ・メール",
        signature: "list_recent_notifications(limit?: number, keyword?: string)",
        purpose: "最新のお知らせ一覧。keywordを渡すと全お知らせをキーワード検索する",
        schema: ArgSchema::LimitKeyword,
    },
    ToolSpec {
        name: "get_notification_detail",
        category: "お知らせ・メール",
        signature: "get_notification_detail(title: string)",
        purpose: "KWIC/KGC/Lunaのお知らせ本文・送信者・添付を取得(直近の一覧キャッシュから検索)",
        schema: ArgSchema::Text {
            key: "title",
            max_len: 200,
        },
    },
    ToolSpec {
        name: "list_recent_mail",
        category: "お知らせ・メール",
        signature: "list_recent_mail(limit?: number, keyword?: string)",
        purpose: "受信メール一覧。keywordを渡すと件名・本文プレビュー・送信者をキーワード検索する",
        schema: ArgSchema::LimitKeyword,
    },
    ToolSpec {
        name: "read_mail",
        category: "お知らせ・メール",
        signature: "read_mail(message_id: string)",
        purpose: "メール本文",
        schema: ArgSchema::MailMessageId,
    },
    ToolSpec {
        name: "list_luna_announcements",
        category: "お知らせ・メール",
        signature: "list_luna_announcements(limit?: number, keyword?: string)",
        purpose: "Luna科目掲示の一覧(keywordで科目名フィルタ)",
        schema: ArgSchema::LimitKeyword,
    },
    ToolSpec {
        name: "get_mail_profile",
        category: "お知らせ・メール",
        signature: "get_mail_profile()",
        purpose: "メールアカウント情報",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_student_profile",
        category: "学生情報・その他",
        signature: "get_student_profile()",
        purpose: "学籍番号・氏名・学部学科など",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_weather",
        category: "学生情報・その他",
        signature: "get_weather()",
        purpose: "今日と明日の天気(西宮キャンパス)",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_weekly_summary",
        category: "学生情報・その他",
        signature: "get_weekly_summary()",
        purpose: "AI生成済みの週間サマリー・来週の準備事項",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_todo_guide",
        category: "課題・成績・履修",
        signature: "get_todo_guide()",
        purpose: "AI生成のタスクガイド・学習ヒント・3日間の計画",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_upcoming_deadlines",
        category: "課題・成績・履修",
        signature: "get_upcoming_deadlines()",
        purpose: "全科目の締め切り間近のレポート・テスト(着手状況付き)",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "get_luna_activity_detail",
        category: "課題・成績・履修",
        signature: "get_luna_activity_detail(title: string, activity_type?: string, luna_id?: string)",
        purpose: "タイトル、種別、luna_id等でレポート/テスト/掲示/お知らせの本文・提出要件・添付を取得",
        schema: ArgSchema::LunaActivityDetail,
    },
    ToolSpec {
        name: "refresh_data",
        category: "学生情報・その他",
        signature: "refresh_data()",
        purpose: "Lunaの課題・お知らせ・提出状況を強制的に最新化(数秒かかる)",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "list_downloaded_files",
        category: "ダウンロードファイル",
        signature: "list_downloaded_files(limit?: number, keyword?: string)",
        purpose: "ダウンロードフォルダ内の最近のファイルを検索・一覧表示",
        schema: ArgSchema::LimitKeyword,
    },
    ToolSpec {
        name: "read_downloaded_file",
        category: "ダウンロードファイル",
        signature: "read_downloaded_file(path?: string, filename?: string)",
        purpose: "ダウンロード済み PDF / DOCX / TXT / MD / JSON / CSV / HTML の本文を抽出して読む。path が不明な場合は filename で検索できる",
        schema: ArgSchema::FilePath,
    },
    ToolSpec {
        name: "write_downloaded_text_file",
        category: "ダウンロードファイル",
        signature: "write_downloaded_text_file(path: string, content: string)",
        purpose: "ダウンロードフォルダ内の .txt / .md / .json / .csv / .html を安全に上書き保存",
        schema: ArgSchema::FileWrite,
    },
    ToolSpec {
        name: "open_downloaded_file",
        category: "ダウンロードファイル",
        signature: "open_downloaded_file(path: string)",
        purpose: "ダウンロード済みファイルをアプリ外部の既定アプリで開く",
        schema: ArgSchema::FilePath,
    },
    ToolSpec {
        name: "delete_downloaded_file",
        category: "ダウンロードファイル",
        signature: "delete_downloaded_file(path: string)",
        purpose: "ダウンロードフォルダ内のファイルを削除する",
        schema: ArgSchema::FilePath,
    },
    ToolSpec {
        name: "download_url",
        category: "ダウンロードファイル",
        signature: "download_url(url: string, filename?: string)",
        purpose: "任意の http(s) URL をダウンロードフォルダに保存する(50MB上限)",
        schema: ArgSchema::DownloadUrl,
    },
    ToolSpec {
        name: "open_luna_attachment",
        category: "ダウンロードファイル",
        signature: "open_luna_attachment(title: string, attachment_name?: string)",
        purpose: "Luna 詳細から添付ファイル/外部資料リンクを探して開く",
        schema: ArgSchema::TitleAttachment,
    },
    ToolSpec {
        name: "download_luna_attachment",
        category: "ダウンロードファイル",
        signature: "download_luna_attachment(title: string, attachment_name?: string, luna_id?: string)",
        purpose: "Luna 詳細から添付ファイル/資料を探してダウンロードする",
        schema: ArgSchema::DownloadLunaAttachment,
    },
    ToolSpec {
        name: "download_course_material",
        category: "ダウンロードファイル",
        signature: "download_course_material(filename: string, title?: string, luna_id?: string)",
        purpose: "指定されたファイル名の授業資料などを自動検索しダウンロードする",
        schema: ArgSchema::DownloadCourseMaterial,
    },
    ToolSpec {
        name: "list_browser_windows",
        category: "ブラウザ",
        signature: "list_browser_windows()",
        purpose: "現在開いているアプリ内ブラウザ一覧",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "open_browser_url",
        category: "ブラウザ",
        signature: "open_browser_url(url: string)",
        purpose: "URL をアプリ内ブラウザ webview で開く",
        schema: ArgSchema::Url,
    },
    ToolSpec {
        name: "open_copilot_page",
        category: "ブラウザ",
        signature: "open_copilot_page(page: new_tab|files|luna|luna_course|luna_activity|kwic|kwic_notification|kwic_cabinet|kgc|kgc_notification, context?: string, luna_id?: string, identifier?: string)",
        purpose: "SelahのCopilotウィンドウで関連ページを開く。luna_courseはcontextに科目名を渡してその科目のLuna詳細(教材・お知らせ等)を開く。個別活動・通知ページでは検索結果のluna_id/identifierを渡して同名項目を区別する。既知の画面を開く場合はURLを推測せずこちらを使う",
        schema: ArgSchema::CopilotPage,
    },
    ToolSpec {
        name: "read_browser_page",
        category: "ブラウザ",
        signature: "read_browser_page(target?: string)",
        purpose: "ブラウザ webview の主内容・見出し・リンク・操作要素を抽出して読む",
        schema: ArgSchema::OptionalText {
            key: "target",
            max_len: 120,
        },
    },
    ToolSpec {
        name: "browser_back",
        category: "ブラウザ",
        signature: "browser_back(target?: string)",
        purpose: "ブラウザ webview を戻る",
        schema: ArgSchema::OptionalText {
            key: "target",
            max_len: 120,
        },
    },
    ToolSpec {
        name: "browser_forward",
        category: "ブラウザ",
        signature: "browser_forward(target?: string)",
        purpose: "ブラウザ webview を進む",
        schema: ArgSchema::OptionalText {
            key: "target",
            max_len: 120,
        },
    },
    ToolSpec {
        name: "browser_reload_page",
        category: "ブラウザ",
        signature: "browser_reload_page(target?: string)",
        purpose: "ブラウザ webview を再読み込み",
        schema: ArgSchema::OptionalText {
            key: "target",
            max_len: 120,
        },
    },
    ToolSpec {
        name: "browser_click",
        category: "ブラウザ",
        signature: "browser_click(target?: string, text?: string, selector?: string, href_contains?: string, index?: number)",
        purpose: "ページ内のリンク・ボタン・タブなどをクリックする",
        schema: ArgSchema::BrowserClick,
    },
    ToolSpec {
        name: "browser_mouse_click",
        category: "ブラウザ",
        signature: "browser_mouse_click(target?: string, x: number, y: number)",
        purpose: "可視ビューポート内の座標をマウスクリックする。text/label/selectorで届かない時だけ使う",
        schema: ArgSchema::BrowserMouseClick,
    },
    ToolSpec {
        name: "browser_mouse_drag",
        category: "ブラウザ",
        signature: "browser_mouse_drag(target?: string, from_x: number, from_y: number, to_x: number, to_y: number, steps?: number)",
        purpose: "可視ビューポート内でマウスドラッグする。スライダーやキャンバス等の座標操作用",
        schema: ArgSchema::BrowserMouseDrag,
    },
    ToolSpec {
        name: "browser_fill",
        category: "ブラウザ",
        signature: "browser_fill(target?: string, label?: string, selector?: string, value: string, index?: number)",
        purpose: "ページ内の入力欄・テキスト欄に値を入力する",
        schema: ArgSchema::BrowserFill,
    },
    ToolSpec {
        name: "browser_select_option",
        category: "ブラウザ",
        signature: "browser_select_option(target?: string, label?: string, selector?: string, value: string, index?: number)",
        purpose: "ページ内の select / プルダウンで選択する",
        schema: ArgSchema::BrowserSelect,
    },
    ToolSpec {
        name: "browser_press",
        category: "ブラウザ",
        signature: "browser_press(target?: string, key: string, selector?: string)",
        purpose: "ページまたは特定要素へ Enter / Tab などのキー入力を送る",
        schema: ArgSchema::BrowserPress,
    },
    ToolSpec {
        name: "browser_scroll",
        category: "ブラウザ",
        signature: "browser_scroll(target?: string, direction?: up|down|top|bottom, amount?: number, selector?: string)",
        purpose: "ページをスクロール、または要素位置へ移動する",
        schema: ArgSchema::BrowserScroll,
    },
    ToolSpec {
        name: "browser_wait_for",
        category: "ブラウザ",
        signature: "browser_wait_for(target?: string, text?: string, selector?: string, timeout_ms?: number)",
        purpose: "指定したテキストや要素が出るまで少し待つ",
        schema: ArgSchema::BrowserWait,
    },
    ToolSpec {
        name: "browser_close",
        category: "ブラウザ",
        signature: "browser_close(target?: string)",
        purpose: "アプリ内ブラウザのウィンドウを閉じる",
        schema: ArgSchema::OptionalText {
            key: "target",
            max_len: 120,
        },
    },
    ToolSpec {
        name: "computer_screenshot",
        category: "コンピュータ操作",
        signature: "computer_screenshot(target?: string)",
        purpose: "対象ウィンドウを実際のPNGスクリーンショットとして取得する",
        schema: ArgSchema::ComputerScreenshot,
    },
    ToolSpec {
        name: "computer_mouse_click",
        category: "コンピュータ操作",
        signature: "computer_mouse_click(target?: string, x: number, y: number, coordinate_space?: screenshot|screen|webview|viewport)",
        purpose: "実際のOSマウスイベントで指定座標をクリックする",
        schema: ArgSchema::ComputerMouseClick,
    },
    ToolSpec {
        name: "computer_mouse_drag",
        category: "コンピュータ操作",
        signature: "computer_mouse_drag(target?: string, from_x: number, from_y: number, to_x: number, to_y: number, steps?: number, coordinate_space?: screenshot|screen|webview|viewport)",
        purpose: "実際のOSマウスイベントで指定座標間をドラッグする",
        schema: ArgSchema::ComputerMouseDrag,
    },
    ToolSpec {
        name: "computer_scroll",
        category: "コンピュータ操作",
        signature: "computer_scroll(target?: string, delta_y: number, x?: number, y?: number, coordinate_space?: screenshot|screen|webview|viewport)",
        purpose: "実際のOSスクロールイベントを送る。target指定時は対象ウィンドウ中央へマウスを移してからスクロールする",
        schema: ArgSchema::ComputerScroll,
    },
    ToolSpec {
        name: "get_today_brief",
        category: "学生情報・その他",
        signature: "get_today_brief()",
        purpose: "今日の授業・差し迫った締切・天気をまとめて取得",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "create_google_calendar_event",
        category: "Google Calendar",
        signature: "create_google_calendar_event(title: string, date: string, start_time: string, end_time: string, location?: string, description?: string)",
        purpose: "Google Calendarに単発イベントを追加する。date=YYYY-MM-DD, start_time/end_time=HH:MM。Google連携が必要。",
        schema: ArgSchema::CalendarEvent,
    },
    ToolSpec {
        name: "list_google_calendar_events",
        category: "Google Calendar",
        signature: "list_google_calendar_events()",
        purpose: "予定を返す。events=Agentが登録したイベント(event_id付き、編集・削除前に呼ぶ)、upcoming_events=Selah専用カレンダー上の今後の実際の予定(時間割同期分含む、読み取り専用)。",
        schema: ArgSchema::Empty,
    },
    ToolSpec {
        name: "delete_google_calendar_event",
        category: "Google Calendar",
        signature: "delete_google_calendar_event(event_id: string)",
        purpose: "Agentが登録したGoogle Calendarイベントをevent_idで削除する。",
        schema: ArgSchema::CalendarEventId,
    },
    ToolSpec {
        name: "update_google_calendar_event",
        category: "Google Calendar",
        signature: "update_google_calendar_event(event_id: string, title?: string, date?: string, start_time?: string, end_time?: string, location?: string, description?: string)",
        purpose: "Agentが登録したGoogle Calendarイベントを編集する。event_id必須、他は変更したフィールドのみ指定。",
        schema: ArgSchema::CalendarUpdate,
    },
];

const TOOL_ALIASES: &[(&str, &str)] = &[
    ("inspect_file", "read_downloaded_file"),
    ("search_notifications", "list_recent_notifications"),
    ("search_mail", "list_recent_mail"),
    ("read_file", "read_downloaded_file"),
    ("view_file", "read_downloaded_file"),
    ("view_downloaded_file", "read_downloaded_file"),
    ("view_downloaded", "read_downloaded_file"),
    ("show_file", "read_downloaded_file"),
    ("display_file", "read_downloaded_file"),
    ("get_file", "read_downloaded_file"),
    ("get_file_content", "read_downloaded_file"),
    ("read_file_content", "read_downloaded_file"),
    ("read_downloaded", "read_downloaded_file"),
    ("read_downloaded_files", "read_downloaded_file"),
    ("inspect_downloaded_file", "read_downloaded_file"),
    ("open_file", "open_downloaded_file"),
    ("open_downloaded", "open_downloaded_file"),
    ("open_downloaded_files", "open_downloaded_file"),
    ("list_files", "list_downloaded_files"),
    ("list_downloads", "list_downloaded_files"),
    ("search_downloaded_files", "list_downloaded_files"),
    ("fetch_lms_course_resources", "list_luna_announcements"),
    ("get_lms_course_resources", "list_luna_announcements"),
    ("list_lms_course_resources", "list_luna_announcements"),
    ("fetch_lms_resources", "list_luna_announcements"),
    ("get_lms_resources", "list_luna_announcements"),
    ("list_lms_resources", "list_luna_announcements"),
    ("fetch_luna_course_resources", "list_luna_announcements"),
    ("get_luna_course_resources", "list_luna_announcements"),
    ("list_luna_course_resources", "list_luna_announcements"),
    ("list_course_resources", "list_luna_announcements"),
    ("get_course_resources", "list_luna_announcements"),
    ("get_luna_detail", "get_luna_activity_detail"),
    ("read_luna_activity", "get_luna_activity_detail"),
    ("get_activity_detail", "get_luna_activity_detail"),
    ("download_attachment", "download_luna_attachment"),
    ("download_material", "download_luna_attachment"),
    ("download_file_by_name", "download_course_material"),
    ("download_material_file", "download_course_material"),
    ("download_course_material_file", "download_course_material"),
    ("download_luna_material", "download_course_material"),
    ("download_luna_file", "download_course_material"),
    ("kg_canvas_download_luna_file", "download_course_material"),
    ("browser_reload", "browser_reload_page"),
    ("reload_browser", "browser_reload_page"),
    ("mouse_click", "browser_mouse_click"),
    ("click_at", "browser_mouse_click"),
    ("browser_click_at", "browser_mouse_click"),
    ("mouse_drag", "browser_mouse_drag"),
    ("drag_mouse", "browser_mouse_drag"),
    ("browser_drag", "browser_mouse_drag"),
    ("select_browser_option", "browser_select_option"),
    ("browser_wait", "browser_wait_for"),
    ("wait_for_browser", "browser_wait_for"),
    ("close_browser", "browser_close"),
    ("open_app_page", "open_copilot_page"),
    ("open_copilot", "open_copilot_page"),
    ("calendar_list_events", "list_google_calendar_events"),
    ("calendar_create_event", "create_google_calendar_event"),
    ("calendar_delete_event", "delete_google_calendar_event"),
    ("calendar_update_event", "update_google_calendar_event"),
];

#[cfg(test)]
const DISPATCH_TOOL_NAMES: &[&str] = &[
    "list_today_classes",
    "list_week_classes",
    "search_courses",
    "get_course_context",
    "get_course_detail",
    "get_cancellations",
    "get_makeup_classes",
    "get_room_changes",
    "get_exam_timetable",
    "list_luna_todos",
    "get_grades",
    "get_registration",
    "list_syllabus_favorites",
    "list_recent_notifications",
    "get_notification_detail",
    "list_recent_mail",
    "read_mail",
    "list_luna_announcements",
    "get_mail_profile",
    "get_student_profile",
    "get_weather",
    "get_weekly_summary",
    "get_todo_guide",
    "get_upcoming_deadlines",
    "get_luna_activity_detail",
    "refresh_data",
    "list_downloaded_files",
    "read_downloaded_file",
    "write_downloaded_text_file",
    "open_downloaded_file",
    "delete_downloaded_file",
    "download_url",
    "open_luna_attachment",
    "download_luna_attachment",
    "download_course_material",
    "list_browser_windows",
    "open_browser_url",
    "open_copilot_page",
    "read_browser_page",
    "browser_back",
    "browser_forward",
    "browser_reload_page",
    "browser_click",
    "browser_mouse_click",
    "browser_mouse_drag",
    "browser_fill",
    "browser_select_option",
    "browser_press",
    "browser_scroll",
    "browser_wait_for",
    "browser_close",
    "computer_screenshot",
    "computer_mouse_click",
    "computer_mouse_drag",
    "computer_scroll",
    "get_today_brief",
    "create_google_calendar_event",
    "list_google_calendar_events",
    "delete_google_calendar_event",
    "update_google_calendar_event",
];

/// Check if a tool name is in the registry.
#[cfg(test)]
pub fn is_known_tool(name: &str) -> bool {
    canonical_tool_name(name).is_some()
}

#[cfg(test)]
pub fn registered_tool_names() -> impl Iterator<Item = &'static str> {
    TOOL_SPECS.iter().map(|spec| spec.name)
}

#[cfg(test)]
pub fn dispatched_tool_names() -> &'static [&'static str] {
    DISPATCH_TOOL_NAMES
}

pub fn exact_tool_name(name: &str) -> Option<&'static str> {
    let trimmed = name
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'');
    TOOL_SPECS
        .iter()
        .find(|spec| spec.name == trimmed)
        .map(|spec| spec.name)
}

pub fn canonical_tool_name(name: &str) -> Option<&'static str> {
    let trimmed = name
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'');
    if let Some(spec) = TOOL_SPECS.iter().find(|s| s.name == trimmed) {
        return Some(spec.name);
    }
    let trimmed = trimmed
        .rsplit([':', '.', '/'])
        .next()
        .unwrap_or(trimmed)
        .trim();
    if let Some(spec) = TOOL_SPECS.iter().find(|s| s.name == trimmed) {
        return Some(spec.name);
    }
    TOOL_ALIASES
        .iter()
        .find_map(|(alias, target)| (*alias == trimmed).then_some(*target))
}

/// Dispatch a single tool call.  Returns a JSON value even on failure so the
/// agent can still surface the error to the user.
pub async fn dispatch(app: &tauri::AppHandle, name: &str, args: &Value) -> Value {
    let Some(name) = canonical_tool_name(name) else {
        return json!({ "error": format!("unknown tool: {}", name) });
    };
    let result: Result<Value, String> = match name {
        "list_today_classes" => list_today_classes(app).await,
        "list_week_classes" => list_week_classes(app, args).await,
        "search_courses" => search_courses(app, args).await,
        "get_course_context" => get_course_context(app, args).await,
        "list_luna_todos" => list_luna_todos(app).await,
        "list_recent_notifications" => list_recent_notifications(app, args).await,
        "get_notification_detail" => get_notification_detail(app, args).await,
        "get_course_detail" => get_course_detail(app, args).await,
        "list_recent_mail" => list_recent_mail(app, args).await,
        "read_mail" => read_mail(app, args).await,
        "list_luna_announcements" => list_luna_announcements(app, args).await,
        "get_student_profile" => get_student_profile(app).await,
        "get_mail_profile" => get_mail_profile(app).await,
        "list_syllabus_favorites" => list_syllabus_favorites(app, args).await,
        "get_grades" => get_grades(app).await,
        "get_cancellations" => get_cancellations(app).await,
        "get_makeup_classes" => get_makeup_classes(app).await,
        "get_room_changes" => get_room_changes(app).await,
        "get_registration" => get_registration(app).await,
        "get_exam_timetable" => get_exam_timetable(app).await,
        "get_weather" => get_weather(app).await,
        "get_weekly_summary" => get_weekly_summary(app).await,
        "get_todo_guide" => get_todo_guide(app).await,
        "get_upcoming_deadlines" => get_upcoming_deadlines(app).await,
        "get_luna_activity_detail" => get_luna_activity_detail(app, args).await,
        "refresh_data" => refresh_data(app).await,
        "list_downloaded_files" => list_downloaded_files(args).await,
        "read_downloaded_file" => read_downloaded_file(app, args).await,
        "write_downloaded_text_file" => write_downloaded_text_file(args).await,
        "open_downloaded_file" => open_downloaded_file(app, args).await,
        "delete_downloaded_file" => delete_downloaded_file(args).await,
        "download_url" => download_url(args).await,
        "open_luna_attachment" => open_luna_attachment(app, args).await,
        "download_luna_attachment" => download_luna_attachment(app, args).await,
        "download_course_material" => download_course_material(app, args).await,
        "list_browser_windows" => list_browser_windows(app).await,
        "open_browser_url" => open_browser_url(app, args).await,
        "open_copilot_page" => open_copilot_page(app, args).await,
        "read_browser_page" => read_browser_page(app, args).await,
        "browser_back" => browser_back(app, args).await,
        "browser_forward" => browser_forward(app, args).await,
        "browser_reload_page" => browser_reload_page(app, args).await,
        "browser_click" => browser_click(app, args).await,
        "browser_mouse_click" => browser_mouse_click(app, args).await,
        "browser_mouse_drag" => browser_mouse_drag(app, args).await,
        "browser_fill" => browser_fill(app, args).await,
        "browser_select_option" => browser_select_option(app, args).await,
        "browser_press" => browser_press(app, args).await,
        "browser_scroll" => browser_scroll(app, args).await,
        "browser_wait_for" => browser_wait_for(app, args).await,
        "browser_close" => browser_close_tool(app, args).await,
        "computer_screenshot" => computer_screenshot(app, args).await,
        "computer_mouse_click" => computer_mouse_click(app, args).await,
        "computer_mouse_drag" => computer_mouse_drag(app, args).await,
        "computer_scroll" => computer_scroll(app, args).await,
        "get_today_brief" => get_today_brief(app).await,
        "create_google_calendar_event" => create_google_calendar_event(app, args).await,
        "list_google_calendar_events" => list_google_calendar_events(app).await,
        "delete_google_calendar_event" => delete_google_calendar_event(app, args).await,
        "update_google_calendar_event" => update_google_calendar_event(app, args).await,
        // Listed in TOOL_SPECS but not yet wired here. Treated as a soft
        // failure so a forgotten dispatch arm cannot panic in production.
        other => {
            log::error!(
                "[agent tools] tool {} is registered but has no dispatch arm",
                other
            );
            Err(format!("tool {} is not implemented yet", other))
        }
    };
    match result {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

/// Static description given to the model during the planning phase.
pub fn tool_catalog_prompt() -> &'static str {
    static TOOL_CATALOG_PROMPT: LazyLock<String> = LazyLock::new(build_tool_catalog_prompt);
    &TOOL_CATALOG_PROMPT
}

/// Names and argument shapes only. The full catalog does not fit Apple's 4096-token window.
pub fn tool_catalog_signatures() -> &'static str {
    static TOOL_CATALOG_SIGNATURES: LazyLock<String> = LazyLock::new(|| {
        TOOL_SPECS
            .iter()
            .map(|spec| format!("- {}", spec.signature))
            .collect::<Vec<_>>()
            .join("\n")
    });
    &TOOL_CATALOG_SIGNATURES
}

fn build_tool_catalog_prompt() -> String {
    let mut out = String::new();
    let mut current_category = "";
    for spec in TOOL_SPECS {
        if spec.category != current_category {
            if !out.is_empty() {
                out.push('\n');
            }
            current_category = spec.category;
            out.push_str(&format!("【{}】\n", spec.category));
        }
        out.push_str(&format!("- {}: {}\n", spec.signature, spec.purpose));
    }
    out.trim_end().to_string()
}

pub fn sanitize_tool_args(name: &str, args: &Value) -> Option<Value> {
    let name = canonical_tool_name(name)?;
    let spec = TOOL_SPECS.iter().find(|s| s.name == name)?;
    sanitize_by_schema(spec.schema, args)
}

fn sanitize_by_schema(schema: ArgSchema, args: &Value) -> Option<Value> {
    match schema {
        ArgSchema::Empty => Some(json!({})),
        ArgSchema::Int { key, max } => {
            let val = args
                .get(key)
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(0, max);
            Some(json!({ key: val }))
        }
        ArgSchema::Text { key, max_len } => {
            let value = sanitize_text_arg(args, key, max_len).or_else(|| {
                if key == "query" {
                    sanitize_text_arg(args, "kgc_code", max_len)
                        .or_else(|| sanitize_text_arg(args, "course_code", max_len))
                        .or_else(|| sanitize_text_arg(args, "code", max_len))
                        .or_else(|| sanitize_text_arg(args, "idnumber", max_len))
                        .or_else(|| sanitize_text_arg(args, "luna_id", max_len))
                        .or_else(|| sanitize_text_arg(args, "course_name", max_len))
                        .or_else(|| sanitize_text_arg(args, "course", max_len))
                        .or_else(|| sanitize_text_arg(args, "keyword", max_len))
                } else {
                    None
                }
            });
            value.map(|v| json!({ key: v }))
        }
        ArgSchema::CourseCode { key } => sanitize_course_code(args, key)
            .or_else(|| sanitize_course_code(args, "course_code"))
            .or_else(|| sanitize_course_code(args, "code"))
            .or_else(|| sanitize_course_code(args, "query"))
            .map(|v| json!({ key: v })),
        ArgSchema::LimitKeyword => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .min(LIST_CAP as u64);
            let keyword = sanitize_text_arg(args, "keyword", 80)
                .or_else(|| sanitize_text_arg(args, "course_name", 80))
                .or_else(|| sanitize_text_arg(args, "course", 80))
                .or_else(|| sanitize_text_arg(args, "query", 80));
            let mut out = json!({ "limit": limit });
            if let Some(keyword) = keyword {
                out["keyword"] = Value::String(keyword);
            }
            Some(out)
        }
        ArgSchema::MailMessageId => sanitize_text_arg(args, "message_id", 200)
            .or_else(|| sanitize_text_arg(args, "id", 200))
            .and_then(|message_id| {
                crate::mail::validate_message_id(&message_id).ok()?;
                Some(json!({ "message_id": message_id }))
            }),
        ArgSchema::FilePath => {
            if let Some(path) = sanitize_file_path_arg(args, "path") {
                Some(json!({ "path": path }))
            } else {
                let filename = sanitize_filename_arg(args, "filename", 240)
                    .or_else(|| sanitize_filename_arg(args, "file_name", 240))?;
                let mut out = serde_json::Map::new();
                out.insert("filename".to_string(), Value::String(filename));
                if let Some(course) = sanitize_text_arg(args, "course_name", 120)
                    .or_else(|| sanitize_text_arg(args, "course", 120))
                {
                    out.insert("course_name".to_string(), Value::String(course));
                }
                Some(Value::Object(out))
            }
        }
        ArgSchema::FileWrite => {
            let path = sanitize_file_path_arg(args, "path")?;
            let content = sanitize_text_blob_arg(args, "content", 100_000)?;
            Some(json!({ "path": path, "content": content }))
        }
        ArgSchema::TitleAttachment => {
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120))
                .or_else(|| sanitize_text_arg(args, "name", 120))?;
            let attachment_name = sanitize_text_arg(args, "attachment_name", 160)
                .or_else(|| sanitize_text_arg(args, "filename", 160))
                .or_else(|| sanitize_text_arg(args, "file_name", 160));
            let mut out = serde_json::Map::new();
            out.insert("title".to_string(), Value::String(title));
            if let Some(name) = attachment_name {
                out.insert("attachment_name".to_string(), Value::String(name));
            }
            Some(Value::Object(out))
        }
        ArgSchema::LunaActivityDetail => {
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120))
                .or_else(|| sanitize_text_arg(args, "name", 120));
            let activity_type = sanitize_text_arg(args, "activity_type", 80)
                .or_else(|| sanitize_text_arg(args, "type", 80));
            let luna_id = sanitize_text_arg(args, "luna_id", 80);
            // Require at least one meaningful field; reject fully-empty calls.
            if title.is_none() && activity_type.is_none() && luna_id.is_none() {
                return None;
            }
            let mut out = serde_json::Map::new();
            if let Some(t) = title {
                out.insert("title".to_string(), Value::String(t));
            }
            if let Some(atype) = activity_type {
                out.insert("activity_type".to_string(), Value::String(atype));
            }
            if let Some(id) = luna_id {
                out.insert("luna_id".to_string(), Value::String(id));
            }
            Some(Value::Object(out))
        }
        ArgSchema::DownloadLunaAttachment => {
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120))
                .or_else(|| sanitize_text_arg(args, "name", 120))?;
            let attachment_name = sanitize_text_arg(args, "attachment_name", 160)
                .or_else(|| sanitize_text_arg(args, "filename", 160))
                .or_else(|| sanitize_text_arg(args, "file_name", 160));
            let luna_id = sanitize_text_arg(args, "luna_id", 80);
            let mut out = serde_json::Map::new();
            out.insert("title".to_string(), Value::String(title));
            if let Some(name) = attachment_name {
                out.insert("attachment_name".to_string(), Value::String(name));
            }
            if let Some(id) = luna_id {
                out.insert("luna_id".to_string(), Value::String(id));
            }
            Some(Value::Object(out))
        }
        ArgSchema::DownloadCourseMaterial => {
            let filename = sanitize_text_arg(args, "filename", 160)
                .or_else(|| sanitize_text_arg(args, "file_name", 160))
                .or_else(|| sanitize_text_arg(args, "attachment_name", 160))
                .or_else(|| sanitize_text_arg(args, "name", 160))?;
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120));
            let luna_id = sanitize_text_arg(args, "luna_id", 80);
            let mut out = serde_json::Map::new();
            out.insert("filename".to_string(), Value::String(filename));
            if let Some(t) = title {
                out.insert("title".to_string(), Value::String(t));
            }
            if let Some(id) = luna_id {
                out.insert("luna_id".to_string(), Value::String(id));
            }
            Some(Value::Object(out))
        }
        ArgSchema::OptionalText { key, max_len } => {
            let val = sanitize_text_arg(args, key, max_len);
            let mut out = serde_json::Map::new();
            if let Some(val) = val {
                out.insert(key.to_string(), Value::String(val));
            }
            Some(Value::Object(out))
        }
        ArgSchema::Url => sanitize_url_arg(args, "url").map(|url| json!({ "url": url })),
        ArgSchema::CopilotPage => sanitize_copilot_page_args(args),
        ArgSchema::DownloadUrl => {
            let url = sanitize_url_arg(args, "url")?;
            let filename = sanitize_text_arg(args, "filename", 200);
            let mut out = serde_json::Map::new();
            out.insert("url".into(), Value::String(url));
            if let Some(name) = filename {
                out.insert("filename".into(), Value::String(name));
            }
            Some(Value::Object(out))
        }
        ArgSchema::BrowserClick => sanitize_browser_click_args(args),
        ArgSchema::BrowserMouseClick => sanitize_browser_mouse_click_args(args),
        ArgSchema::BrowserMouseDrag => sanitize_browser_mouse_drag_args(args),
        ArgSchema::BrowserFill => sanitize_browser_fill_args(args),
        ArgSchema::BrowserSelect => sanitize_browser_select_args(args),
        ArgSchema::BrowserPress => sanitize_browser_press_args(args),
        ArgSchema::BrowserScroll => sanitize_browser_scroll_args(args),
        ArgSchema::BrowserWait => sanitize_browser_wait_args(args),
        ArgSchema::ComputerScreenshot => sanitize_computer_screenshot_args(args),
        ArgSchema::ComputerMouseClick => sanitize_computer_mouse_click_args(args),
        ArgSchema::ComputerMouseDrag => sanitize_computer_mouse_drag_args(args),
        ArgSchema::ComputerScroll => sanitize_computer_scroll_args(args),
        ArgSchema::CalendarEvent => sanitize_calendar_event_args(args),
        ArgSchema::CalendarUpdate => sanitize_calendar_update_args(args),
        ArgSchema::CalendarEventId => {
            sanitize_text_arg(args, "event_id", 200).map(|id| json!({ "event_id": id }))
        }
    }
}

fn sanitize_text_arg(args: &Value, key: &str, max_len: usize) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() {
        return None;
    }
    let mut out = value.chars().take(max_len).collect::<String>();
    out = out.replace(['\n', '\r'], " ");
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn sanitize_copilot_page_args(args: &Value) -> Option<Value> {
    let page = sanitize_text_arg(args, "page", 40)?.to_ascii_lowercase();
    if !matches!(
        page.as_str(),
        "new_tab"
            | "files"
            | "luna"
            | "luna_course"
            | "luna_activity"
            | "kwic"
            | "kwic_notification"
            | "kwic_cabinet"
            | "kgc"
            | "kgc_notification"
    ) {
        return None;
    }
    let mut out = serde_json::Map::new();
    let context = sanitize_text_arg(args, "context", 120)
        .or_else(|| sanitize_text_arg(args, "course", 120))
        .or_else(|| sanitize_text_arg(args, "course_name", 120));
    let identifier = sanitize_text_arg(args, "identifier", 240)
        .or_else(|| sanitize_text_arg(args, "item_id", 240))
        .or_else(|| sanitize_text_arg(args, "id", 240));
    if matches!(page.as_str(), "kwic_notification" | "kgc_notification")
        && context.is_none()
        && identifier.is_none()
    {
        return None;
    }
    if matches!(page.as_str(), "luna_activity" | "luna_course") && context.is_none() {
        return None;
    }
    if matches!(page.as_str(), "luna_activity" | "luna_course") {
        if let Some(luna_id) = sanitize_text_arg(args, "luna_id", 80) {
            out.insert("luna_id".into(), Value::String(luna_id));
        }
    }
    out.insert("page".into(), Value::String(page));
    if let Some(context) = context {
        out.insert("context".into(), Value::String(context));
    }
    if let Some(identifier) = identifier {
        out.insert("identifier".into(), Value::String(identifier));
    }
    Some(Value::Object(out))
}

enum LunaCopilotDestination {
    Detail(String),
    External(String),
}

fn luna_path_query(path: &str, key: &str) -> Option<String> {
    let full = if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", crate::config::LUNA_BASE, path)
    };
    url::Url::parse(&full)
        .ok()?
        .query_pairs()
        .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
        .filter(|value| !value.is_empty())
}

fn luna_activity_copilot_destination(row: &crate::db::LunaActivityRow) -> LunaCopilotDestination {
    let encoded_path = urlencoding::encode(&row.detail_path);
    let encoded_title = urlencoding::encode(&row.title);
    let encoded_id = urlencoding::encode(&row.luna_id);
    let encoded_period = urlencoding::encode(&row.period);
    let encoded_status = urlencoding::encode(&row.status);
    let params = match row.activity_type.as_str() {
        "announcement" => luna_path_query(&row.detail_path, "informationId")
            .map(|info_id| {
                format!(
                    "mode=announcement&title={}&idnumber={}&infoId={}",
                    encoded_title,
                    encoded_id,
                    urlencoding::encode(&info_id),
                )
            })
            .unwrap_or_else(|| format!("path={}&title={}", encoded_path, encoded_title)),
        "report" => {
            let report_id = luna_path_query(&row.detail_path, "reportId").unwrap_or_default();
            format!(
                "mode=report&path={}&title={}&idnumber={}&reportId={}&period={}",
                encoded_path,
                encoded_title,
                encoded_id,
                urlencoding::encode(&report_id),
                encoded_period,
            )
        }
        "discussion" => format!(
            "mode=discussion&path={}&title={}",
            encoded_path, encoded_title
        ),
        "exam" => {
            let url = if row.detail_path.starts_with("http://")
                || row.detail_path.starts_with("https://")
            {
                row.detail_path.clone()
            } else {
                format!("{}{}", crate::config::LUNA_BASE, row.detail_path)
            };
            return LunaCopilotDestination::External(url);
        }
        _ => format!(
            "path={}&title={}&idnumber={}&period={}&status={}",
            encoded_path, encoded_title, encoded_id, encoded_period, encoded_status,
        ),
    };
    LunaCopilotDestination::Detail(params)
}

async fn open_copilot_page(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let page = args
        .get("page")
        .and_then(|v| v.as_str())
        .ok_or("pageは必須です")?;
    let context = args
        .get("context")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let luna_id = args
        .get("luna_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let identifier = args
        .get("identifier")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Whether the Copilot window already existed before this call. If not, this
    // call is what opens it (e.g. the main-window agent opening a Copilot page),
    // so afterwards we also bring up the sidebar agent for a continuous chat.
    let copilot_window_existed = app.get_window("document-tabs").is_some();

    let (title, target) = match page {
        "new_tab" => {
            let tab = crate::document_tabs::open_new_tab(app)?;
            (tab.title, tab.target)
        }
        "files" => {
            let tab = crate::document_tabs::open_files_tab(
                app,
                context.map(str::to_string),
                "ファイル".to_string(),
            )?;
            (tab.title, tab.target)
        }
        "luna" => {
            let tab = crate::document_tabs::open_external_tab(
                app,
                crate::config::LUNA_BASE.to_string(),
                Some("Luna".to_string()),
            )?;
            (tab.title, tab.target)
        }
        "luna_course" => {
            let query = context.ok_or("luna_courseにはcontext(科目名)が必要です")?;
            let needle = normalize_text(query);
            let db = app.state::<Database>();
            let courses = db.get_luna_courses().unwrap_or_default();
            let candidates = courses
                .iter()
                .filter(|row| luna_id.is_none_or(|id| row.luna_id == id))
                .collect::<Vec<_>>();
            let row = candidates
                .iter()
                .find(|row| normalize_text(&row.name) == needle)
                .or_else(|| {
                    candidates.iter().find(|row| {
                        let name = normalize_text(&row.name);
                        name.contains(&needle) || needle.contains(&name)
                    })
                })
                .copied()
                .ok_or_else(|| format!("「{}」に一致する科目が見つかりません", query))?;
            // Luna course-top detail page; the course detail surface uses the
            // course's luna_id as its idnumber (see Timetable/HomePage).
            let params = format!(
                "mode=course&idnumber={}&title={}",
                urlencoding::encode(&row.luna_id),
                urlencoding::encode(&row.name),
            );
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, row.name.clone())?;
            (tab.title, tab.target)
        }
        "luna_activity" => {
            let query = context.ok_or("luna_activityにはcontextが必要です")?;
            let needle = normalize_text(query);
            let db = app.state::<Database>();
            let activities = db.get_all_luna_activities().unwrap_or_default();
            let candidates = activities
                .iter()
                .filter(|row| luna_id.is_none_or(|id| row.luna_id == id))
                .collect::<Vec<_>>();
            let row = candidates
                .iter()
                .find(|row| normalize_text(&row.title) == needle)
                .or_else(|| {
                    candidates.iter().find(|row| {
                        let title = normalize_text(&row.title);
                        title.contains(&needle) || needle.contains(&title)
                    })
                })
                .copied()
                .ok_or_else(|| format!("「{}」に一致するLuna活動が見つかりません", query))?;
            if row.detail_path.is_empty() {
                return Err(format!("「{}」には詳細ページがありません", row.title));
            }
            match luna_activity_copilot_destination(row) {
                LunaCopilotDestination::Detail(params) => {
                    let tab = crate::document_tabs::open_university_detail_tab(
                        app,
                        params,
                        row.title.clone(),
                    )?;
                    (tab.title, tab.target)
                }
                LunaCopilotDestination::External(url) => {
                    let tab =
                        crate::document_tabs::open_external_tab(app, url, Some(row.title.clone()))?;
                    (tab.title, tab.target)
                }
            }
        }
        "kwic" => {
            let tab = crate::document_tabs::open_external_tab(
                app,
                crate::config::KWIC_BASE.to_string(),
                Some("KWIC".to_string()),
            )?;
            (tab.title, tab.target)
        }
        "kwic_notification" => {
            let needle = context.map(normalize_text);
            let db = app.state::<Database>();
            let (json_str, _) = db
                .get_data_cache("kwic_home")
                .map_err(|e| e.to_string())?
                .ok_or("KWIC通知キャッシュがありません。先に通知を取得してください")?;
            let home: crate::kwic_commands::KwicPortalHome =
                serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
            let item = home
                .sections
                .iter()
                .flat_map(|section| section.items.iter())
                .find(|item| identifier.is_some_and(|id| item.id == id))
                .or_else(|| {
                    home.sections
                        .iter()
                        .flat_map(|section| section.items.iter())
                        .find(|item| {
                            needle.as_ref().is_some_and(|needle| {
                                let title = normalize_text(&item.title);
                                title == needle.as_str()
                                    || title.contains(needle)
                                    || needle.contains(&title)
                            })
                        })
                })
                .ok_or_else(|| {
                    format!(
                        "指定されたKWIC通知が見つかりません: {}",
                        context.or(identifier).unwrap_or("")
                    )
                })?;
            if item.id.is_empty() {
                return Err(format!("「{}」には詳細IDがありません", item.title));
            }
            let params = format!(
                "mode=kwic&informationId={}&informationType={}&personCategoryCd={}&categoryCd={}&title={}",
                urlencoding::encode(&item.id),
                urlencoding::encode(&item.information_type),
                urlencoding::encode(&item.person_category_cd),
                urlencoding::encode(&item.category_cd),
                urlencoding::encode(&item.title),
            );
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, item.title.clone())?;
            (tab.title, tab.target)
        }
        "kwic_cabinet" => {
            let title = context.unwrap_or("学生キャビネット");
            let params = format!("mode=kwicCabinet&title={}", urlencoding::encode(title));
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, title.to_string())?;
            (tab.title, tab.target)
        }
        "kgc" => {
            let tab = crate::document_tabs::open_external_tab(
                app,
                crate::config::KG_COURSE_BASE.to_string(),
                Some("KG Course".to_string()),
            )?;
            (tab.title, tab.target)
        }
        "kgc_notification" => {
            let needle = context.map(normalize_text);
            let db = app.state::<Database>();
            let (json_str, _) = db
                .get_data_cache("notifications")
                .map_err(|e| e.to_string())?
                .ok_or("KGC通知キャッシュがありません。先に通知を取得してください")?;
            let data: crate::parser::NotificationsData =
                serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
            let item = data
                .entries
                .iter()
                .find(|item| identifier.is_some_and(|id| item.id == id))
                .or_else(|| {
                    data.entries.iter().find(|item| {
                        needle.as_ref().is_some_and(|needle| {
                            let title = normalize_text(&item.title);
                            title == needle.as_str()
                                || title.contains(needle)
                                || needle.contains(&title)
                        })
                    })
                })
                .ok_or_else(|| {
                    format!(
                        "指定されたKGC通知が見つかりません: {}",
                        context.or(identifier).unwrap_or("")
                    )
                })?;
            if item.url.is_empty() {
                return Err(format!("「{}」には詳細ページがありません", item.title));
            }
            let path = if item.url.starts_with('/') {
                item.url.clone()
            } else if item.url.starts_with("http") {
                return Err("KGC外部リンクはCopilot詳細ページで開けません".into());
            } else {
                format!("/uniasv2/{}", item.url)
            };
            let params = format!(
                "mode=kgc&path={}&name={}",
                urlencoding::encode(&path),
                urlencoding::encode(&item.title),
            );
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, item.title.clone())?;
            (tab.title, tab.target)
        }
        _ => return Err(format!("未対応のCopilotページです: {}", page)),
    };

    // Newly opening the Copilot window (e.g. from the main-window agent): also
    // reveal the sidebar agent so the conversation can continue there.
    if !copilot_window_existed {
        let _ = crate::document_tabs::open_agent_workspace(app);
    }

    Ok(json!({
        "page": page,
        "title": title,
        "target": target,
        "opened_in": "copilot",
    }))
}

fn sanitize_filename_arg(args: &Value, key: &str, max_len: usize) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() || value.chars().count() > max_len {
        return None;
    }
    if value.contains('\0') || value.contains('/') || value.contains('\\') {
        return None;
    }
    let out = value.replace(['\n', '\r'], " ");
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn sanitize_course_code(args: &Value, key: &str) -> Option<String> {
    let raw = sanitize_text_arg(args, key, 32)?;
    let code = raw.to_uppercase();
    if code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some(code)
    } else {
        None
    }
}

fn sanitize_file_path_arg(args: &Value, key: &str) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() || value.len() > 600 {
        return None;
    }
    if value.contains('\0') {
        return None;
    }
    Some(value.to_string())
}

fn sanitize_text_blob_arg(args: &Value, key: &str, max_len: usize) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?;
    if value.is_empty() || value.len() > max_len {
        return None;
    }
    Some(value.replace('\0', ""))
}

fn sanitize_url_arg(args: &Value, key: &str) -> Option<String> {
    let raw = args.get(key).and_then(|v| v.as_str())?.trim();
    if raw.is_empty() || raw.len() > 1000 {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}

fn sanitize_browser_target_arg(args: &Value) -> Option<String> {
    sanitize_text_arg(args, "target", 120)
}

fn sanitize_selector_arg(args: &Value, key: &str, max_len: usize) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() || value.len() > max_len || value.contains('\0') {
        return None;
    }
    let value = value.replace(['\n', '\r'], " ");
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn sanitize_small_index(args: &Value, key: &str, max: u64) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v.min(max))
}

fn sanitize_browser_coord(args: &Value, key: &str) -> Option<u64> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v.min(20_000))
}

fn sanitize_browser_click_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let text = sanitize_text_arg(args, "text", 120);
    let href_contains = sanitize_text_arg(args, "href_contains", 240);
    let index = sanitize_small_index(args, "index", 20).unwrap_or(0);
    if selector.is_none() && text.is_none() && href_contains.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    if let Some(text) = text {
        out.insert("text".into(), Value::String(text));
    }
    if let Some(href_contains) = href_contains {
        out.insert("href_contains".into(), Value::String(href_contains));
    }
    if index > 0 {
        out.insert("index".into(), Value::Number(index.into()));
    }
    Some(Value::Object(out))
}

fn sanitize_browser_mouse_click_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let x = sanitize_browser_coord(args, "x")?;
    let y = sanitize_browser_coord(args, "y")?;
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("x".into(), Value::Number(x.into()));
    out.insert("y".into(), Value::Number(y.into()));
    Some(Value::Object(out))
}

fn sanitize_browser_mouse_drag_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let from_x =
        sanitize_browser_coord(args, "from_x").or_else(|| sanitize_browser_coord(args, "fromX"))?;
    let from_y =
        sanitize_browser_coord(args, "from_y").or_else(|| sanitize_browser_coord(args, "fromY"))?;
    let to_x =
        sanitize_browser_coord(args, "to_x").or_else(|| sanitize_browser_coord(args, "toX"))?;
    let to_y =
        sanitize_browser_coord(args, "to_y").or_else(|| sanitize_browser_coord(args, "toY"))?;
    let steps = args
        .get("steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(2, 24);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("from_x".into(), Value::Number(from_x.into()));
    out.insert("from_y".into(), Value::Number(from_y.into()));
    out.insert("to_x".into(), Value::Number(to_x.into()));
    out.insert("to_y".into(), Value::Number(to_y.into()));
    out.insert("steps".into(), Value::Number(steps.into()));
    Some(Value::Object(out))
}

fn sanitize_browser_fill_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let label = sanitize_text_arg(args, "label", 120);
    let value = sanitize_text_blob_arg(args, "value", 2000)?;
    let index = sanitize_small_index(args, "index", 20).unwrap_or(0);
    if selector.is_none() && label.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    if let Some(label) = label {
        out.insert("label".into(), Value::String(label));
    }
    out.insert("value".into(), Value::String(value));
    if index > 0 {
        out.insert("index".into(), Value::Number(index.into()));
    }
    Some(Value::Object(out))
}

fn sanitize_browser_select_args(args: &Value) -> Option<Value> {
    sanitize_browser_fill_args(args)
}

fn normalize_browser_key(raw: &str) -> Option<String> {
    let key = raw.trim();
    if key.is_empty() || key.len() > 32 {
        return None;
    }
    let normalized = match key.to_ascii_lowercase().as_str() {
        "enter" => "Enter",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "arrowup" | "up" => "ArrowUp",
        "arrowdown" | "down" => "ArrowDown",
        "arrowleft" | "left" => "ArrowLeft",
        "arrowright" | "right" => "ArrowRight",
        "space" | "spacebar" => " ",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "home" => "Home",
        "end" => "End",
        _ => key,
    };
    Some(normalized.to_string())
}

fn sanitize_browser_press_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .and_then(normalize_browser_key)?;
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    out.insert("key".into(), Value::String(key));
    Some(Value::Object(out))
}

fn sanitize_browser_scroll_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| matches!(v.as_str(), "up" | "down" | "top" | "bottom"))
        .unwrap_or_else(|| "down".into());
    let amount = args
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(900)
        .clamp(80, 4000);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    out.insert("direction".into(), Value::String(direction));
    out.insert("amount".into(), Value::Number(amount.into()));
    Some(Value::Object(out))
}

fn sanitize_browser_wait_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let selector = sanitize_selector_arg(args, "selector", 240);
    let text = sanitize_text_arg(args, "text", 160);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(3000)
        .clamp(400, 12_000);
    if selector.is_none() && text.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    if let Some(selector) = selector {
        out.insert("selector".into(), Value::String(selector));
    }
    if let Some(text) = text {
        out.insert("text".into(), Value::String(text));
    }
    out.insert("timeout_ms".into(), Value::Number(timeout_ms.into()));
    Some(Value::Object(out))
}

fn sanitize_coordinate_space_arg(args: &Value) -> Option<String> {
    args.get("coordinate_space")
        .or_else(|| args.get("coordinateSpace"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| {
            matches!(
                v.as_str(),
                "screenshot" | "target" | "screen" | "webview" | "viewport"
            )
        })
}

fn sanitize_computer_screenshot_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    Some(Value::Object(out))
}

fn sanitize_computer_mouse_click_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let x = sanitize_browser_coord(args, "x")?;
    let y = sanitize_browser_coord(args, "y")?;
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("x".into(), Value::Number(x.into()));
    out.insert("y".into(), Value::Number(y.into()));
    if let Some(space) = sanitize_coordinate_space_arg(args) {
        out.insert("coordinate_space".into(), Value::String(space));
    }
    Some(Value::Object(out))
}

fn sanitize_computer_mouse_drag_args(args: &Value) -> Option<Value> {
    let mut out = sanitize_browser_mouse_drag_args(args)?;
    if let (Value::Object(map), Some(space)) = (&mut out, sanitize_coordinate_space_arg(args)) {
        map.insert("coordinate_space".into(), Value::String(space));
    }
    Some(out)
}

fn sanitize_computer_scroll_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let delta_y = args
        .get("delta_y")
        .or_else(|| args.get("deltaY"))
        .and_then(|v| v.as_i64())
        .or_else(|| {
            args.get("direction").and_then(|v| v.as_str()).map(|dir| {
                match dir.trim().to_ascii_lowercase().as_str() {
                    "up" => 700,
                    "down" => -700,
                    _ => 0,
                }
            })
        })
        .unwrap_or(-700)
        .clamp(-5000, 5000);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("delta_y".into(), Value::Number(delta_y.into()));
    let x = args.get("x").and_then(|v| v.as_i64());
    let y = args.get("y").and_then(|v| v.as_i64());
    if let (Some(x), Some(y)) = (x, y) {
        out.insert("x".into(), Value::Number(x.clamp(-200_000, 200_000).into()));
        out.insert("y".into(), Value::Number(y.clamp(-200_000, 200_000).into()));
    }
    if let Some(space) = sanitize_coordinate_space_arg(args) {
        out.insert("coordinate_space".into(), Value::String(space));
    }
    Some(Value::Object(out))
}

fn sanitize_calendar_event_args(args: &Value) -> Option<Value> {
    // title, date, start_time, end_time are required.
    let title = sanitize_text_arg(args, "title", 200)?;
    let date = sanitize_text_arg(args, "date", 10)?;
    let start_time = sanitize_text_arg(args, "start_time", 5)?;
    let end_time = sanitize_text_arg(args, "end_time", 5)?;
    // Basic structural validation — real format validation happens inside the tool.
    if date.len() != 10 || !date.chars().nth(4).map(|c| c == '-').unwrap_or(false) {
        return None;
    }
    if start_time.len() != 5 || end_time.len() != 5 {
        return None;
    }
    let location = sanitize_text_arg(args, "location", 200);
    let description = sanitize_text_arg(args, "description", 500);
    let mut out = serde_json::Map::new();
    out.insert("title".into(), Value::String(title));
    out.insert("date".into(), Value::String(date));
    out.insert("start_time".into(), Value::String(start_time));
    out.insert("end_time".into(), Value::String(end_time));
    if let Some(loc) = location {
        out.insert("location".into(), Value::String(loc));
    }
    if let Some(desc) = description {
        out.insert("description".into(), Value::String(desc));
    }
    Some(Value::Object(out))
}

fn sanitize_calendar_update_args(args: &Value) -> Option<Value> {
    // event_id is required; all other fields are optional.
    let event_id = sanitize_text_arg(args, "event_id", 200)?;
    let title = sanitize_text_arg(args, "title", 200);
    let date = sanitize_text_arg(args, "date", 10)
        .filter(|d| d.len() == 10 && d.chars().nth(4).map(|c| c == '-').unwrap_or(false));
    let start_time = sanitize_text_arg(args, "start_time", 5).filter(|t| t.len() == 5);
    let end_time = sanitize_text_arg(args, "end_time", 5).filter(|t| t.len() == 5);
    let location = sanitize_text_arg(args, "location", 200);
    let description = sanitize_text_arg(args, "description", 500);
    let mut out = serde_json::Map::new();
    out.insert("event_id".into(), Value::String(event_id));
    if let Some(v) = title {
        out.insert("title".into(), Value::String(v));
    }
    if let Some(v) = date {
        out.insert("date".into(), Value::String(v));
    }
    if let Some(v) = start_time {
        out.insert("start_time".into(), Value::String(v));
    }
    if let Some(v) = end_time {
        out.insert("end_time".into(), Value::String(v));
    }
    // Pass through location/description even if empty so tool knows to clear them.
    if args.get("location").is_some() {
        out.insert(
            "location".into(),
            location.map(Value::String).unwrap_or(Value::Null),
        );
    }
    if args.get("description").is_some() {
        out.insert(
            "description".into(),
            description.map(Value::String).unwrap_or(Value::Null),
        );
    }
    Some(Value::Object(out))
}

/// Map simplified Chinese characters to their Japanese kanji equivalents
/// so cross-lingual course searches work.
pub(crate) fn normalize_cjk_char(c: char) -> char {
    match c {
        '际' => '際',
        '关' => '関',
        '历' => '歴',
        '础' => '礎',
        '现' => '現',
        '经' => '経',
        '济' => '済',
        '统' => '統',
        '计' => '計',
        '术' => '術',
        '语' => '語',
        '论' => '論',
        '电' => '電',
        '机' => '機',
        '业' => '業',
        '环' => '環',
        '药' => '薬',
        '设' => '設',
        '构' => '構',
        '门' => '門',
        '发' => '発',
        '报' => '報',
        '导' => '導',
        '义' => '義',
        '种' => '種',
        '类' => '類',
        '图' => '図',
        '馆' => '館',
        '问' => '問',
        '题' => '題',
        '对' => '対',
        '乐' => '楽',
        '书' => '書',
        '习' => '習',
        '练' => '練',
        '传' => '伝',
        '识' => '識',
        '认' => '認',
        '讲' => '講',
        '谈' => '談',
        '词' => '詞',
        '读' => '読',
        '记' => '記',
        '证' => '証',
        '评' => '評',
        '试' => '試',
        '验' => '験',
        '实' => '実',
        '达' => '達',
        '远' => '遠',
        '运' => '運',
        '进' => '進',
        '选' => '選',
        '过' => '過',
        '专' => '専',
        '组' => '組',
        '绍' => '紹',
        '细' => '細',
        '约' => '約',
        '线' => '線',
        '确' => '確',
        '长' => '長',
        '广' => '広',
        '应' => '応',
        '贸' => '貿',
        '资' => '資',
        '连' => '連',
        '层' => '層',
        '积' => '積',
        '质' => '質',
        '单' => '単',
        '变' => '変',
        '观' => '観',
        '规' => '規',
        '视' => '視',
        '战' => '戦',
        '动' => '動',
        '产' => '産',
        '营' => '営',
        '织' => '織',
        '举' => '挙',
        '兴' => '興',
        '项' => '項',
        '归' => '帰',
        '满' => '満',
        '难' => '難',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn tool_aliases_point_to_registered_tools() {
        let registered = TOOL_SPECS
            .iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        let mut aliases = BTreeSet::new();
        for (alias, target) in TOOL_ALIASES {
            assert!(aliases.insert(*alias), "duplicate alias: {alias}");
            assert!(
                registered.contains(target),
                "alias {alias} points to missing tool {target}"
            );
            assert_eq!(canonical_tool_name(alias), Some(*target));
        }
    }

    #[test]
    fn tool_catalog_prompt_is_cached() {
        let first = tool_catalog_prompt().as_ptr();
        let second = tool_catalog_prompt().as_ptr();
        assert_eq!(first, second);
        assert!(tool_catalog_prompt().contains("open_browser_url(url: string)"));
        assert!(tool_catalog_prompt().contains("open_copilot_page(page:"));
    }

    #[test]
    fn copilot_page_args_are_structured_and_whitelisted() {
        assert_eq!(
            sanitize_tool_args(
                "open_copilot_page",
                &json!({"page":"files","course_name":"政治学"})
            ),
            Some(json!({"page":"files","context":"政治学"}))
        );
        assert_eq!(
            sanitize_tool_args("open_copilot_page", &json!({"page":"luna"})),
            Some(json!({"page":"luna"}))
        );
        assert_eq!(
            sanitize_tool_args(
                "open_copilot_page",
                &json!({"page":"luna_activity","context":"第7回課題","luna_id":"LUNA-42"})
            ),
            Some(json!({"page":"luna_activity","context":"第7回課題","luna_id":"LUNA-42"}))
        );
        assert!(
            sanitize_tool_args("open_copilot_page", &json!({"page":"luna_activity"})).is_none()
        );
        assert_eq!(
            sanitize_tool_args(
                "open_copilot_page",
                &json!({"page":"luna_course","course":"政治学","luna_id":"LUNA-7"})
            ),
            Some(json!({"page":"luna_course","context":"政治学","luna_id":"LUNA-7"}))
        );
        assert!(sanitize_tool_args("open_copilot_page", &json!({"page":"luna_course"})).is_none());
        assert_eq!(
            sanitize_tool_args(
                "open_copilot_page",
                &json!({"page":"kwic_notification","context":"履修登録のお知らせ","id":"kwic-7"})
            ),
            Some(
                json!({"page":"kwic_notification","context":"履修登録のお知らせ","identifier":"kwic-7"})
            )
        );
        assert_eq!(
            sanitize_tool_args(
                "open_copilot_page",
                &json!({"page":"kgc_notification","identifier":"kgc-9"})
            ),
            Some(json!({"page":"kgc_notification","identifier":"kgc-9"}))
        );
        assert_eq!(
            sanitize_tool_args("open_copilot_page", &json!({"page":"kwic_cabinet"})),
            Some(json!({"page":"kwic_cabinet"}))
        );
        assert!(
            sanitize_tool_args("open_copilot_page", &json!({"page":"kgc_notification"})).is_none()
        );
        assert!(sanitize_tool_args("open_copilot_page", &json!({"page":"made-up-page"})).is_none());
    }

    #[test]
    fn luna_copilot_destination_uses_activity_specific_surface() {
        let row = |activity_type: &str, detail_path: &str| crate::db::LunaActivityRow {
            luna_id: "LUNA-42".into(),
            activity_type: activity_type.into(),
            title: "第7回課題".into(),
            period: "2026-06-20".into(),
            status: "未提出".into(),
            detail_path: detail_path.into(),
        };

        let LunaCopilotDestination::Detail(announcement) = luna_activity_copilot_destination(&row(
            "announcement",
            "/lms/coursetop/information/listdetail?idnumber=LUNA-42&informationId=7",
        )) else {
            panic!("announcement should use detail surface");
        };
        assert!(announcement.starts_with("mode=announcement&"));
        assert!(announcement.contains("infoId=7"));

        let LunaCopilotDestination::Detail(report) = luna_activity_copilot_destination(&row(
            "report",
            "/lms/course/report/submission?idnumber=LUNA-42&reportId=9",
        )) else {
            panic!("report should use detail surface");
        };
        assert!(report.starts_with("mode=report&"));
        assert!(report.contains("reportId=9"));

        let LunaCopilotDestination::Detail(discussion) = luna_activity_copilot_destination(&row(
            "discussion",
            "/lms/course/forums/themetop?forumId=3",
        )) else {
            panic!("discussion should use detail surface");
        };
        assert!(discussion.starts_with("mode=discussion&"));

        let LunaCopilotDestination::External(exam) =
            luna_activity_copilot_destination(&row("exam", "/lms/course/exam/start?id=5"))
        else {
            panic!("exam should use external Luna page");
        };
        assert_eq!(
            exam,
            "https://luna.kwansei.ac.jp/lms/course/exam/start?id=5"
        );
    }
}
