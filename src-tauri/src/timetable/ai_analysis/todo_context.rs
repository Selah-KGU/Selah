//! Todo prompts, live-note context, and the todo analysis prompt builder.

use super::super::day_int_to_str;
use crate::commands;
use crate::db::ScheduleRawData;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const TODO_API_LIVE_NOTE_MAX_PER_COURSE: usize = 2;
const TODO_API_LIVE_NOTE_MAX_CHARS: usize = 1600;

pub(super) const TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生専属の学習コンサルタントAIです。
学生が今抱えている未提出課題・テスト・ディスカッション等のタスクと、それに紐づくコースの授業計画（シラバス）・教材・過去の授業内容・対応するliveノートを受け取り、**本当に役立つ具体的な学習支援**を行ってください。

## あなたの役割
1. **課題の本質を理解する**: 授業計画・シラバスから、その課題が「何を求めているか」「どの授業回の内容に対応するか」を特定し、学生に伝える
2. **必要な知識を整理する**: その課題に取り組むために必要な前提知識・概念・理論を、授業内容から推測して簡潔にまとめる
3. **具体的な行動手順を示す**: 「調べましょう」「頑張りましょう」のような曖昧な助言ではなく、「第N回の〇〇の概念を復習→△△の観点でアウトラインを作成→□□に注意して執筆」のように、実際に手を動かせるステップを示す
4. **時間配分を最適化する**: 3日間の計画で、各タスクの所要時間・優先度・授業スケジュール（空きコマ）を考慮した現実的な作業スケジュールを組む
5. **liveノートを根拠に説明する**: 対応するliveノートがある場合は、授業で実際に扱った概念・キーワード・例示・先生の強調点を優先的に使って、課題とのつながりを説明する
6. **学生の代わりに一部を先に作る**: 単なる助言で終わらず、その課題にすぐ使える下書き・構成案・復習チェックリスト・投稿たたき台などを直接作る

## 出力JSON形式（他のテキストは一切不要）
{
  "task_guides": [
    {
      "task_name": "課題名（提出物のタイトルをそのまま使用）",
      "course_name": "科目名",
      "deadline": "YYYY/MM/DD HH:MM",
      "urgency": "overdue|critical|soon|normal",
      "background": "この課題の文脈説明。「第N回で扱った〇〇（具体的な概念名）に関連する課題です。△△の理論/手法/知識が前提となります。教材『□□』の内容を参照すると理解が深まります。」のように、授業計画・教材タイトル・シラバスの内容を根拠にした具体的な記述を2-4文で書く。",
      "live_note_summary": "対応するliveノートがある場合のみ2〜3文。実際の授業で何を扱い、この課題とどうつながるかを具体語で説明する。無い場合は空文字。",
      "study_hints": [
        "第5回の講義スライドでXXの定義を確認する",
        "XXとYYの関係を表にまとめる",
        "序論でXXを定義し、本論でYYの事例を3つ挙げて分析する"
      ],
      "ready_to_use_label": "提纲|讨论草稿|复习清单|答案骨架",
      "ready_to_use": "学生が今すぐ使える具体的な叩き台。3〜8行程度。情報が足りない箇所は [ ] のプレースホルダで残してよいが、汎用的な励まし文は禁止。",
      "estimated_minutes": 120
    }
  ],
  "daily_plan": [
    {
      "label": "今日（M/D）|明日（M/D）|明後日（M/D）",
      "tasks": ["（task_guidesのtask_nameと完全一致）（N分・緊急）", "（同上）（N分）"],
      "free_hours": 4.0
    }
  ],
  "advice": "句点区切りの3〜5文。作業負荷の全体像と戦略的なアドバイス。"
}

## urgency判定基準
- overdue: 締切が既に過ぎている
- critical: 締切まで24時間以内
- soon: 締切まで3日以内
- normal: それ以外

## 品質基準
- backgroundは**必ず授業計画・教材・シラバス・liveノートの具体的な情報を引用**すること。汎用的な文章は禁止
- study_hintsは**その課題固有の手順**を書くこと。どの課題にも使い回せる汎用ステップは禁止
  - 良い例: 「第3回で扱ったマーケティングミックス（4P）のフレームワークを使って分析する」
  - 悪い例: 「関連資料を調べて要点をまとめる」
- live_note_summaryは、対応するliveノートがある課題のみ記入する。授業で実際に出た概念・用語・例示を使って「この課題で何を見ればよいか」を説明し、liveノートが無い課題では空文字にする
- ready_to_useは、学生がそのまま土台として使える具体物にする
  - レポート/課題 → 構成案、論点整理、冒頭段落のたたき台、見出し案
  - テスト/小テスト → 復習チェックリスト、出題されそうな論点一覧、暗記確認用の穴埋め骨子
  - ディスカッション → 投稿文のたたき台、賛成/反対の観点整理、引用すべき論点の箇条書き
- ready_to_useでは、与えられていない事実や課題要件を捏造しない。不足情報があれば [授業で扱った事例] のようなプレースホルダで残す
- 課題タイプ別の重点:
  - レポート/課題 → テーマの特定・必要な理論の整理・構成案・執筆・推敲の手順
  - テスト/小テスト → 出題範囲の特定・重要概念リストアップ・理解度チェック方法
  - ディスカッション → 議題の背景理解・多角的な視点の整理・投稿文の構成
- study_hintsの各項目に「ステップN:」「第N步:」「まず」「次に」などの序数・接続詞を付けない（UIが自動で番号を付与する）
- daily_planのlabelには日付を含める（例: 「今日（4/12）」）
- daily_plan.tasksの各文字列は「{task_guidesのtask_nameと完全一致}（N分）」の形式にする。task_nameと一致しないと詳細が表示されないため厳守
- free_hoursは時間割の授業時間を除いた学習可能時間（9:00-22:00の範囲で概算）
- タスク一覧には期限切れ（締切超過）タスクは含まれない。締切が近いタスクを優先して今日の計画に入れる
- estimated_minutesは課題の複雑さと授業レベルを考慮した現実的な見積もり
- adviceは「。」区切りで循環表示されるため、各文が独立して意味をなすようにする
- liveノートが授業計画と食い違う場合は、実際の授業進度としてliveノートを優先しつつ、不確実なら断定を避ける
- 回答はJSONのみ。マークダウンのコードブロック(```)は使わない
- 回答は指定された言語で書くこと"#;

pub(super) const LOCAL_TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生専属の学習コンサルタントAIです。
未提出タスク・授業計画・教材情報を使って、実行可能な学習支援をJSONのみで返してください。

重要:
- JSON以外の文は出力しない
- マークダウンのコードブロック（```）を使わない

出力形式（キー名を厳守）:
{
    "task_guides": [
        {
            "task_name": "課題名",
            "course_name": "科目名",
            "deadline": "YYYY/MM/DD HH:MM",
            "urgency": "overdue|critical|soon|normal",
            "background": "2〜4文の具体説明",
            "study_hints": ["具体手順", "具体手順"],
            "estimated_minutes": 120
        }
    ],
    "daily_plan": [
        {
            "label": "今日（M/D）",
            "tasks": ["task_nameと完全一致（N分）"],
            "free_hours": 4.0
        }
    ],
    "advice": "3〜5文。文は。で区切る"
}

品質ルール:
- background は授業計画/教材の具体語を入れる（抽象論のみは禁止）
- study_hints は課題固有の行動手順にする（汎用文禁止）
- daily_plan.tasks は task_guides.task_name と完全一致させる
- 24h以内のタスクを最優先にする（一覧に期限切れタスクは含まれない）
- 各フィールドは必ず型を守る（string/array/number）。nullを使わない
- 回答は指定された言語で書くこと"#;

#[derive(Debug, Clone)]
pub(super) struct RelevantLiveNote {
    matched_course_name: String,
    file_name: String,
    downloaded_at: i64,
    summary: String,
}

/// 締切超過のタスクか判定する。締切が空、または解析できない場合は false。
pub(super) fn todo_is_overdue(
    item: &crate::luna_parser::LunaTodoItem,
    now: chrono::NaiveDateTime,
) -> bool {
    let deadline = item.deadline.trim();
    if deadline.is_empty() {
        return false;
    }
    chrono::NaiveDateTime::parse_from_str(deadline, "%Y-%m-%d %H:%M")
        .map(|dl| dl < now)
        .unwrap_or(false)
}

pub(super) fn build_todo_ai_prompt(
    todos: &[crate::luna_parser::LunaTodoItem],
    raw: &ScheduleRawData,
    is_local: bool,
    live_notes: &[RelevantLiveNote],
) -> String {
    let cal_cfg = commands::load_calendar_config();
    let mut text = String::new();

    let today = chrono::Local::now();
    let today_date = today.date_naive();
    text.push_str(&format!(
        "## 今日: {} ({})\n",
        today.format("%Y年%m月%d日"),
        today.format("%A")
    ));

    let mut current_week: i32 = 4;
    if !cal_cfg.spring_start.is_empty() {
        if let Ok(spring) = chrono::NaiveDate::parse_from_str(&cal_cfg.spring_start, "%Y-%m-%d") {
            let days_since = (today_date - spring).num_days();
            if (0..150).contains(&days_since) {
                let week = (days_since / 7 + 1) as i32;
                current_week = week;
                text.push_str(&format!("春学期 第{}週目（全15週）\n", week));
            }
        }
    }
    if !cal_cfg.fall_start.is_empty() {
        if let Ok(fall) = chrono::NaiveDate::parse_from_str(&cal_cfg.fall_start, "%Y-%m-%d") {
            let days_since = (today_date - fall).num_days();
            if (0..150).contains(&days_since) {
                let week = (days_since / 7 + 1) as i32;
                current_week = week;
                text.push_str(&format!("秋学期 第{}週目（全15週）\n", week));
            }
        }
    }

    text.push_str("\n## 未提出タスク一覧\n");
    // 期限切れ（締切超過）タスクは分析データに含めない。
    let now_naive = today.naive_local();
    let pending: Vec<&crate::luna_parser::LunaTodoItem> = todos
        .iter()
        .filter(|t| !t.status.contains("提出済"))
        .filter(|t| !todo_is_overdue(t, now_naive))
        .collect();

    for item in &pending {
        let urgency_hint = if !item.deadline.is_empty() {
            if let Ok(dl) = chrono::NaiveDateTime::parse_from_str(&item.deadline, "%Y-%m-%d %H:%M")
            {
                let diff = dl.signed_duration_since(today.naive_local());
                let hours = diff.num_hours();
                if hours < 0 {
                    "【期限超過】"
                } else if hours < 24 {
                    "【24h以内】"
                } else if hours < 72 {
                    "【3日以内】"
                } else {
                    ""
                }
            } else {
                ""
            }
        } else {
            ""
        };

        text.push_str(&format!(
            "- {}{} [{}] | 科目: {} | 締切: {} | 状態: {}\n",
            urgency_hint,
            item.content_name,
            item.content_type,
            item.course_name,
            if item.deadline.is_empty() {
                "未設定"
            } else {
                &item.deadline
            },
            item.status,
        ));
        if !item.feedback.is_empty() {
            text.push_str(&format!("  教員フィードバック: {}\n", item.feedback));
        }
    }

    if !is_local && !live_notes.is_empty() {
        text.push_str("\n## 対応するliveノート（実際の授業メモ）\n");
        text.push_str("以下は当該コースの最近のliveノートです。課題の背景や授業進度の判断では、シラバスだけでなくこのliveノートの具体語・扱った概念・先生の強調点を優先的に参照してください。liveノートがある課題では、backgroundとlive_note_summaryの両方に反映してください。\n");
        for note in live_notes {
            text.push_str(&format!(
                "### {} | {}\n{}\n",
                note.matched_course_name, note.file_name, note.summary
            ));
        }
    }

    if !raw.kgc_entries_current.is_empty() {
        text.push_str(&format!("\n## 今週の時間割 ({})\n", raw.current_week_label));
        for e in &raw.kgc_entries_current {
            let status = if e.is_cancelled {
                " [休講]"
            } else if e.is_makeup {
                " [補講]"
            } else {
                ""
            };
            text.push_str(&format!(
                "- {}曜{}限: {}{}\n",
                day_int_to_str(e.day),
                e.period,
                e.name,
                status
            ));
        }
    }

    let pending_course_names: HashSet<&str> =
        pending.iter().map(|t| t.course_name.as_str()).collect();

    if !is_local && !raw.luna_activities.is_empty() {
        let luna_id_to_name: HashMap<&str, &str> = raw
            .luna_courses
            .iter()
            .map(|c| (c.luna_id.as_str(), c.name.as_str()))
            .collect();

        let mut grouped: HashMap<&str, Vec<&crate::db::LunaActivityRow>> = Default::default();
        for a in &raw.luna_activities {
            grouped.entry(a.luna_id.as_str()).or_default().push(a);
        }

        text.push_str("\n## コース別の活動詳細（教材・課題・テスト・ディスカッション）\n");
        for (id, items) in &grouped {
            let name = luna_id_to_name.get(id).unwrap_or(id);
            if !pending_course_names.contains(*name) {
                continue;
            }

            text.push_str(&format!("### {}\n", name));

            let materials: Vec<_> = items
                .iter()
                .filter(|a| a.activity_type == "material")
                .collect();
            let reports: Vec<_> = items
                .iter()
                .filter(|a| a.activity_type == "report")
                .collect();
            let exams: Vec<_> = items.iter().filter(|a| a.activity_type == "exam").collect();
            let discussions: Vec<_> = items
                .iter()
                .filter(|a| a.activity_type == "discussion")
                .collect();

            if !materials.is_empty() {
                text.push_str("  教材:\n");
                for a in &materials {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() {
                        text.push_str(&format!(" ({})", a.period));
                    }
                    text.push('\n');
                }
            }
            if !reports.is_empty() {
                text.push_str("  課題:\n");
                for a in &reports {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() {
                        text.push_str(&format!(" (期限: {})", a.period));
                    }
                    if !a.status.is_empty() {
                        text.push_str(&format!(" [{}]", a.status));
                    }
                    text.push('\n');
                }
            }
            if !exams.is_empty() {
                text.push_str("  テスト:\n");
                for a in &exams {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() {
                        text.push_str(&format!(" (期間: {})", a.period));
                    }
                    if !a.status.is_empty() {
                        text.push_str(&format!(" [{}]", a.status));
                    }
                    text.push('\n');
                }
            }
            if !discussions.is_empty() {
                text.push_str("  ディスカッション:\n");
                for a in &discussions {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() {
                        text.push_str(&format!(" (期間: {})", a.period));
                    }
                    if !a.status.is_empty() {
                        text.push_str(&format!(" [{}]", a.status));
                    }
                    text.push('\n');
                }
            }
        }
    }

    if !is_local && !raw.session_plans.is_empty() {
        let code_to_name: HashMap<&str, &str> = raw
            .kgc_entries_current
            .iter()
            .chain(raw.kgc_entries_next.iter())
            .map(|e| (e.kgc_code.as_str(), e.name.as_str()))
            .collect();

        let mut any_plan = false;
        for (code, plans) in &raw.session_plans {
            let cname = code_to_name.get(code.as_str()).copied().unwrap_or("");
            if !pending_course_names.contains(cname) {
                continue;
            }
            if !any_plan {
                text.push_str("\n## 関連コースの授業計画\n");
                any_plan = true;
            }
            text.push_str(&format!("### {} [{}]\n", cname, code));
            for p in plans {
                if p.session_num <= current_week + 3 {
                    let marker = if p.session_num == current_week {
                        " ← 今週"
                    } else if p.session_num == current_week - 1 {
                        " ← 先週"
                    } else {
                        ""
                    };
                    let mut line = format!("  第{}回:", p.session_num);
                    if !p.topic.is_empty() {
                        line.push_str(&format!(" {}", p.topic));
                    }
                    if !p.delivery_mode.is_empty() {
                        line.push_str(&format!(" [{}]", p.delivery_mode));
                    }
                    if !p.study_outside.is_empty() {
                        line.push_str(&format!(" | 予復習: {}", p.study_outside));
                    }
                    line.push_str(marker);
                    line.push('\n');
                    text.push_str(&line);
                }
            }
        }
    }

    if !is_local && !raw.kgc_course_details.is_empty() {
        let code_to_name: HashMap<&str, &str> = raw
            .kgc_entries_current
            .iter()
            .chain(raw.kgc_entries_next.iter())
            .map(|e| (e.kgc_code.as_str(), e.name.as_str()))
            .collect();

        let mut any_detail = false;
        for detail in &raw.kgc_course_details {
            let cname = code_to_name
                .get(detail.kgc_code.as_str())
                .copied()
                .unwrap_or("");
            if !pending_course_names.contains(cname) {
                continue;
            }
            if detail.fields.is_empty() {
                continue;
            }
            if !any_detail {
                text.push_str("\n## 関連コースのシラバス詳細\n");
                any_detail = true;
            }
            text.push_str(&format!("### {} [{}]\n", cname, detail.kgc_code));
            if !detail.delivery_mode.is_empty() {
                text.push_str(&format!("  授業形態: {}\n", detail.delivery_mode));
            }
            for (label, value) in &detail.fields {
                if !value.is_empty() {
                    text.push_str(&format!("  {}: {}\n", label, value));
                }
            }
        }
    }

    text
}

pub(super) fn collect_relevant_live_notes(
    todos: &[crate::luna_parser::LunaTodoItem],
) -> Vec<RelevantLiveNote> {
    // 期限切れタスクは分析対象外なので、その科目の live ノートも収集しない。
    let now_naive = chrono::Local::now().naive_local();
    let mut course_names: Vec<String> = todos
        .iter()
        .filter(|t| !t.status.contains("提出済"))
        .filter(|t| !todo_is_overdue(t, now_naive))
        .map(|t| t.course_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    course_names.sort();
    course_names.dedup();
    if course_names.is_empty() {
        return Vec::new();
    }

    let records = match commands::list_downloads_snapshot() {
        Ok(records) => records,
        Err(error) => {
            log::warn!("Reading LIVE download history for TODO context failed: {error}");
            return Vec::new();
        }
    };
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    for course_name in &course_names {
        let course_key = normalize_course_match_key(course_name);
        if course_key.is_empty() {
            continue;
        }

        let mut matched: Vec<&crate::commands::DownloadRecord> = records
            .iter()
            .filter(|record| {
                record.file_exists
                    && (record.source == "live" || record.filename.contains("_live"))
                    && record_matches_course(record, &course_key)
            })
            .collect();
        matched.sort_by(|a, b| b.downloaded_at.cmp(&a.downloaded_at));

        for record in matched.into_iter().take(TODO_API_LIVE_NOTE_MAX_PER_COURSE) {
            if !seen_paths.insert(record.path.clone()) {
                continue;
            }
            if let Some(note) = build_relevant_live_note(course_name, record) {
                out.push(note);
            }
        }
    }

    out.sort_by(|a, b| b.downloaded_at.cmp(&a.downloaded_at));
    out
}

fn build_relevant_live_note(
    matched_course_name: &str,
    record: &crate::commands::DownloadRecord,
) -> Option<RelevantLiveNote> {
    let path = Path::new(&record.path);
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > 2_000_000 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let markdown = String::from_utf8_lossy(&bytes).to_string();
    let summary = compact_live_note_markdown(&markdown, TODO_API_LIVE_NOTE_MAX_CHARS);
    if summary.is_empty() {
        return None;
    }

    Some(RelevantLiveNote {
        matched_course_name: matched_course_name.to_string(),
        file_name: record.filename.clone(),
        downloaded_at: record.downloaded_at,
        summary,
    })
}

pub(super) fn compact_live_note_markdown(markdown: &str, max_chars: usize) -> String {
    let head = markdown.split("\n## 全文転写").next().unwrap_or(markdown);
    let mut lines = Vec::new();

    for line in head.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(
            trimmed,
            "- 開始:" | "- 終了:" | "- 授業コード:" | "- 教員:" | "- 教室:" | "- 時間帯:"
        ) {
            continue;
        }
        if trimmed.starts_with("- 開始:")
            || trimmed.starts_with("- 終了:")
            || trimmed.starts_with("- 授業コード:")
            || trimmed.starts_with("- 教員:")
            || trimmed.starts_with("- 教室:")
            || trimmed.starts_with("- 時間帯:")
        {
            continue;
        }

        let cleaned = trimmed
            .trim_start_matches('#')
            .trim()
            .trim_start_matches('-')
            .trim();
        if cleaned.is_empty() || cleaned == "区間ごとの要約" || cleaned == "全文転写" {
            continue;
        }
        lines.push(cleaned.to_string());
    }

    truncate_chars(&lines.join("\n"), max_chars)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{}…", truncated)
}

fn normalize_course_match_key(name: &str) -> String {
    commands::simplify_course_name(name)
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn record_matches_course(record: &crate::commands::DownloadRecord, course_key: &str) -> bool {
    if course_key.is_empty() {
        return false;
    }

    let mut candidates = vec![record.course_name.clone(), record.filename.clone()];
    if let Some(parent) = Path::new(&record.path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|name| name.to_str())
    {
        candidates.push(parent.to_string());
    }

    candidates.into_iter().any(|candidate| {
        let key = normalize_course_match_key(&candidate);
        !key.is_empty()
            && (key == course_key
                || (course_key.len() >= 6 && key.contains(course_key))
                || (key.len() >= 6 && course_key.contains(&key)))
    })
}

/// 旧バージョンが保存した内部フィールドを取り除いてから結果を返す。
pub(super) fn strip_todo_internal_fields(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("_cache_fingerprint");
    }
    value
}
