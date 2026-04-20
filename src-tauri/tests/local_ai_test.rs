use app_lib::ai::ChatMessage;
/// Integration test — runs the real Qwen3.5-2B model with realistic schedule data.
/// Run with: cargo test --test local_ai_test -- --nocapture
///
/// This test is ignored by default because it requires ~1.2 GB model to be downloaded.
use app_lib::local_ai::{run_inference, InferenceRequest, SamplerConfig};

const SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生向けスケジュール分析AIです。
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
- 回答は日本語で書くこと"#;

const USER_PROMPT: &str = r#"## 今日の日付: 2026-04-18 (土曜日)

## 学期情報
- 春学期開始: 2026-04-06
- ★ 現在は春学期 第2週目

## 今週: 4/13(月)～4/18(土)
### KGC時間割（今週）
- 月2限:英語IIIa [EN301] 教室:B-202
- 月3限:データサイエンス入門 [DS101] 教室:A-301
- 火1限:マクロ経済学 [EC201] 教室:C-103 [休講]
- 水2限:プログラミング演習 [CS202] 教室:PC-Lab-1
- 水4限:第二外国語(中国語) [CH101] 教室:D-405
- 木1限:線形代数 [MA201] 教室:B-101
- 木3限:日本史概論 [HI101] 教室:A-205
- 金2限:英語IIIb [EN302] 教室:B-203

## 来週: 4/20(月)～4/25(土)
### KGC時間割（来週）
- 月2限:英語IIIa [EN301] 教室:B-202
- 月3限:データサイエンス入門 [DS101] 教室:A-301
- 火1限:マクロ経済学 [EC201] 教室:C-103
- 水2限:プログラミング演習 [CS202] 教室:PC-Lab-1
- 水4限:第二外国語(中国語) [CH101] 教室:D-405
- 木1限:線形代数 [MA201] 教室:B-101
- 木3限:日本史概論 [HI101] 教室:A-205
- 金2限:英語IIIb [EN302] 教室:B-203

### Luna登録コース
- 月2限: English IIIa [luna_id:1001] 教員:Smith
- 月3限: データサイエンス入門 [luna_id:1002] 教員:田中太郎
- 火1限: マクロ経済学 [luna_id:1003] 教員:佐藤次郎
- 水2限: プログラミング演習 [luna_id:1004] 教員:鈴木花子
- 水4限: 中国語初級I [luna_id:1005] 教員:王明
- 木1限: 線形代数 [luna_id:1006] 教員:山田一郎
- 木3限: 日本史概論 [luna_id:1007] 教員:高橋史子
- 金2限: English IIIb [luna_id:1008] 教員:Johnson

### 授業計画
#### データサイエンス入門 [DS101]
  第2回: データの種類と尺度水準。質的・量的データの区別
  第3回: 記述統計量（平均・中央値・分散）。Pythonで計算

#### プログラミング演習 [CS202]
  第2回: 変数・データ型・演算子。Python基礎文法
  第3回: 条件分岐（if文）。フローチャートの書き方

#### 線形代数 [MA201]
  第2回: 行列の定義と演算。加法・スカラー倍
  第3回: 行列の積。結合法則と非可換性

### Luna活動詳細
#### English IIIa [1001]
  [課題] Unit 2 Reading Response (期限: 2026-04-20 23:59) {状態: 未提出}
  [テスト] Vocabulary Quiz Week 2 (期間: 2026-04-21 09:00～2026-04-21 09:30) {状態: 未受験}

#### プログラミング演習 [1004]
  [課題] Python基礎演習1 (期限: 2026-04-22 17:00) {状態: 未提出}

#### マクロ経済学 [1003]
  [お知らせ] 4/14の授業は休講です。補講日は後日連絡します。"#;

#[test]
#[ignore] // remove to run: cargo test --test local_ai_test -- --ignored --nocapture
fn test_schedule_generation() {
    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.into(),
            images: vec![],
        },
        ChatMessage {
            role: "user".into(),
            content: USER_PROMPT.into(),
            images: vec![],
        },
    ];

    eprintln!("=== Loading model and running inference ===");
    eprintln!("System prompt: {} chars", SYSTEM_PROMPT.len());
    eprintln!("User prompt: {} chars", USER_PROMPT.len());

    let start = std::time::Instant::now();
    let result = run_inference(InferenceRequest {
        model_id: "qwen3.5-2b".into(),
        file_name: "Qwen3.5-2B-Q4_K_M.gguf".into(),
        messages,
        sampler: SamplerConfig::default(),
        max_tokens: 8192,
        prefill: String::new(),
        gen_id: String::new(),
        think_budget_pct: 40,
    });
    let elapsed = start.elapsed();

    match result {
        Ok(output) => {
            eprintln!(
                "\n=== Output ({:.1}s, {} chars) ===",
                elapsed.as_secs_f64(),
                output.len()
            );
            eprintln!("{}", output);

            // Basic checks
            let has_think = output.contains("<think>");
            let has_think_close = output.contains("</think>");
            let has_json_brace = output.contains('{');
            let has_current_week = output.contains("current_week");

            eprintln!("\n=== Diagnostics ===");
            eprintln!("Has <think>: {}", has_think);
            eprintln!("Has </think>: {}", has_think_close);
            eprintln!("Has JSON brace: {}", has_json_brace);
            eprintln!("Has 'current_week': {}", has_current_week);

            if has_think {
                // Measure think block size
                if let Some(think_start) = output.find("<think>") {
                    if let Some(think_end) = output.find("</think>") {
                        let think_len = think_end - think_start;
                        let total_len = output.len();
                        eprintln!(
                            "Think block: {} chars ({:.0}% of output)",
                            think_len,
                            think_len as f64 / total_len as f64 * 100.0
                        );
                    } else {
                        eprintln!("WARNING: <think> opened but never closed!");
                    }
                }
            }

            // Try JSON extraction (simulate extract_json_from_local_response logic)
            let json_text = if has_think {
                // Strip think blocks
                let mut clean = output.clone();
                while let Some(start) = clean.find("<think>") {
                    if let Some(end) = clean.find("</think>") {
                        clean = format!("{}{}", &clean[..start], &clean[end + "</think>".len()..]);
                    } else {
                        clean = clean[..start].to_string();
                        break;
                    }
                }
                clean.trim().to_string()
            } else {
                output.clone()
            };

            // Try to find JSON object
            if let Some(json_start) = json_text.find('{') {
                if let Some(json_end) = json_text.rfind('}') {
                    let json_str = &json_text[json_start..=json_end];
                    eprintln!("\n=== JSON extraction ({} chars) ===", json_str.len());
                    match serde_json::from_str::<serde_json::Value>(json_str) {
                        Ok(val) => {
                            eprintln!("JSON parse: SUCCESS");
                            if let Some(cw) = val.get("current_week").and_then(|v| v.as_array()) {
                                eprintln!("current_week entries: {}", cw.len());
                            }
                            if let Some(nw) = val.get("next_week").and_then(|v| v.as_array()) {
                                eprintln!("next_week entries: {}", nw.len());
                            }
                            if let Some(ws) = val.get("weekly_summary").and_then(|v| v.as_str()) {
                                eprintln!("weekly_summary: {}...", &ws[..ws.len().min(80)]);
                            }
                        }
                        Err(e) => {
                            eprintln!("JSON parse: FAILED - {}", e);
                            eprintln!("First 500 chars of extracted JSON:");
                            eprintln!("{}", &json_str[..json_str.len().min(500)]);
                        }
                    }
                } else {
                    eprintln!("No closing '}}' found in cleaned output");
                }
            } else {
                eprintln!("No '{{' found in cleaned output — model produced no JSON!");
                eprintln!("Cleaned output (first 500 chars):");
                eprintln!("{}", &json_text[..json_text.len().min(500)]);
            }

            // The test passes if we at least got some output.
            // JSON validity is a bonus.
            assert!(!output.is_empty(), "Model produced empty output");
        }
        Err(e) => {
            eprintln!("\n=== ERROR ({:.1}s) ===", elapsed.as_secs_f64());
            eprintln!("{}", e);
            panic!("local_chat_completion failed: {}", e);
        }
    }
}
