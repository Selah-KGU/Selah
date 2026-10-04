//! Schedule prompts and parsing of the schedule analysis response.

use super::super::day_int_to_str;
use super::*;
use crate::commands;
use crate::db::{AiScheduleItem, AiScheduleResult, ScheduleRawData};
use serde::Deserialize;

pub(super) const SCHEDULE_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生向けスケジュール分析AIです。
提供された時間割データ（KGC + Luna）を分析し、構造化されたJSON形式で2週間分の日程表を生成してください。

出力は必ず以下のJSON形式で返してください（他のテキストは不要）:
{
  "current_week": [
    {
      "day": 1,
      "period": 1,
      "course_name": "科目名",
      "delivery_mode": "対面/オンライン/同時双方向/オンデマンド",
      "room": "教室",
      "teacher": "教員名",
      "session_topic": "第N回: 今回の授業内容",
      "is_cancelled": false,
      "notifications": ["新しいお知らせの概要"],
      "assignments": ["未提出課題: 課題名 (締切: YYYY-MM-DD)"],
      "exams": ["テスト名 (期間: YYYY-MM-DD ~ YYYY-MM-DD, 状態: 未回答)"]
    }
  ],
  "next_week": [...],
  "weekly_summary": "今週の提案1。今週の提案2。今週の提案3",
  "cross_week_insights": "来週に向けた提案1。来週に向けた提案2"
}

表示上の注意:
- weekly_summaryとcross_week_insightsは画面上部で句単位で循環表示される。「。」で区切る。
- これらは単なる要約ではなく、学生への具体的なアドバイスや行動提案として書くこと。
  良い例: 「木曜の○○は課題締切前日、早めに着手しましょう」「来週○曜は対面とオンデマンドが混在、時間管理に注意」
  悪い例: 「今週は5コマある」「来週も通常通り」（情報の羅列や当たり前の内容は避ける）
- weekly_summaryは4〜6文程度。休講・締切間近・テスト・形態変更・予習が必要な科目など、実際に行動すべき内容を具体的に。
- cross_week_insightsは2〜4文程度。来週の対策に絞る。
- session_topicはセル内に表示されるため簡潔に（例: 「第5回: データ構造の基礎」）。
- notifications/assignments/examsの各項目も1行でセル内に表示されるため、要点のみ記載。

ルール:
- day: 1=月曜 2=火曜 3=水曜 4=木曜 5=金曜 6=土曜
- period: 1-7限
- 今週/来週ラベル（例: "2026年04月07日～2026年04月11日"）は該当週の月曜～金曜の日付範囲を示す。授業計画のtopicにある日付がこの範囲に含まれるかで今週/来週の対応回を判定する

## Step 1: 授業計画から日付とdelivery_modeを読み取る

各科目の授業計画にはheader（セッションヘッダー）、column（表の中間列データ）、topic（授業内容）が分離して提供される。

**headerから読み取れる情報:**
- 週タイプ: 【スタートアップウィーク】【コアウィークス①】【フレックスアワーズ①】等
- 授業形態: 「・オンライン授業（オンデマンド型）・」「対面授業／」「同時双方向型」等
- 授業時間: 《60分》《90分》等

**columnから読み取れる情報:**
- 授業形態の短縮表記: 「対面」「オンデマンド」「オンライン」等（表の独立した列データ）

**topicから読み取れる情報:**
- 日付: 「4/16」「5/14」等
- 授業内容の説明文

**delivery_modeの判定順序:**
1. headerに「オンデマンド」「同時双方向」「オンライン」「対面授業」がある → そのまま使う
2. columnが存在する → columnの値を使う（「対面」「オンデマンド」等、表の独立した列なので信頼性が高い）
3. headerにもcolumnにも情報がない → KGC課程詳細の授業形態を参照
4. どこにも明示的な情報がない → 「対面」と推定する（大学の授業は対面がデフォルト）
- **「要確認」はdelivery_modeには使わない**。必ず上記の優先順位で判定すること。
- **topicの授業内容説明文の中に含まれる「対面」「対面授業XX回中」等の単語は出席ルール説明であり、delivery_modeではない**

## Step 2: 授業回数の特定

### 2a. 日付で確定できる回を先に埋める
topicに日付がある回を、今週・来週のカレンダーと照合して確定する。

### 2b. 日付のない回は前後関係から推論する
- 2つの日付付き回の間に日付のない回がある場合:
  例: 第2回=4/16、第4回=4/23 → 第3回は4/16～4/22の期間（フレックスアワーズ等のオンデマンド回の可能性が高い）
  → この回は対面で出席する必要がないので、出力のis_cancelledをfalseにしつつ、session_topicに「(オンデマンド視聴期間)」と補足
- 毎週ある科目で1週間に2コマ消化されるパターン（例: 9日→16日に2回分進む）場合も同様に推論

### 2c. 全回に日付がなく、学期開始日が提供されている場合
ユーザデータに「現在は春/秋学期 第N週目」と記載されていればその週数を使う。
記載がなければ学期開始日と曜日から逆算して「第N週目」を計算し、対応する回を推定する。
ただし祝日・長期休暇のスキップがあるため、推定であることをsession_topicに「(推定)」と付記する。

### 2d. 確定できない場合
学期開始日からの週数推定を優先し、session_topicに「(推定)」と付記する。
推定すらできない場合のみ「(要確認)」を使う。ただし空白や省略はしない——必ず最も可能性の高い回を記載する。

## Step 3: データ統合
- KGCデータとLunaデータを照合し、同じコマの情報を統合（科目名/曜日/時限で照合）
- Luna活動詳細の個別アイテム（課題名、締切、状態）をassignments/exams/notificationsに反映
- 休講の授業は is_cancelled: true
- 通知・課題・テストが無い場合は空配列
- weekly_summaryは「今週やるべきこと・気をつけること」を具体的に提案
- cross_week_insightsは「来週に向けて今週中にやっておくべきこと」を提案
- 回答はJSONのみ、マークダウンのコードブロックは不要
- 回答は指定された言語で書くこと"#;

pub(super) const LOCAL_SCHEDULE_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生向けスケジュール分析AIです。
提供データ（KGC + Luna）から2週間分の時間割を作成し、JSONのみで返してください。

重要:
- JSON以外の文章は出力しない
- マークダウンのコードブロック（```）を使わない

出力形式（このキー名を厳守）:
{
    "current_week": [
        {
            "day": 1,
            "period": 1,
            "course_name": "科目名",
            "delivery_mode": "対面/オンライン/同時双方向/オンデマンド",
            "room": "教室",
            "teacher": "教員名",
            "session_topic": "第N回: 内容",
            "is_cancelled": false,
            "notifications": ["項目"],
            "assignments": ["項目"],
            "exams": ["項目"]
        }
    ],
    "next_week": [同じ形式],
    "weekly_summary": "3〜5文。文は。で区切る",
    "cross_week_insights": "2〜3文。文は。で区切る"
}

品質ルール（重要な精髄）:
- day は 1=月, 2=火, 3=水, 4=木, 5=金, 6=土
- period は 1〜7
- 休講は is_cancelled=true
- notifications/assignments/exams が無ければ []
- 文字列フィールドは必ず文字列で出力（objectやarrayを入れない）
- 各セルに入る文は短くする
- delivery_mode 判定優先順: header > column > KGC課程詳細 > 対面(デフォルト)
- topic内の日付（例: 4/16）を今週/来週ラベルの範囲と照合して session_topic の回次を決める
- 日付が欠ける回は前後の回次から補完し、必要なら「(推定)」を付ける
- weekly_summary は情報羅列ではなく、今週の具体的行動提案を4〜6文で書く（締切・休講・予習・形態混在への対処を優先）
- cross_week_insights は来週に向けた準備行動を2〜4文で書く
- assignments/exams は課題名・テスト名と期間/締切を優先して短く記載
- JSONの各配列/文字列は必ず型を守る（nullは使わない）
- 回答は指定された言語で書くこと"#;

pub(super) fn build_ai_schedule_prompt(raw: &ScheduleRawData, is_local: bool) -> String {
    let cal_cfg = commands::load_calendar_config();
    let mut text = String::new();

    let today = chrono::Local::now();
    let today_str = today.format("%Y-%m-%d (%A)").to_string();
    text.push_str(&format!("## 今日の日付: {}\n", today_str));

    let current_date = today.date_naive();
    {
        let mut semester_lines: Vec<String> = Vec::new();
        if !cal_cfg.spring_start.is_empty() {
            if let Ok(spring) = chrono::NaiveDate::parse_from_str(&cal_cfg.spring_start, "%Y-%m-%d")
            {
                semester_lines.push(format!("- 春学期開始: {}", cal_cfg.spring_start));
                let days_since = (current_date - spring).num_days();
                if (0..150).contains(&days_since) {
                    semester_lines.push(format!("- ★ 現在は春学期 第{}週目", days_since / 7 + 1));
                }
            }
        }
        if !cal_cfg.fall_start.is_empty() {
            if let Ok(fall) = chrono::NaiveDate::parse_from_str(&cal_cfg.fall_start, "%Y-%m-%d") {
                semester_lines.push(format!("- 秋学期開始: {}", cal_cfg.fall_start));
                let days_since = (current_date - fall).num_days();
                if (0..150).contains(&days_since) {
                    semester_lines.push(format!("- ★ 現在は秋学期 第{}週目", days_since / 7 + 1));
                }
            }
        }
        if !semester_lines.is_empty() {
            text.push_str("\n## 学期情報\n");
            for line in &semester_lines {
                text.push_str(line);
                text.push('\n');
            }
        }
    }

    text.push('\n');

    text.push_str(&format!("## 今週: {}\n", raw.current_week_label));
    text.push_str("### KGC時間割（今週）\n");
    for e in &raw.kgc_entries_current {
        if is_local {
            let mut flags = String::new();
            if e.is_cancelled {
                flags.push_str(" [休講]");
            }
            if e.is_makeup {
                flags.push_str(" [補講]");
            }
            if e.is_room_changed {
                flags.push_str(" [変更]");
            }
            text.push_str(&format!(
                "- {}曜{}限: {} [{}] 教室:{}{}\n",
                day_int_to_str(e.day),
                e.period,
                e.name,
                e.kgc_code,
                e.room,
                flags
            ));
        } else {
            text.push_str(&format!(
                "- {}曜{}限: {} [{}] 教室:{} 休講:{} 補講:{} 変更:{}\n",
                day_int_to_str(e.day),
                e.period,
                e.name,
                e.kgc_code,
                e.room,
                e.is_cancelled,
                e.is_makeup,
                e.is_room_changed
            ));
        }
    }

    text.push_str(&format!("\n## 来週: {}\n", raw.next_week_label));
    text.push_str("### KGC時間割（来週）\n");
    for e in &raw.kgc_entries_next {
        if is_local {
            let mut flags = String::new();
            if e.is_cancelled {
                flags.push_str(" [休講]");
            }
            if e.is_makeup {
                flags.push_str(" [補講]");
            }
            if e.is_room_changed {
                flags.push_str(" [変更]");
            }
            text.push_str(&format!(
                "- {}曜{}限: {} [{}] 教室:{}{}\n",
                day_int_to_str(e.day),
                e.period,
                e.name,
                e.kgc_code,
                e.room,
                flags
            ));
        } else {
            text.push_str(&format!(
                "- {}曜{}限: {} [{}] 教室:{} 休講:{} 補講:{} 変更:{}\n",
                day_int_to_str(e.day),
                e.period,
                e.name,
                e.kgc_code,
                e.room,
                e.is_cancelled,
                e.is_makeup,
                e.is_room_changed
            ));
        }
    }

    if !raw.luna_courses.is_empty() {
        text.push_str("\n### Luna登録コース\n");
        for c in &raw.luna_courses {
            text.push_str(&format!(
                "- {}曜{}限: {} [luna_id:{}] 教員:{}\n",
                day_int_to_str(c.day),
                c.period,
                c.name,
                c.luna_id,
                c.teacher
            ));
        }
    }

    let code_to_name: std::collections::HashMap<&str, &str> = raw
        .kgc_entries_current
        .iter()
        .chain(raw.kgc_entries_next.iter())
        .map(|e| (e.kgc_code.as_str(), e.name.as_str()))
        .collect();

    let semester_week: i32 = if is_local {
        let mut w: i32 = 4;
        if !cal_cfg.spring_start.is_empty() {
            if let Ok(spring) = chrono::NaiveDate::parse_from_str(&cal_cfg.spring_start, "%Y-%m-%d")
            {
                let d = (current_date - spring).num_days();
                if (0..150).contains(&d) {
                    w = (d / 7 + 1) as i32;
                }
            }
        }
        if !cal_cfg.fall_start.is_empty() {
            if let Ok(fall) = chrono::NaiveDate::parse_from_str(&cal_cfg.fall_start, "%Y-%m-%d") {
                let d = (current_date - fall).num_days();
                if (0..150).contains(&d) {
                    w = (d / 7 + 1) as i32;
                }
            }
        }
        w
    } else {
        0
    };

    if !raw.session_plans.is_empty() {
        text.push_str("\n### 授業計画\n");
        for (code, plans) in &raw.session_plans {
            let course_label = code_to_name
                .get(code.as_str())
                .map(|n| format!("{} [{}]", n, code))
                .unwrap_or_else(|| code.clone());
            text.push_str(&format!("#### {}\n", course_label));
            for p in plans {
                if is_local
                    && (p.session_num < semester_week - 2 || p.session_num > semester_week + 2)
                {
                    continue;
                }
                let mut line = format!("  第{}回:", p.session_num);
                if !p.th_header.is_empty() {
                    line.push_str(&format!(" [header: {}]", p.th_header));
                }
                if !p.delivery_mode.is_empty() {
                    line.push_str(&format!(" [column: {}]", p.delivery_mode));
                }
                if !p.topic.is_empty() {
                    line.push_str(&format!(" {}", p.topic));
                }
                if !is_local && !p.study_outside.is_empty() {
                    line.push_str(&format!(" (予習: {})", p.study_outside));
                }
                line.push('\n');
                text.push_str(&line);
            }
        }
    }

    if !is_local && !raw.kgc_course_details.is_empty() {
        text.push_str("\n### KGC課程詳細情報\n");
        let important_labels = [
            "授業形態",
            "授業方法",
            "授業スタイル",
            "授業の進め方",
            "備考",
            "注意事項",
        ];
        for d in &raw.kgc_course_details {
            let course_label = code_to_name
                .get(d.kgc_code.as_str())
                .map(|n| format!("{} [{}]", n, d.kgc_code))
                .unwrap_or_else(|| d.kgc_code.clone());
            let relevant: Vec<_> = d
                .fields
                .iter()
                .filter(|(label, value)| {
                    !value.is_empty() && important_labels.iter().any(|k| label.contains(k))
                })
                .collect();
            if relevant.is_empty() && d.delivery_mode.is_empty() {
                continue;
            }
            text.push_str(&format!("#### {}\n", course_label));
            if !d.delivery_mode.is_empty() {
                text.push_str(&format!("  授業形態: {}\n", d.delivery_mode));
            }
            for (label, value) in &relevant {
                let truncated = if value.chars().count() > 300 {
                    let s: String = value.chars().take(300).collect();
                    format!("{}...", s)
                } else {
                    value.clone()
                };
                text.push_str(&format!("  {}: {}\n", label, truncated));
            }
        }
    }

    if !is_local && !raw.luna_counts.is_empty() {
        text.push_str("\n### Luna活動サマリー\n");
        let luna_id_to_name: std::collections::HashMap<&str, &str> = raw
            .luna_courses
            .iter()
            .map(|c| (c.luna_id.as_str(), c.name.as_str()))
            .collect();
        for (id, c) in &raw.luna_counts {
            let fallback = id.as_str();
            let name = luna_id_to_name.get(id.as_str()).unwrap_or(&fallback);
            text.push_str(&format!(
                "- {} [{}]: お知らせ{}(新{}), 未提出課題{}, テスト{}, ディスカッション{}\n",
                name, id, c.announcements, c.new_announcements, c.reports, c.exams, c.discussions
            ));
        }
    }

    if !raw.luna_activities.is_empty() {
        text.push_str("\n### Luna活動詳細\n");
        let luna_id_to_name: std::collections::HashMap<&str, &str> = raw
            .luna_courses
            .iter()
            .map(|c| (c.luna_id.as_str(), c.name.as_str()))
            .collect();

        let mut grouped: std::collections::HashMap<&str, Vec<&crate::db::LunaActivityRow>> =
            Default::default();
        for a in &raw.luna_activities {
            grouped.entry(a.luna_id.as_str()).or_default().push(a);
        }

        for (id, items) in &grouped {
            let name = luna_id_to_name.get(id).unwrap_or(id);
            text.push_str(&format!("#### {} [{}]\n", name, id));
            for a in items {
                if is_local && !matches!(a.activity_type.as_str(), "report" | "exam") {
                    continue;
                }
                let type_label = match a.activity_type.as_str() {
                    "announcement" => "お知らせ",
                    "report" => "課題",
                    "exam" => "テスト",
                    "discussion" => "ディスカッション",
                    "material" => "教材",
                    _ => &a.activity_type,
                };
                let mut line = format!("  [{}] {}", type_label, a.title);
                if !a.period.is_empty() {
                    line.push_str(&format!(" (期間: {})", a.period));
                }
                if !a.status.is_empty() {
                    line.push_str(&format!(" {{状態: {}}}", a.status));
                }
                line.push('\n');
                text.push_str(&line);
            }
        }
    }

    text
}

pub(super) fn parse_ai_schedule_response(
    response: &str,
    current_week_label: &str,
    next_week_label: &str,
    is_local: bool,
) -> Result<AiScheduleResult, String> {
    let json_str = if is_local {
        extract_json_from_local_response(response)?
    } else {
        let sanitized = sanitize_ai_response_text(response);
        if sanitized.is_empty() {
            return Err("AI応答が空です。".into());
        }
        extract_json_from_response(&sanitized).to_string()
    };

    #[derive(Deserialize)]
    #[serde(default)]
    #[derive(Default)]
    struct AiResponse {
        current_week: Vec<AiScheduleItem>,
        next_week: Vec<AiScheduleItem>,
        weekly_summary: Option<String>,
        cross_week_insights: Option<String>,
    }

    let raw_value: serde_json::Value = serde_json::from_str(&json_str)
        .or_else(|_| {
            log::warn!("ai schedule: initial JSON parse failed, attempting truncation repair");
            let repaired = repair_truncated_json(&json_str);
            serde_json::from_str::<serde_json::Value>(&repaired)
        })
        .map_err(|e| {
            format!(
                "AI応答のJSON解析に失敗: {} — 応答: {}",
                e,
                safe_preview(&json_str, 200)
            )
        })?;

    let normalized = normalize_ai_schedule_json(raw_value);

    let parsed: AiResponse = serde_json::from_value(normalized).map_err(|e| {
        format!(
            "AI応答のJSON解析に失敗: {} — 応答: {}",
            e,
            safe_preview(&json_str, 200)
        )
    })?;

    if parsed.next_week.is_empty() && !next_week_label.is_empty() {
        log::warn!("ai schedule: next_week is empty — AI response may have been truncated");
    }

    Ok(AiScheduleResult {
        current_week_label: current_week_label.to_string(),
        next_week_label: next_week_label.to_string(),
        current_week: parsed.current_week,
        next_week: parsed.next_week,
        weekly_summary: parsed.weekly_summary.unwrap_or_default(),
        cross_week_insights: parsed.cross_week_insights.unwrap_or_default(),
    })
}
