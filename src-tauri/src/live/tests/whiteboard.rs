use super::super::*;

fn test_whiteboard_node(
    id: &str,
    label: &str,
    node_type: &str,
    source_type: &str,
    source_excerpt: &str,
) -> LiveWhiteboardNode {
    LiveWhiteboardNode {
        id: id.to_string(),
        label: label.to_string(),
        detail: String::new(),
        node_type: node_type.to_string(),
        kind: if node_type == "term" {
            "support".to_string()
        } else {
            "core".to_string()
        },
        role: if node_type == "term" {
            "branch".to_string()
        } else {
            "main".to_string()
        },
        parent_id: String::new(),
        source_type: source_type.to_string(),
        source_excerpt: source_excerpt.to_string(),
        external_source: String::new(),
    }
}

fn test_whiteboard(nodes: Vec<LiveWhiteboardNode>) -> LiveWhiteboard {
    LiveWhiteboard {
        title: "テスト白板".to_string(),
        layout: "grid".to_string(),
        nodes,
        edges: Vec::new(),
        schema_version: 1,
        normalized_by: "backend".to_string(),
    }
}

#[test]
fn enrich_whiteboard_source_excerpts_uses_previous_terms_and_transcript() {
    let previous = test_whiteboard(vec![test_whiteboard_node(
        "old",
        "既存ノード",
        "structure",
        "lecture",
        "既存の根拠",
    )]);
    let mut term_node = test_whiteboard_node("term-source", "一次資料", "term", "lecture", "");
    term_node.parent_id = "old".to_string();
    let mut external_node =
        test_whiteboard_node("external", "外部補足", "structure", "external", "");
    external_node.external_source = "外部資料".to_string();
    let board = test_whiteboard(vec![
        test_whiteboard_node("old", "既存ノード", "structure", "lecture", ""),
        term_node,
        test_whiteboard_node("theme", "個人発表のテーマ選定", "structure", "lecture", ""),
        external_node,
    ]);
    let terms = vec![LiveTermExplanation {
        term: "一次資料".to_string(),
        explanation: "大元の資料".to_string(),
        source_excerpt: "一次資料まで遡る必要があります".to_string(),
        external_source: String::new(),
    }];
    let lines = vec![LiveTranscriptLine {
        at: "12:00:00".to_string(),
        text: "個人発表のテーマ選定では、賛否が分かれる問いを選んでください。".to_string(),
    }];

    let enriched = enrich_whiteboard_source_excerpts(Some(board), Some(&previous), &terms, &lines)
        .expect("whiteboard should remain available");
    let source_by_id = enriched
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.source_excerpt.as_str()))
        .collect::<std::collections::HashMap<_, _>>();

    assert_eq!(source_by_id.get("old"), Some(&"既存の根拠"));
    assert_eq!(
        source_by_id.get("term-source"),
        Some(&"一次資料まで遡る必要があります")
    );
    assert_eq!(
        source_by_id.get("theme"),
        Some(&"個人発表のテーマ選定では、賛否が分かれる問いを選んでください。")
    );
    assert_eq!(source_by_id.get("external"), Some(&""));
}

#[test]
fn live_prompts_keep_language_policy_consistent() {
    let language_hint = live_reply_language_hint("zh");
    assert!(language_hint.contains("简体中文"));

    let chunk_prompt = live_chunk_system_prompt(language_hint, false);
    // Call 1 prompt produces summary + terms only; it must NOT mention the
    // whiteboard schema (that lives in the standalone whiteboard prompt).
    assert!(chunk_prompt.contains("\"summary_markdown\""));
    assert!(chunk_prompt.contains("\"terms\""));
    assert!(!chunk_prompt.contains("\"whiteboard\""));
    assert!(!chunk_prompt.contains("\"role\":\"main|branch\""));
    assert!(!chunk_prompt.contains("\"node_type\":\"structure|term\""));

    let board_prompt =
        live_whiteboard_system_prompt(live_whiteboard_language_instruction("zh"), false);
    // Call 2 prompt owns the whiteboard JSON schema now.
    assert!(
        board_prompt.chars().count() < 16_000,
        "whiteboard prompt grew too large: {} chars",
        board_prompt.chars().count()
    );
    assert!(board_prompt.contains("\"role\":\"main|branch\""));
    assert!(board_prompt.contains("\"node_type\":\"structure|term\""));
    assert!(board_prompt.contains("\"source_type\":\"lecture|external\""));
    assert!(board_prompt.contains("whiteboard JSON だけ"));
    assert!(board_prompt.contains("目的"));
    assert!(board_prompt.contains("ノードや edge の量は固定目標ではなく"));
    assert!(board_prompt.contains("実行順序"));
    assert!(board_prompt.contains("内容本体・活動内容・方法論・運営連絡"));
    assert!(board_prompt.contains("パターン名自体を node にしない"));
    assert!(board_prompt.contains("内容構造の再編"));
    assert!(board_prompt.contains("時系列メモにしない"));
    assert!(board_prompt.contains("分割要約の区間や発話順をそのまま main にしない"));
    assert!(board_prompt.contains("基礎概念 → 中核メカニズム/歴史的展開 → 現代的課題"));
    assert!(board_prompt.contains("背景/問題 → 解決策 → 実装/応用 → 帰結/限界"));
    assert!(board_prompt.contains("同一の発展脈絡"));
    assert!(board_prompt.contains("branch 間 edge"));
    assert!(board_prompt.contains("内容タイプ別の骨格選択（構造パターン庫）"));
    for marker in [
        "討論・ディベート・賛否検討",
        "論題 main -> 肯定側/否定側",
        "理由ノードを方法論 main 配下に置かない",
        "比較・対照",
        "ケース分析・事例紹介",
        "問題解決・政策/制度検討",
        "因果メカニズム・理論説明",
        "分類・体系整理",
        "手順・プロセス・歴史展開",
        "資料・文献・テキスト読解",
        "データ・統計・図表解釈",
        "Q&A・相談・個別指導",
        "発表・作品・提出物への講評",
        "研究指導・レポート相談",
        "実習・演習・ワークショップ",
        "語学・表現練習",
        "意思決定・計画立案",
        "ブレインストーミング・アイデア整理",
        "物語・出来事の整理",
        "連絡・運営・予定調整",
        "雑談・反応・メタコメント",
    ] {
        assert!(board_prompt.contains(marker), "missing marker: {marker}");
    }
    assert!(board_prompt.contains("混在タイプの分離"));
    assert!(board_prompt.contains("正文内容"));
    assert!(board_prompt.contains("何について学んでいるか"));
    assert!(board_prompt.contains("どう扱っているか"));
    assert!(board_prompt.contains("正文 main"));
    assert!(board_prompt.contains("活動 main"));
    assert!(board_prompt.contains("判断手順"));
    assert!(board_prompt.contains("既存ノードの更新で十分な材料"));
    assert!(board_prompt.contains("既存ノードでは表せない新材料だけ"));
    assert!(board_prompt.contains("parent_id が意味上の所属先"));
    assert!(board_prompt.contains("内容本体の理由・根拠・事例"));
    assert!(board_prompt.contains("ノード総数に上限はない"));
    assert!(board_prompt.contains("既存情報は原則すべて引き継ぐ"));
    assert!(board_prompt.contains("nodes 配列の長さは、直前ボードの長さ「以上」を基本"));
    assert!(board_prompt.contains("重複・STT 誤認識"));
    assert!(board_prompt.contains("既存ノードを更新しない"));
    assert!(board_prompt.contains("アップグレードしてよい"));
    assert!(board_prompt.contains("古い id 固執より"));
    assert!(board_prompt.contains("前区間の内容"));
    assert!(board_prompt.contains("話題境界"));
    assert!(board_prompt.contains("別素材/会話/教材/議題"));
    assert!(board_prompt.contains("既存話題の続きなら"));
    assert!(board_prompt.contains("散らばった点"));
    assert!(board_prompt.contains("合理的な上位 main を合成"));
    assert!(board_prompt
        .contains("新しい語・固有名詞・出来事・数値・属性だけを理由に main を増やさない"));
    assert!(board_prompt.contains("branch が別 branch の下位要素に見える場合"));
    assert!(board_prompt.contains("中心放射に見せるためだけに hub を選ばない"));
    assert!(board_prompt.contains("発話順より概念上の前提→展開→帰結を優先する"));
    assert!(board_prompt.contains("構造ノード"));
    assert!(board_prompt.contains("node_type=\"term\""));
    assert!(board_prompt.contains("用語ノード"));
    assert!(board_prompt.contains("source_excerpt に根拠となる短い発話断片"));
    assert!(board_prompt.contains("source_excerpt をまとめて空にしない"));
    assert!(board_prompt.contains("人物名・地名・組織名・道具名"));
    assert!(board_prompt.contains("parent_id=\"\""));
    assert!(board_prompt.contains("用語ノードの edge は親構造ノードとだけ"));
    assert!(board_prompt.contains("外すと構造理解が悪くなる関係だけ"));
    assert!(board_prompt.contains("最終セルフチェック"));
    assert!(board_prompt.contains("混在内容では"));
    assert!(board_prompt.contains("branch 同士の下位関係は edge.label"));
    assert!(board_prompt.contains("result 同士"));
    assert!(board_prompt.contains("非空 edge.label 也必须使用简体中文"));
    assert!(!board_prompt.contains("ゲーム"));
    assert!(!board_prompt.contains("動画"));
    assert!(!board_prompt.contains("実況"));

    let overall_prompt = live_overall_system_prompt("zh", language_hint, false);
    assert!(overall_prompt.contains("### 整体总结"));
    assert!(overall_prompt.contains("### 本次论点"));

    let todo_prompt = live_todo_system_prompt("zh");
    assert!(todo_prompt.contains("title、note、source_excerpt 使用简体中文"));
    assert!(todo_prompt.contains("content_type\":\"課題|レポート|予習|復習|テスト準備|その他"));
}

#[test]
fn free_note_prompts_do_not_dismiss_non_lecture_content() {
    let language_hint = live_reply_language_hint("zh");
    let chunk_prompt = live_chunk_system_prompt(language_hint, true);
    assert!(chunk_prompt.contains("自由ノートは講義とは限りません"));
    assert!(chunk_prompt.contains("非学術的という理由だけで「整理対象外」にしない"));
    assert!(chunk_prompt.contains("人物関係"));
    assert!(chunk_prompt.contains("一度だけ出た固有名詞"));
    assert!(chunk_prompt.contains("録音内だけで十分理解できる名前や固有設定"));
    assert!(!chunk_prompt.contains("ゲーム"));
    assert!(!chunk_prompt.contains("動画"));
    assert!(!chunk_prompt.contains("実況"));

    let board_prompt =
        live_whiteboard_system_prompt(live_whiteboard_language_instruction("zh"), true);
    assert!(board_prompt.contains("録音内容"));
    assert!(board_prompt.contains("録音開始から現在まで"));
    assert!(board_prompt.contains("基礎概念 → 中核メカニズム/歴史的展開 → 現代的課題"));
    assert!(board_prompt.contains("録音の主要話題・場面・観点"));
    assert!(board_prompt.contains("整理対象外にしない"));
    assert!(board_prompt.contains("source_type=\"lecture\" は互換性のための列挙値"));
    assert!(board_prompt.contains("散らばった点"));
    assert!(board_prompt.contains("反応"));
    assert!(!board_prompt.contains("講義開始"));
    assert!(!board_prompt.contains("講義の流れ"));
    assert!(!board_prompt.contains("講義・録音全体"));
    assert!(!board_prompt.contains("講義の主要課題・章・観点"));

    let overall_prompt = live_overall_system_prompt("zh", language_hint, true);
    assert!(overall_prompt.contains("自由ノート録音"));
    assert!(overall_prompt.contains("非学術的という理由だけで除外しない"));
}
