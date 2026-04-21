//! Read-only tool implementations for the Selah agent.
//!
//! Each tool takes a JSON-encoded argument object (often empty `{}`) and
//! returns a JSON value.  Tools are intentionally few and semantically
//! narrow so a 2B model can reliably pick among them.

use serde_json::{json, Value};
use tauri::Manager;

use crate::db::Database;

#[path = "agent_tools/academic.rs"]
mod academic;
#[path = "agent_tools/files_browser.rs"]
mod files_browser;
#[path = "agent_tools/insights.rs"]
mod insights;
#[path = "agent_tools/mail_lookup.rs"]
mod mail_lookup;
#[path = "agent_tools/records.rs"]
mod records;

use academic::*;
use files_browser::*;
use insights::*;
use mail_lookup::*;
use records::*;

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
    /// Optional text arg, omitted when empty.
    OptionalText { key: &'static str, max_len: usize },
    /// URL arg.
    Url,
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
        signature: "list_recent_notifications(limit?: number)",
        purpose: "最新のお知らせ",
        schema: ArgSchema::LimitKeyword,
    },
    ToolSpec {
        name: "search_notifications",
        category: "お知らせ・メール",
        signature: "search_notifications(keyword: string)",
        purpose: "お知らせをキーワード検索",
        schema: ArgSchema::Text {
            key: "keyword",
            max_len: 80,
        },
    },
    ToolSpec {
        name: "list_recent_mail",
        category: "お知らせ・メール",
        signature: "list_recent_mail(limit?: number)",
        purpose: "受信メール一覧",
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
        signature: "get_luna_activity_detail(title: string)",
        purpose: "タイトルでレポート/テスト/掲示/お知らせの本文・提出要件・添付を取得",
        schema: ArgSchema::Text {
            key: "title",
            max_len: 120,
        },
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
        signature: "read_downloaded_file(path: string)",
        purpose: "ダウンロード済み PDF / DOCX / TXT / MD / JSON / CSV / HTML の本文を抽出して読む",
        schema: ArgSchema::FilePath,
    },
    ToolSpec {
        name: "inspect_file",
        category: "ダウンロードファイル",
        signature: "inspect_file(path: string)",
        purpose: "read_downloaded_file の互換エイリアス",
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
        name: "open_luna_attachment",
        category: "ダウンロードファイル",
        signature: "open_luna_attachment(title: string, attachment_name?: string)",
        purpose: "Luna 詳細から添付ファイル/外部資料リンクを探して開く",
        schema: ArgSchema::TitleAttachment,
    },
    ToolSpec {
        name: "download_luna_attachment",
        category: "ダウンロードファイル",
        signature: "download_luna_attachment(title: string, attachment_name?: string)",
        purpose: "Luna 詳細から添付ファイル/資料を探してダウンロードする",
        schema: ArgSchema::TitleAttachment,
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
        name: "read_browser_page",
        category: "ブラウザ",
        signature: "read_browser_page(target?: string)",
        purpose: "ブラウザ webview の現在ページ本文を抽出して読む",
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
];

/// Check if a tool name is in the registry.
pub fn is_known_tool(name: &str) -> bool {
    TOOL_SPECS.iter().any(|s| s.name == name)
}

/// Dispatch a single tool call.  Returns a JSON value even on failure so the
/// agent can still surface the error to the user.
pub async fn dispatch(app: &tauri::AppHandle, name: &str, args: &Value) -> Value {
    if !is_known_tool(name) {
        return json!({ "error": format!("unknown tool: {}", name) });
    }
    let result: Result<Value, String> = match name {
        "list_today_classes" => list_today_classes(app).await,
        "list_week_classes" => list_week_classes(app, args).await,
        "search_courses" => search_courses(app, args).await,
        "get_course_context" => get_course_context(app, args).await,
        "list_luna_todos" => list_luna_todos(app).await,
        "list_recent_notifications" => list_recent_notifications(app, args).await,
        "search_notifications" => search_notifications(app, args).await,
        "get_course_detail" => get_course_detail(app, args).await,
        "list_recent_mail" => list_recent_mail(app, args).await,
        "read_mail" => read_mail(app, args).await,
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
        "read_downloaded_file" | "inspect_file" => read_downloaded_file(args).await,
        "write_downloaded_text_file" => write_downloaded_text_file(args).await,
        "open_downloaded_file" => open_downloaded_file(app, args).await,
        "open_luna_attachment" => open_luna_attachment(app, args).await,
        "download_luna_attachment" => download_luna_attachment(app, args).await,
        "list_browser_windows" => list_browser_windows(app).await,
        "open_browser_url" => open_browser_url(app, args).await,
        "read_browser_page" => read_browser_page(app, args).await,
        "browser_back" => browser_back(app, args).await,
        "browser_forward" => browser_forward(app, args).await,
        "browser_reload_page" => browser_reload_page(app, args).await,
        // is_known_tool guard above ensures we never reach here
        _ => unreachable!(),
    };
    match result {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

/// Static description given to the model during the planning phase.
pub fn tool_catalog_prompt() -> String {
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
            sanitize_text_arg(args, key, max_len).map(|v| json!({ key: v }))
        }
        ArgSchema::CourseCode { key } => sanitize_course_code(args, key).map(|v| json!({ key: v })),
        ArgSchema::LimitKeyword => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .min(LIST_CAP as u64);
            let keyword = sanitize_text_arg(args, "keyword", 80);
            let mut out = json!({ "limit": limit });
            if let Some(keyword) = keyword {
                out["keyword"] = Value::String(keyword);
            }
            Some(out)
        }
        ArgSchema::MailMessageId => {
            sanitize_text_arg(args, "message_id", 200).and_then(|message_id| {
                crate::mail::validate_message_id(&message_id).ok()?;
                Some(json!({ "message_id": message_id }))
            })
        }
        ArgSchema::FilePath => {
            sanitize_file_path_arg(args, "path").map(|path| json!({ "path": path }))
        }
        ArgSchema::FileWrite => {
            let path = sanitize_file_path_arg(args, "path")?;
            let content = sanitize_text_blob_arg(args, "content", 100_000)?;
            Some(json!({ "path": path, "content": content }))
        }
        ArgSchema::TitleAttachment => {
            let title = sanitize_text_arg(args, "title", 120)?;
            let attachment_name = sanitize_text_arg(args, "attachment_name", 160);
            let mut out = serde_json::Map::new();
            out.insert("title".to_string(), Value::String(title));
            if let Some(name) = attachment_name {
                out.insert("attachment_name".to_string(), Value::String(name));
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
    }
}

fn sanitize_text_arg(args: &Value, key: &str, max_len: usize) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() {
        return None;
    }
    let mut out = value.chars().take(max_len).collect::<String>();
    out = out.replace('\n', " ").replace('\r', " ");
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() {
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

/// Map simplified Chinese characters to their Japanese kanji equivalents
/// so cross-lingual course searches work.
fn normalize_cjk_char(c: char) -> char {
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
