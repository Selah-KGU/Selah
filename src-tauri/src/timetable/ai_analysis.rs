use super::{day_int_to_str, safe_preview};
use crate::ai;
use crate::commands;
use crate::db::{AiScheduleItem, AiScheduleResult, Database, ScheduleRawData};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use tauri::State;

const AI_CACHE_MAX_AGE: i64 = 12 * 3600; // 12 hours

const TODO_AI_CACHE_KEY: &str = "ai_todo_analysis";
const TODO_AI_CACHE_MAX_AGE: i64 = 6 * 3600; // 6 hours
const TODO_API_LIVE_NOTE_MAX_PER_COURSE: usize = 2;
const TODO_API_LIVE_NOTE_MAX_CHARS: usize = 1600;

const TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生専属の学習コンサルタントAIです。
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
- 期限切れタスクは最優先で今日の計画に入れる
- estimated_minutesは課題の複雑さと授業レベルを考慮した現実的な見積もり
- adviceは「。」区切りで循環表示されるため、各文が独立して意味をなすようにする
- liveノートが授業計画と食い違う場合は、実際の授業進度としてliveノートを優先しつつ、不確実なら断定を避ける
- 回答はJSONのみ。マークダウンのコードブロック(```)は使わない
- 回答は指定された言語で書くこと"#;

const LOCAL_TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生専属の学習コンサルタントAIです。
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
- 期限超過・24h以内のタスクを最優先にする
- 各フィールドは必ず型を守る（string/array/number）。nullを使わない
- 回答は指定された言語で書くこと"#;

#[derive(Debug, Clone)]
struct RelevantLiveNote {
    matched_course_name: String,
    file_name: String,
    path: String,
    downloaded_at: i64,
    summary: String,
}

const SCHEDULE_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生向けスケジュール分析AIです。
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

const LOCAL_SCHEDULE_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生向けスケジュール分析AIです。
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

#[tauri::command]
pub async fn ai_generate_schedule(
    db: State<'_, Database>,
    current_week_label: String,
    next_week_label: String,
    force: bool,
) -> Result<AiScheduleResult, String> {
    if !force {
        if let Some((cached, _)) = load_ai_cache_inner(&db)? {
            if cached.current_week_label == current_week_label {
                return Ok(cached);
            }
        }
    }

    let raw = db.build_raw_data(&current_week_label, &next_week_label, Vec::new())?;
    let config = ai::load_ai_config();

    let is_local = config.provider == "local";
    let prompt = build_ai_schedule_prompt(&raw, is_local);
    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\n重要: 所有文本字段用中文（简体字）写。科目名・日付保持原数据不变。",
        "\n\nIMPORTANT: Write all text fields in English. Keep course names and dates as-is.",
        "\n\n중요: 모든 텍스트 필드를 한국어로 작성. 과목명・날짜는 원본 그대로.",
    );
    log::info!(
        "ai_generate_schedule: calling AI with {} chars prompt (local={})",
        prompt.len(),
        is_local
    );
    if !is_local {
        log::debug!("ai_generate_schedule: full prompt:\n{}", prompt);
    }
    let base_system_prompt = if config.provider == "local" {
        LOCAL_SCHEDULE_SYSTEM_PROMPT
    } else {
        SCHEDULE_SYSTEM_PROMPT
    };
    let sys = if lang_hint.is_empty() {
        base_system_prompt.to_string()
    } else {
        format!("{}{}", base_system_prompt, lang_hint)
    };
    let messages = vec![
        ai::ChatMessage {
            role: "system".into(),
            content: sys,
            images: Vec::new(),
        },
        ai::ChatMessage {
            role: "user".into(),
            content: prompt,
            images: Vec::new(),
        },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!(
        "ai_generate_schedule: got response ({} chars)",
        response.len()
    );
    if !is_local {
        log::debug!(
            "ai_generate_schedule: response preview: {}",
            safe_preview(&response, 500)
        );
    }
    let result =
        parse_ai_schedule_response(&response, &current_week_label, &next_week_label, is_local)?;
    log::info!(
        "ai_generate_schedule: parsed OK — current_week={} items, next_week={} items",
        result.current_week.len(),
        result.next_week.len()
    );

    db.save_ai_schedule_cache(&result)?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_analyze_todo(
    db: State<'_, Database>,
    force: bool,
) -> Result<serde_json::Value, String> {
    let config = ai::load_ai_config();

    let (todo_items, todo_cache_ts): (Vec<crate::luna_parser::LunaTodoItem>, i64) = db
        .get_data_cache("luna_todo")
        .ok()
        .flatten()
        .and_then(|(json, ts)| serde_json::from_str(&json).ok().map(|items| (items, ts)))
        .unwrap_or_default();

    if todo_items.is_empty() {
        return Err("TODO項目がありません。先にTODOリストを読み込んでください。".into());
    }

    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let raw = db.build_raw_data(&snap.current_week_label, &snap.next_week_label, Vec::new())?;
    let is_local = config.provider == "local";
    let live_notes = if is_local {
        Vec::new()
    } else {
        collect_relevant_live_notes(&todo_items)
    };
    let cache_fingerprint =
        build_todo_cache_fingerprint(&todo_items, todo_cache_ts, snap.updated_at, &live_notes);

    if !force {
        if let Ok(Some((json, ts))) = db.get_data_cache(TODO_AI_CACHE_KEY) {
            let now = crate::db::epoch_secs();
            if now - ts < TODO_AI_CACHE_MAX_AGE {
                if let Ok(cached) = serde_json::from_str::<serde_json::Value>(&json) {
                    if extract_todo_cache_fingerprint(&cached) == Some(cache_fingerprint.as_str()) {
                        log::info!(
                            "ai_analyze_todo: returning cached result (age={}s, fingerprint matched)",
                            now - ts
                        );
                        return Ok(strip_todo_internal_fields(cached));
                    }
                }
            }
        }
    }

    let prompt = build_todo_ai_prompt(&todo_items, &raw, is_local, &live_notes);
    log::info!(
        "ai_analyze_todo: calling AI with {} chars prompt, {} todo items (local={})",
        prompt.len(),
        todo_items.len(),
        is_local
    );
    if !is_local {
        log::debug!("ai_analyze_todo: full prompt:\n{}", prompt);
    }

    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\n重要: background, live_note_summary, study_hints, ready_to_use_label, ready_to_use, advice, daily_plan.label, daily_plan.tasks 等所有文本用中文（简体字）写。task_name・course_name・deadline保持原数据不变。",
        "\n\nIMPORTANT: Write background, live_note_summary, study_hints, ready_to_use_label, ready_to_use, advice, daily_plan.label, daily_plan.tasks in English. Keep task_name, course_name, deadline as-is from source data.",
        "\n\n중요: background, live_note_summary, study_hints, ready_to_use_label, ready_to_use, advice, daily_plan.label, daily_plan.tasks 등 모든 텍스트를 한국어로 작성. task_name・course_name・deadline은 원본 데이터 그대로.",
    );
    let base_system_prompt = if config.provider == "local" {
        LOCAL_TODO_SYSTEM_PROMPT
    } else {
        TODO_SYSTEM_PROMPT
    };
    let sys = if lang_hint.is_empty() {
        base_system_prompt.to_string()
    } else {
        format!("{}{}", base_system_prompt, lang_hint)
    };
    let messages = vec![
        ai::ChatMessage {
            role: "system".into(),
            content: sys,
            images: Vec::new(),
        },
        ai::ChatMessage {
            role: "user".into(),
            content: prompt,
            images: Vec::new(),
        },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!("ai_analyze_todo: got response ({} chars)", response.len());

    let json_str = if is_local {
        extract_json_from_local_response(&response)?
    } else {
        let sanitized = sanitize_ai_response_text(&response);
        if sanitized.is_empty() {
            return Err("AI応答が空です。".into());
        }
        extract_json_from_response(&sanitized).to_string()
    };
    let result: serde_json::Value = serde_json::from_str(&json_str)
        .or_else(|_| {
            log::warn!("ai todo: initial JSON parse failed, attempting truncation repair");
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

    let result = normalize_ai_todo_json(result);

    let mut cache_value = result.clone();
    if let Some(obj) = cache_value.as_object_mut() {
        obj.insert(
            "_cache_fingerprint".to_string(),
            serde_json::Value::String(cache_fingerprint),
        );
    }

    let cache_json = serde_json::to_string(&cache_value).unwrap_or_default();
    let _ = db.save_data_cache(TODO_AI_CACHE_KEY, &cache_json);

    Ok(strip_todo_internal_fields(cache_value))
}

// ── 詳細TODO抽出 (extract TODOs from Luna 消息/課題/通知) ──────────────────

const DETAIL_TODO_CACHE_KEY: &str = "ai_detail_todo_suggestions";
const DETAIL_TODO_CACHE_MAX_AGE: i64 = 6 * 3600; // 6 hours

const DETAIL_TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学のLunaシステムと学生のメール受信箱を監視するAIです。
Lunaに届いた最新の「お知らせ」「掲示板スレッド（メッセージ）」「課題」と、受信メールを読み、**明確な締切日時を含み、学生が実際に行動する必要があるTODO項目**だけを抽出してください。

## 抽出基準（厳格）
- **必ず明確な締切（日付・時刻）が文中に書かれている項目だけを採用**する。締切が読み取れない場合は採用しない。
- 単なる連絡・案内・既読推奨・補足説明・お礼などは採用しない。
- 既に提出済み・終了・回答済みと明示されているものは採用しない。
- 同じ内容が複数の通知やメールで重複している場合は、最新のものだけを採用する。
- メールは件名・本文プレビューから締切が読み取れる場合のみ採用。広告・宣伝・通知のみのメールは無視。

## 出力JSON形式（他のテキストは一切不要、マークダウンのコードブロックも使わない）
{
  "items": [
    {
      "title": "学生がやるべきこと（簡潔に1文、原文の語彙を活かす）",
      "course_name": "科目名（course_info そのまま）。メール由来でコース不明なら差出人名を入れる",
      "content_type": "課題|お知らせ|掲示板|テスト|アンケート|メール",
      "deadline": "YYYY-MM-DD HH:MM（時刻不明なら YYYY-MM-DD 23:59）",
      "source_url": "Luna 由来なら通知の url。メール由来なら 'mail://<mail_id>' 形式",
      "source_excerpt": "通知の content や メールの subject/preview から該当箇所を 60〜140 字程度で抜粋",
      "note": "なぜこれをTODOと判断したかを 1 文（学生に提示する補足説明、20〜60字）"
    }
  ]
}

## 規則
- items が空でも `{"items": []}` を返す。
- deadlineは必ずYYYY-MM-DD HH:MM形式。元文に「6月20日 17:00まで」とあれば、現在の年を補って "2026-06-20 17:00" のように整形する。年が文中に無い場合は今日の日付から推測する（過去にならないよう来年にする）。
- title は学生視点で書く（例: 「第3回レポートを提出する」「アンケートに回答する」）。
- 1 通の通知やメールから複数のTODOが読み取れる場合は分割してよい。
- 回答は指定された言語で書くこと。"#;

const LOCAL_DETAIL_TODO_SYSTEM_PROMPT: &str = r#"Lunaの「お知らせ・掲示板・課題」と受信メールを読み、明確な締切がある実行必須項目のみJSONで抽出する。

重要:
- JSON以外の文字を出力しない
- マークダウンのコードブロックは使わない
- 締切日時が文中に無いものは絶対に出力しない
- メールの広告・案内・宣伝は無視

形式:
{
  "items": [
    {
      "title": "やることを1文",
      "course_name": "科目名 (メールならコース不明時に差出人名)",
      "content_type": "課題|お知らせ|掲示板|テスト|アンケート|メール",
      "deadline": "YYYY-MM-DD HH:MM",
      "source_url": "通知のurl または 'mail://<mail_id>'",
      "source_excerpt": "原文の60〜140字抜粋",
      "note": "判断理由1文"
    }
  ]
}

ルール:
- items が無ければ {"items": []} を返す
- deadline は YYYY-MM-DD HH:MM 厳守。時刻不明は 23:59 とする
- 回答は指定された言語で書くこと"#;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DetailTodoSuggestion {
    pub title: String,
    pub course_name: String,
    pub content_type: String,
    pub deadline: String,
    pub source_url: String,
    pub source_excerpt: String,
    pub note: String,
}

fn detail_module_kind(module: &str) -> Option<&'static str> {
    let m = module.trim();
    if m.contains("掲示板") || m.contains("スレッド") || m.contains("メッセージ") {
        Some("掲示板")
    } else if m.contains("課題") || m.contains("レポート") {
        Some("課題")
    } else if m.contains("お知らせ") || m.contains("通知") {
        Some("お知らせ")
    } else if m.contains("テスト") || m.contains("試験") {
        Some("テスト")
    } else if m.contains("アンケート") || m.contains("調査") {
        Some("アンケート")
    } else {
        None
    }
}

fn build_detail_todo_fingerprint(
    notifs: &[(usize, &crate::luna_parser::LunaNotification)],
    mails: &[(usize, &crate::mail::MailMessage)],
) -> String {
    let mut parts: Vec<String> = notifs
        .iter()
        .map(|(_, n)| {
            format!(
                "L|{}|{}|{}|{}|{}",
                n.date, n.course_info, n.module, n.idnumber, n.content
            )
        })
        .collect();
    parts.extend(mails.iter().map(|(_, m)| {
        let body_sig = m
            .body
            .as_ref()
            .and_then(|b| b.content.as_deref())
            .unwrap_or("");
        format!(
            "M|{}|{}|{}|{}|{}",
            m.id,
            m.received_date_time.clone().unwrap_or_default(),
            m.subject.clone().unwrap_or_default(),
            m.body_preview.clone().unwrap_or_default(),
            body_sig.len()
        )
    }));
    parts.sort();
    let payload = parts.join("||");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn build_detail_todo_prompt(
    notifs: &[(usize, &crate::luna_parser::LunaNotification)],
    mails: &[(usize, &crate::mail::MailMessage)],
) -> String {
    let today = chrono::Local::now();
    let mut text = String::new();
    text.push_str(&format!(
        "## 今日: {} ({})\n",
        today.format("%Y年%m月%d日"),
        today.format("%A")
    ));
    text.push_str(&format!(
        "## 現在時刻: {}\n",
        today.format("%Y-%m-%d %H:%M")
    ));

    if !notifs.is_empty() {
        text.push_str("\n## Luna 通知一覧\n");
        text.push_str("以下は Luna の updateinfo に表示されている最新の お知らせ／掲示板／課題 です。各項目から **明確な締切がある TODO のみ** を抽出してください。\n\n");

        for (idx, n) in notifs {
            let kind = detail_module_kind(&n.module).unwrap_or("その他");
            text.push_str(&format!(
                "### [L{idx}] {date} | {course} | {module}（種別: {kind}）\n",
                idx = idx,
                date = n.date,
                course = if n.course_info.is_empty() {
                    "(コース不明)"
                } else {
                    n.course_info.as_str()
                },
                module = n.module,
                kind = kind,
            ));
            if !n.content.is_empty() {
                text.push_str(&format!("content: {}\n", n.content));
            }
            if !n.url.is_empty() {
                text.push_str(&format!("url: {}\n", n.url));
            }
            text.push('\n');
        }
    }

    if !mails.is_empty() {
        text.push_str("\n## メール一覧\n");
        text.push_str("以下は受信箱の最新メールです。**学生が締切までに行動する必要があるもの** (申請、提出、回答など) のみ TODO として抽出してください。広告／案内のみのメールは無視。\n\n");

        for (idx, m) in mails {
            let from_str = m
                .from
                .as_ref()
                .and_then(|f| {
                    let n = f.email_address.name.clone().unwrap_or_default();
                    let a = f.email_address.address.clone().unwrap_or_default();
                    if !n.is_empty() && !a.is_empty() {
                        Some(format!("{} <{}>", n, a))
                    } else if !n.is_empty() {
                        Some(n)
                    } else if !a.is_empty() {
                        Some(a)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "(差出人不明)".to_string());
            text.push_str(&format!(
                "### [M{idx}] {date} | {from} | 種別: メール\n",
                idx = idx,
                date = m.received_date_time.clone().unwrap_or_default(),
                from = from_str,
            ));
            if let Some(subj) = &m.subject {
                if !subj.is_empty() {
                    text.push_str(&format!("subject: {}\n", subj));
                }
            }
            // Prefer the full plain-text body; fall back to the short preview when
            // older cache entries lack it. Truncate so a single huge mail doesn't
            // dominate the prompt budget.
            let full_body = m
                .body
                .as_ref()
                .and_then(|b| b.content.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match full_body {
                Some(content) => {
                    let truncated: String = content.chars().take(1200).collect();
                    text.push_str(&format!("body: {}\n", truncated));
                    if content.chars().count() > 1200 {
                        text.push_str("body: ...(truncated)\n");
                    }
                }
                None => {
                    if let Some(preview) = &m.body_preview {
                        if !preview.is_empty() {
                            text.push_str(&format!("preview: {}\n", preview));
                        }
                    }
                }
            }
            text.push_str(&format!("mail_id: {}\n\n", m.id));
        }
    }

    text
}

#[tauri::command]
pub async fn ai_extract_detail_todos(
    db: State<'_, Database>,
    force: bool,
) -> Result<Vec<DetailTodoSuggestion>, String> {
    let config = ai::load_ai_config();

    let notifs: Vec<crate::luna_parser::LunaNotification> = db
        .get_data_cache("luna_updates")
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let filtered: Vec<(usize, &crate::luna_parser::LunaNotification)> = notifs
        .iter()
        .enumerate()
        .filter(|(_, n)| detail_module_kind(&n.module).is_some())
        .collect();

    let mails: Vec<crate::mail::MailMessage> = db
        .get_data_cache("mail_inbox")
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    // Limit mail volume to keep prompt size manageable — most TODO-bearing mail
    // is recent; older items are usually already-actioned or noise.
    let mail_refs: Vec<(usize, &crate::mail::MailMessage)> =
        mails.iter().enumerate().take(40).collect();

    if filtered.is_empty() && mail_refs.is_empty() {
        return Ok(Vec::new());
    }

    let fingerprint = build_detail_todo_fingerprint(&filtered, &mail_refs);

    if !force {
        if let Ok(Some((json, ts))) = db.get_data_cache(DETAIL_TODO_CACHE_KEY) {
            let now = crate::db::epoch_secs();
            if now - ts < DETAIL_TODO_CACHE_MAX_AGE {
                if let Ok(cached) = serde_json::from_str::<serde_json::Value>(&json) {
                    if cached.get("_fingerprint").and_then(|v| v.as_str())
                        == Some(fingerprint.as_str())
                    {
                        if let Some(items) = cached.get("items") {
                            if let Ok(parsed) =
                                serde_json::from_value::<Vec<DetailTodoSuggestion>>(items.clone())
                            {
                                log::info!(
                                    "ai_extract_detail_todos: cache hit (age={}s, items={})",
                                    now - ts,
                                    parsed.len()
                                );
                                return Ok(parsed);
                            }
                        }
                    }
                }
            }
        }
    }

    let is_local = config.provider == "local";
    let prompt = build_detail_todo_prompt(&filtered, &mail_refs);

    log::info!(
        "ai_extract_detail_todos: calling AI ({} notifications, {} mails, {} chars prompt, local={})",
        filtered.len(),
        mail_refs.len(),
        prompt.len(),
        is_local
    );

    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\n重要: title, source_excerpt, note 等の文章は中文（简体字）で書く。course_name, deadline, source_url は元データのまま。",
        "\n\nIMPORTANT: Write title, source_excerpt, note in English. Keep course_name, deadline, source_url as-is.",
        "\n\n중요: title, source_excerpt, note는 한국어로 작성. course_name, deadline, source_url는 원본 그대로.",
    );
    let base_system = if is_local {
        LOCAL_DETAIL_TODO_SYSTEM_PROMPT
    } else {
        DETAIL_TODO_SYSTEM_PROMPT
    };
    let sys = if lang_hint.is_empty() {
        base_system.to_string()
    } else {
        format!("{}{}", base_system, lang_hint)
    };

    let messages = vec![
        ai::ChatMessage {
            role: "system".into(),
            content: sys,
            images: Vec::new(),
        },
        ai::ChatMessage {
            role: "user".into(),
            content: prompt,
            images: Vec::new(),
        },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!(
        "ai_extract_detail_todos: got response ({} chars)",
        response.len()
    );

    let json_str = if is_local {
        extract_json_from_local_response(&response)?
    } else {
        let sanitized = sanitize_ai_response_text(&response);
        if sanitized.is_empty() {
            return Err("AI応答が空です。".into());
        }
        extract_json_from_response(&sanitized).to_string()
    };

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .or_else(|_| {
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

    let mut items: Vec<DetailTodoSuggestion> = parsed
        .get("items")
        .and_then(|v| serde_json::from_value::<Vec<DetailTodoSuggestion>>(v.clone()).ok())
        .unwrap_or_default();

    // Drop items without a parseable deadline — the prompt requires one, this enforces it.
    items.retain(|item| !item.deadline.trim().is_empty() && !item.title.trim().is_empty());

    let cache_value = serde_json::json!({
        "items": items,
        "_fingerprint": fingerprint,
    });
    let _ = db.save_data_cache(
        DETAIL_TODO_CACHE_KEY,
        &serde_json::to_string(&cache_value).unwrap_or_default(),
    );

    Ok(items)
}

pub(super) fn load_ai_cache(db: &Database) -> Result<(Option<AiScheduleResult>, bool), String> {
    load_ai_cache_inner(db).map(|opt| match opt {
        Some((result, ts)) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let stale = now - ts > AI_CACHE_MAX_AGE;
            (Some(result), stale)
        }
        None => (None, true),
    })
}

fn load_ai_cache_inner(db: &Database) -> Result<Option<(AiScheduleResult, i64)>, String> {
    db.get_ai_schedule_cache()
}

fn build_todo_ai_prompt(
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
    let pending: Vec<&crate::luna_parser::LunaTodoItem> = todos
        .iter()
        .filter(|t| !t.status.contains("提出済"))
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

fn collect_relevant_live_notes(
    todos: &[crate::luna_parser::LunaTodoItem],
) -> Vec<RelevantLiveNote> {
    let mut course_names: Vec<String> = todos
        .iter()
        .filter(|t| !t.status.contains("提出済"))
        .map(|t| t.course_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    course_names.sort();
    course_names.dedup();
    if course_names.is_empty() {
        return Vec::new();
    }

    let records = commands::scan_download_dir();
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
        path: record.path.clone(),
        downloaded_at: record.downloaded_at,
        summary,
    })
}

fn compact_live_note_markdown(markdown: &str, max_chars: usize) -> String {
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

fn build_todo_cache_fingerprint(
    todos: &[crate::luna_parser::LunaTodoItem],
    todo_cache_ts: i64,
    snapshot_updated_at: i64,
    live_notes: &[RelevantLiveNote],
) -> String {
    let mut pending: Vec<String> = todos
        .iter()
        .filter(|t| !t.status.contains("提出済"))
        .map(|t| {
            format!(
                "{}|{}|{}|{}|{}",
                t.course_name, t.content_type, t.content_name, t.deadline, t.feedback
            )
        })
        .collect();
    pending.sort();

    let mut note_parts: Vec<String> = live_notes
        .iter()
        .map(|note| {
            format!(
                "{}|{}|{}|{}",
                note.matched_course_name, note.file_name, note.path, note.downloaded_at
            )
        })
        .collect();
    note_parts.sort();

    let payload = format!(
        "todo_cache_ts={todo_cache_ts}|snapshot_updated_at={snapshot_updated_at}|pending={}|live_notes={}",
        pending.join("||"),
        note_parts.join("||"),
    );
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn extract_todo_cache_fingerprint(value: &serde_json::Value) -> Option<&str> {
    value
        .get("_cache_fingerprint")
        .and_then(|fingerprint| fingerprint.as_str())
}

fn strip_todo_internal_fields(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("_cache_fingerprint");
    }
    value
}

fn build_ai_schedule_prompt(raw: &ScheduleRawData, is_local: bool) -> String {
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

fn parse_ai_schedule_response(
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

fn sanitize_ai_response_text(text: &str) -> String {
    let mut s = strip_tag_block_case_insensitive(text, "think");
    s = strip_token_case_insensitive(&s, "<think>");
    s = strip_token_case_insensitive(&s, "</think>");
    s.trim().to_string()
}

fn strip_tag_block_case_insensitive(text: &str, tag: &str) -> String {
    let mut out = text.to_string();
    let open_prefix = format!("<{}", tag.to_ascii_lowercase());
    let close_tag = format!("</{}>", tag.to_ascii_lowercase());

    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find(&open_prefix) else {
            break;
        };

        let Some(open_end_rel) = lower[start..].find('>') else {
            out.truncate(start);
            break;
        };
        let content_start = start + open_end_rel + 1;

        if let Some(close_rel) = lower[content_start..].find(&close_tag) {
            let end = content_start + close_rel + close_tag.len();
            out.replace_range(start..end, "");
        } else {
            out.replace_range(start..out.len(), "");
            break;
        }
    }

    out
}

fn strip_token_case_insensitive(text: &str, token: &str) -> String {
    let mut out = text.to_string();
    let token_lower = token.to_ascii_lowercase();

    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find(&token_lower) else {
            break;
        };
        let end = start + token.len();
        if end <= out.len() {
            out.replace_range(start..end, "");
        } else {
            break;
        }
    }

    out
}

fn normalize_ai_schedule_json(mut root: serde_json::Value) -> serde_json::Value {
    if !root.is_object() {
        return serde_json::json!({
            "current_week": [],
            "next_week": [],
            "weekly_summary": "",
            "cross_week_insights": "",
        });
    }

    if let Some(obj) = root.as_object_mut() {
        let current_week = obj
            .remove("current_week")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let next_week = obj
            .remove("next_week")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let weekly_summary = obj
            .remove("weekly_summary")
            .unwrap_or(serde_json::Value::Null);
        let cross_week_insights = obj
            .remove("cross_week_insights")
            .unwrap_or(serde_json::Value::Null);

        obj.insert(
            "current_week".to_string(),
            normalize_schedule_items(current_week),
        );
        obj.insert("next_week".to_string(), normalize_schedule_items(next_week));
        obj.insert(
            "weekly_summary".to_string(),
            serde_json::Value::String(value_to_string(weekly_summary)),
        );
        obj.insert(
            "cross_week_insights".to_string(),
            serde_json::Value::String(value_to_string(cross_week_insights)),
        );
    }

    root
}

fn normalize_ai_todo_json(mut root: serde_json::Value) -> serde_json::Value {
    if !root.is_object() {
        return serde_json::json!({
            "task_guides": [],
            "daily_plan": [],
            "advice": "",
        });
    }

    if let Some(obj) = root.as_object_mut() {
        let task_guides = obj
            .remove("task_guides")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let daily_plan = obj
            .remove("daily_plan")
            .unwrap_or(serde_json::Value::Array(vec![]));
        let advice = remove_first_value(obj, &["advice", "summary"]);

        obj.insert(
            "task_guides".to_string(),
            normalize_todo_task_guides(task_guides),
        );
        obj.insert(
            "daily_plan".to_string(),
            normalize_todo_daily_plan(daily_plan),
        );
        obj.insert(
            "advice".to_string(),
            serde_json::Value::String(value_to_string(advice)),
        );
        obj.remove("_check");
    }

    root
}

fn normalize_todo_task_guides(value: serde_json::Value) -> serde_json::Value {
    let arr = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    let guides: Vec<serde_json::Value> = arr
        .into_iter()
        .filter_map(normalize_todo_task_guide)
        .collect();
    serde_json::Value::Array(guides)
}

fn normalize_todo_task_guide(value: serde_json::Value) -> Option<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(obj) => obj,
        _ => return None,
    };

    let task_name = value_to_string(remove_first_value(
        &mut obj,
        &["task_name", "title", "task"],
    ));
    let course_name = value_to_string(remove_first_value(
        &mut obj,
        &["course_name", "course", "subject"],
    ));
    if task_name.trim().is_empty() && course_name.trim().is_empty() {
        return None;
    }

    let deadline = value_to_string(remove_first_value(
        &mut obj,
        &["deadline", "due", "due_at", "period"],
    ));
    let urgency = normalize_todo_urgency(value_to_string(remove_first_value(
        &mut obj,
        &["urgency", "priority"],
    )));
    let background = value_to_string(remove_first_value(
        &mut obj,
        &["background", "context", "summary"],
    ));
    let live_note_summary = value_to_string(remove_first_value(
        &mut obj,
        &[
            "live_note_summary",
            "note_summary",
            "note_context",
            "class_note_summary",
        ],
    ));
    let study_hints = value_to_string_vec(remove_first_value(
        &mut obj,
        &["study_hints", "steps", "hints", "actions"],
    ));
    let ready_to_use_label = value_to_string(remove_first_value(
        &mut obj,
        &[
            "ready_to_use_label",
            "starter_output_label",
            "draft_label",
            "output_label",
        ],
    ));
    let ready_to_use = value_to_string(remove_first_value(
        &mut obj,
        &[
            "ready_to_use",
            "starter_output",
            "draft",
            "output",
            "template",
        ],
    ));
    let estimated_minutes = value_to_i32(remove_first_value(
        &mut obj,
        &["estimated_minutes", "minutes", "eta_minutes"],
    ))
    .max(0);

    Some(serde_json::json!({
        "task_name": task_name,
        "course_name": course_name,
        "deadline": deadline,
        "urgency": urgency,
        "background": background,
        "live_note_summary": live_note_summary,
        "study_hints": study_hints,
        "ready_to_use_label": ready_to_use_label,
        "ready_to_use": ready_to_use,
        "estimated_minutes": estimated_minutes,
    }))
}

fn normalize_todo_daily_plan(value: serde_json::Value) -> serde_json::Value {
    let arr = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    let plans: Vec<serde_json::Value> = arr
        .into_iter()
        .filter_map(normalize_todo_daily_plan_item)
        .collect();
    serde_json::Value::Array(plans)
}

fn normalize_todo_daily_plan_item(value: serde_json::Value) -> Option<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(obj) => obj,
        _ => return None,
    };

    let label = value_to_string(remove_first_value(&mut obj, &["label", "day"]));
    let tasks = value_to_string_vec(remove_first_value(&mut obj, &["tasks", "items"]));
    if label.trim().is_empty() && tasks.is_empty() {
        return None;
    }

    let free_hours = value_to_f64(remove_first_value(
        &mut obj,
        &["free_hours", "free_time", "hours"],
    ))
    .max(0.0);
    Some(serde_json::json!({
        "label": label,
        "tasks": tasks,
        "free_hours": free_hours,
    }))
}

fn normalize_todo_urgency(urgency: String) -> String {
    match urgency.trim().to_ascii_lowercase().as_str() {
        "overdue" => "overdue".to_string(),
        "critical" | "urgent" => "critical".to_string(),
        "soon" | "warning" => "soon".to_string(),
        _ => "normal".to_string(),
    }
}

fn remove_first_value(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> serde_json::Value {
    for key in keys {
        if let Some(value) = obj.remove(*key) {
            return value;
        }
    }
    serde_json::Value::Null
}

fn normalize_schedule_items(value: serde_json::Value) -> serde_json::Value {
    let arr = match value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    let out: Vec<serde_json::Value> = arr
        .into_iter()
        .filter_map(normalize_schedule_item)
        .collect();
    serde_json::Value::Array(out)
}

fn normalize_schedule_item(value: serde_json::Value) -> Option<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };

    let item = serde_json::json!({
        "day": value_to_i32(obj.remove("day").unwrap_or(serde_json::Value::Null)),
        "period": value_to_i32(obj.remove("period").unwrap_or(serde_json::Value::Null)),
        "course_name": value_to_string(obj.remove("course_name").unwrap_or(serde_json::Value::Null)),
        "delivery_mode": value_to_string(obj.remove("delivery_mode").unwrap_or(serde_json::Value::Null)),
        "room": value_to_string(obj.remove("room").unwrap_or(serde_json::Value::Null)),
        "teacher": value_to_string(obj.remove("teacher").unwrap_or(serde_json::Value::Null)),
        "session_topic": value_to_string(obj.remove("session_topic").unwrap_or(serde_json::Value::Null)),
        "is_cancelled": value_to_bool(obj.remove("is_cancelled").unwrap_or(serde_json::Value::Null)),
        "notifications": value_to_string_vec(obj.remove("notifications").unwrap_or(serde_json::Value::Null)),
        "assignments": value_to_string_vec(obj.remove("assignments").unwrap_or(serde_json::Value::Null)),
        "exams": value_to_string_vec(obj.remove("exams").unwrap_or(serde_json::Value::Null)),
    });

    Some(item)
}

fn value_to_i32(v: serde_json::Value) -> i32 {
    match v {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        serde_json::Value::String(s) => s.trim().parse::<i32>().unwrap_or(0),
        serde_json::Value::Bool(b) => {
            if b {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn value_to_bool(v: serde_json::Value) -> bool {
    match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        serde_json::Value::String(s) => {
            let t = s.trim().to_lowercase();
            t == "true" || t == "1" || t == "yes"
        }
        _ => false,
    }
}

fn value_to_f64(v: serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        serde_json::Value::Bool(b) => {
            if b {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

fn value_to_string_vec(v: serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::Array(a) => a
            .into_iter()
            .map(value_to_string)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        serde_json::Value::Null => Vec::new(),
        other => {
            let s = value_to_string(other).trim().to_string();
            if s.is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
    }
}

fn value_to_string(v: serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => {
            if b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        serde_json::Value::Array(a) => {
            let parts: Vec<String> = a
                .into_iter()
                .map(value_to_string)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            parts.join(" / ")
        }
        serde_json::Value::Object(mut m) => {
            for key in [
                "text", "value", "name", "title", "content", "teacher", "room",
            ] {
                if let Some(val) = m.remove(key) {
                    let s = value_to_string(val).trim().to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            serde_json::to_string(&serde_json::Value::Object(m)).unwrap_or_default()
        }
    }
}

fn extract_json_from_local_response(response: &str) -> Result<String, String> {
    let lower = response.to_ascii_lowercase();
    let mut segments: Vec<&str> = Vec::new();
    let mut pos: usize = 0;
    let mut had_think = false;

    loop {
        match lower[pos..].find("<think") {
            Some(start_rel) => {
                had_think = true;
                let start = pos + start_rel;
                if start > pos {
                    segments.push(&response[pos..start]);
                }
                let after_open = match lower[start..].find('>') {
                    Some(i) => start + i + 1,
                    None => break,
                };
                match lower[after_open..].find("</think>") {
                    Some(close_rel) => {
                        pos = after_open + close_rel + "</think>".len();
                    }
                    None => {
                        break;
                    }
                }
            }
            None => {
                segments.push(&response[pos..]);
                break;
            }
        }
    }

    let clean = segments.join("");
    let trimmed = clean.trim();

    if trimmed.is_empty() {
        if had_think {
            return Err(
                "AIモデルが推論のみで出力トークンを使い切り、JSONが生成されませんでした。\
                モデルのmax_tokensを増やすか、プロンプトを短くしてください。"
                    .into(),
            );
        } else {
            return Err("AIモデルから空の応答が返されました。".into());
        }
    }

    log::debug!(
        "extract_json_from_local_response: response {}→{} chars (think={})",
        response.len(),
        trimmed.len(),
        had_think
    );

    Ok(extract_json_from_response(trimmed).to_string())
}

fn extract_json_from_response(text: &str) -> &str {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find('{') {
        if let Some(end) = find_matching_brace(text, start) {
            return &text[start..=end];
        }
        if let Some(end) = text.rfind('}') {
            if end > start {
                return &text[start..=end];
            }
        }
        return &text[start..];
    }
    text.trim()
}

fn scan_json_structure(s: &str, offset: usize, mut f: impl FnMut(usize, char)) -> bool {
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in s[offset..].char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        f(offset + i, ch);
    }
    in_string
}

fn find_matching_brace(text: &str, start: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut result = None;
    scan_json_structure(text, start, |i, ch| {
        if result.is_some() {
            return;
        }
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    result = Some(i);
                }
            }
            _ => {}
        }
    });
    result
}

fn repair_truncated_json(input: &str) -> String {
    let s = input.trim_end();

    if serde_json::from_str::<serde_json::Value>(s).is_ok() {
        return s.to_string();
    }

    let mut cleaned = s.to_string();
    loop {
        let t = cleaned.trim_end();
        if t.is_empty() {
            break;
        }
        let last = t.as_bytes()[t.len() - 1];
        if last == b':' || last == b',' || last == b'\\' {
            cleaned = t[..t.len() - 1].to_string();
            continue;
        }
        break;
    }

    let attempt1 = close_json_brackets(&cleaned);
    if serde_json::from_str::<serde_json::Value>(&attempt1).is_ok() {
        return attempt1;
    }

    let commas = find_non_string_commas(&cleaned);
    for &pos in commas.iter().rev().take(20) {
        let candidate = close_json_brackets(&cleaned[..pos]);
        if serde_json::from_str::<serde_json::Value>(&candidate).is_ok() {
            return candidate;
        }
    }

    attempt1
}

fn close_json_brackets(s: &str) -> String {
    let mut stack: Vec<char> = Vec::new();
    let in_string = scan_json_structure(s, 0, |_, ch| match ch {
        '{' => stack.push('{'),
        '[' => stack.push('['),
        '}' => {
            if stack.last() == Some(&'{') {
                stack.pop();
            }
        }
        ']' => {
            if stack.last() == Some(&'[') {
                stack.pop();
            }
        }
        _ => {}
    });

    let mut result = s.to_string();
    if in_string {
        result.push('"');
    }
    for &bracket in stack.iter().rev() {
        match bracket {
            '{' => result.push('}'),
            '[' => result.push(']'),
            _ => {}
        }
    }
    result
}

fn find_non_string_commas(s: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    scan_json_structure(s, 0, |i, ch| {
        if ch == ',' {
            positions.push(i);
        }
    });
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_live_note_markdown_omits_full_transcript() {
        let markdown = "# Course\n\n- 授業コード: TEST\n- 開始: 2026-04-21 09:00:00\n\n### 全体要約\n今日は4P分析とSTPを扱った。\n\n## 区間ごとの要約\n\n## 1つ目\n導入と事例整理。\n\n## 全文転写\n\n- [09:00] transcript";
        let compact = compact_live_note_markdown(markdown, 500);
        assert!(compact.contains("今日は4P分析とSTPを扱った。"));
        assert!(!compact.contains("transcript"));
    }

    #[test]
    fn normalize_ai_todo_json_fills_new_api_fields() {
        let normalized = normalize_ai_todo_json(serde_json::json!({
            "task_guides": [{
                "task_name": "課題A",
                "course_name": "科目A",
                "deadline": "2026/04/22 23:59",
                "urgency": "urgent",
                "background": "背景",
                "note_context": "授業ではSTPを扱った",
                "steps": ["整理する"],
                "draft_label": "提纲",
                "draft": "1. 導入",
                "minutes": "45"
            }],
            "daily_plan": [{
                "day": "今日（4/21）",
                "items": ["課題A（45分）"],
                "hours": "3.5"
            }],
            "summary": "先に着手する。"
        }));

        assert_eq!(normalized["task_guides"][0]["urgency"], "critical");
        assert_eq!(
            normalized["task_guides"][0]["live_note_summary"],
            "授業ではSTPを扱った"
        );
        assert_eq!(normalized["task_guides"][0]["ready_to_use_label"], "提纲");
        assert_eq!(normalized["daily_plan"][0]["free_hours"], 3.5);
        assert_eq!(normalized["advice"], "先に着手する。");
    }
}
