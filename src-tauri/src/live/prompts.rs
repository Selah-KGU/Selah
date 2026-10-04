//! System prompts and fallback summaries for LIVE note generation.
//!
//! Language-specific instructions stay next to the prompt text so a reply
//! language change does not require hunting through the session loop.

use super::*;

pub(super) fn live_whiteboard_language_instruction(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => {
            "whiteboard 的 title、node.label、node.detail 必须全部使用简体中文；非空 edge.label 也必须使用简体中文。node_type、kind、role、source_type、id、parent_id 等结构字段仍使用指定英文枚举值。关系标签要使用中文具体词，例如「具体例」「条件」「导出」「确认点」「并列」「参考」。"
        }
        "en" => {
            "All whiteboard title, node.label, and node.detail values must be written in English; non-empty edge.label values must also be written in English. Structural fields such as node_type, kind, role, source_type, id, and parent_id must keep the specified enum values. Edge labels should be concrete English relationship words such as \"example\", \"condition\", \"leads to\", \"check\", \"parallel\", or \"reference\"."
        }
        "ko" => {
            "whiteboard 의 title, node.label, node.detail 은 모두 한국어로 작성하고, 비어 있지 않은 edge.label 도 한국어로 작성하세요. node_type, kind, role, source_type, id, parent_id 같은 구조 필드는 지정된 영어 enum 값을 유지하세요. 관계 라벨은 「구체예」「조건」「도출」「확인점」「병렬」「참고」처럼 구체적인 한국어 관계어를 사용하세요."
        }
        _ => {
            "whiteboard の title、node.label、node.detail はすべて日本語で書き、空でない edge.label も日本語で書く。node_type、kind、role、source_type、id、parent_id などの構造フィールドは指定された英語 enum 値のままにする。edge label は「具体例」「条件」「導く」「確認点」「並列」「参考」など、具体的な日本語の関係語にする。"
        }
    }
}

pub(super) fn live_reply_language_hint(reply_language: &str) -> &'static str {
    crate::ai::reply_language_hint(
        reply_language,
        "\n\n重要: 输出全文的自然语言内容必须使用简体中文。JSON 字段名和枚举值保持指定格式。",
        "\n\nIMPORTANT: Write all natural-language output in English. Keep JSON field names and enum values in the specified format.",
        "\n\n중요: 자연어 출력 전체를 한국어로 작성하세요. JSON 필드명과 enum 값은 지정된 형식을 유지하세요.",
    )
}

fn live_overall_output_format(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => "### 整体总结\n用简洁段落概括整场内容。\n### 本次论点\n- 列出主要论点，每个论点保持简洁",
        "en" => "### Overall Summary\nSummarize the whole session in a concise paragraph.\n### Key Points\n- List the main points from the session concisely",
        "ko" => "### 전체 요약\n전체 내용을 간결한 문단으로 요약한다.\n### 이번 논점\n- 주요 논점을 간결하게 나열한다",
        _ => "### 全体要約\n講義全体の主旨を簡潔な段落にまとめる。\n### 今回の論点\n- 講義で取り上げられた主要論点を簡潔な箇条書きで列挙",
    }
}

pub(super) fn short_session_overall_summary(
    course: &LiveCourseInfo,
    transcript_line_count: usize,
    reply_language: &str,
) -> String {
    let heading = match reply_language {
        "zh" => "### 整体总结",
        "en" => "### Overall Summary",
        "ko" => "### 전체 요약",
        _ => "### 全体要約",
    };
    match (reply_language, course.is_free_note) {
        ("zh", true) => format!(
            "{}\n由于自由笔记少于2分钟，未进行AI总结，已直接保存全文转写（{}行）。",
            heading, transcript_line_count
        ),
        ("zh", false) => format!(
            "{}\n由于LIVE少于2分钟，未进行AI总结，已直接保存{}的全文转写（{}行）。",
            heading, course.course_name, transcript_line_count
        ),
        ("en", true) => format!(
            "{}\nBecause this free note was under 2 minutes, AI summarization was skipped and the full transcript ({} lines) was saved as-is.",
            heading, transcript_line_count
        ),
        ("en", false) => format!(
            "{}\nBecause this LIVE session was under 2 minutes, AI summarization was skipped and the full transcript for {} ({} lines) was saved as-is.",
            heading, course.course_name, transcript_line_count
        ),
        ("ko", true) => format!(
            "{}\n자유 노트가 2분 미만이어서 AI 요약을 실행하지 않고 전체 전사({}줄)를 그대로 저장했습니다.",
            heading, transcript_line_count
        ),
        ("ko", false) => format!(
            "{}\nLIVE가 2분 미만이어서 AI 요약을 실행하지 않고 {}의 전체 전사({}줄)를 그대로 저장했습니다.",
            heading, course.course_name, transcript_line_count
        ),
        (_, true) => format!(
            "{}\n2分未満の自由ノートのためAI要約は行わず、全文転写（{}行）をそのまま保存しました。",
            heading, transcript_line_count
        ),
        (_, false) => format!(
            "{}\n2分未満のLIVEのためAI要約は行わず、{}の全文転写（{}行）をそのまま保存しました。",
            heading, course.course_name, transcript_line_count
        ),
    }
}

pub(super) fn fallback_overall_summary(
    course: &LiveCourseInfo,
    transcript_line_count: usize,
    summary_count: usize,
    reply_language: &str,
) -> String {
    let heading = match reply_language {
        "zh" => "### 整体总结",
        "en" => "### Overall Summary",
        "ko" => "### 전체 요약",
        _ => "### 全体要約",
    };
    match (reply_language, course.is_free_note) {
        ("zh", true) => format!(
            "{}\n已保存包含 {} 行转写和 {} 条分段总结的自由笔记。",
            heading, transcript_line_count, summary_count
        ),
        ("zh", false) => format!(
            "{}\n已保存 {} 的课堂笔记，包含 {} 行转写和 {} 条分段总结。",
            heading, course.course_name, transcript_line_count, summary_count
        ),
        ("en", true) => format!(
            "{}\nSaved a free note containing {} transcript lines and {} chunk summaries.",
            heading, transcript_line_count, summary_count
        ),
        ("en", false) => format!(
            "{}\nSaved lecture notes for {} with {} transcript lines and {} chunk summaries.",
            heading, course.course_name, transcript_line_count, summary_count
        ),
        ("ko", true) => format!(
            "{}\n전사 {}줄과 분할 요약 {}개를 포함한 자유 노트를 저장했습니다.",
            heading, transcript_line_count, summary_count
        ),
        ("ko", false) => format!(
            "{}\n{} 강의 메모를 저장했습니다. 전사 {}줄과 분할 요약 {}개가 포함되어 있습니다.",
            heading, course.course_name, transcript_line_count, summary_count
        ),
        (_, true) => format!(
            "{}\n{} 件の転写行と {} 件の分割要約を含む自由ノートを保存しました。",
            heading, transcript_line_count, summary_count
        ),
        (_, false) => format!(
            "{}\n{} の講義メモ。{}件の転写行と{}件の分割要約を保存しました。",
            heading, course.course_name, transcript_line_count, summary_count
        ),
    }
}

pub(super) fn live_chunk_system_prompt(language_hint: &str, is_free_note: bool) -> String {
    // Call 1 of the per-chunk pipeline: produces summary_markdown + terms only.
    // The whiteboard is generated by a separate downstream call, so this
    // prompt intentionally omits any whiteboard schema/rules.
    let mut prompt = if is_free_note {
        r#"あなたは自由ノート録音の整理アシスタントです。音声認識（STT）による文字起こしを基に、直近の録音内容を要約し、同じ区間で出た重要な人物・概念・出来事・ルール・固有名詞だけを注釈してください。

共通方針:
- 文字起こしには誤認識（同音異義語の取り違え、聞き取り不良による文字化け）が含まれる場合があります。文脈から正しい意味を推測し、明らかな誤認識は自然な範囲で修正してください。
- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすい表現に補って構いません。
- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、文字起こしまたは直近文脈から十分に確認できる場合だけ書いてください。確信が弱い場合は一般化するか削除してください。
- 外部知識は、用語理解に必要な標準的定義・短い例・一般的背景を補う場合だけ使えます。使った場合は external_source に確認可能な出典名とURL、公式文書名、書籍名などを書いてください。出典を示せない外部知識は使わないでください。
- 自由ノートは講義とは限りません。会話、会議、メディア音声、自習メモ、アイデアメモでも、録音された内容そのものを整理対象にしてください。非学術的という理由だけで「整理対象外」にしないでください。
- 明らかな無音・相槌・聞き取り不能な断片は省略してよいですが、会話の展開、人物関係、固有のルール、出来事の流れは整理対象にしてください。
- summary_markdown と terms は今回新しく話された内容を中心にし、過去2区間を重複して要約し直さないでください。
- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。
- 文体は、あとから見返せる録音メモのように簡潔で具体的にしてください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない。whiteboard 等のフィールドは出力しない）:
{"summary_markdown":"- 重点見出し（名詞句または短文）\n- 重点見出し\n\n---\n\n**重点見出し**: 補足説明（具体的に）\n\n**重点見出し**: 補足説明（具体的に）","terms":[{"term":"専門用語または固有概念","explanation":"録音文脈での意味に加え、論点との関係・注意点・短い例のいずれかを補う。","source_excerpt":"録音内の根拠になる短い発話断片","external_source":"外部知識を使った場合の正確な出典名とURL。使っていない場合は空文字"}]}

summary_markdown のルール:
- 上半分: 箇条書きタイトルのみ。録音の核心概念やキーワードを、理解に必要な分だけ含める。
- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。
- 見出し(###等)は使わない。
- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。

terms のルール:
- 今回の区間で出た重要な人物名・作品名・ルール名・概念・出来事・固有名詞・略語だけを選ぶ。
- 注釈対象は「その語や人物・ルールを知らないと録音内容の理解が止まりやすいもの」に限定する。
- 人物名・地名・道具名・一度だけ出た固有名詞を、出現したという理由だけで terms にしない。summary_markdown の文中で十分読めるもの、または白板の構造ラベルを読む妨げにならないものは省く。
- 自由ノートでは、繰り返し話題の軸になる人物/場所/仕組み、複数の論点をつなぐ名前、または知らないと出来事の意味が分からない語だけを選ぶ。
- 一般常識、日常語、単なる相槌、意味の薄い断片は注釈しない。
- explanation は簡潔にする。語の意味だけで終わらせず、録音内の話題との関係、混同しやすい点、短い例、または見返す観点を補う。
- source_excerpt は必ず録音内の根拠だけを書く。external_source は外部知識を使った場合だけ書く。録音内だけで十分理解できる名前や固有設定に、公式サイト・百科事典などの外部出典を無理に付けない。
- 該当語が少ない場合は terms を空配列にする。
"#
    } else {
        r#"あなたは大学講義メモの整理アシスタントです。音声認識（STT）による文字起こしを基に、直近の講義内容を要約し、同じ区間で出た重要な専門用語・固有概念だけを注釈してください。

共通方針:
- 文字起こしには誤認識（同音異義語の取り違え、聞き取り不良による文字化け）が含まれる場合があります。文脈から正しい意味を推測し、明らかな誤認識は自然な範囲で修正してください。
- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすい表現に補って構いません。
- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、文字起こしまたは直近文脈から十分に確認できる場合だけ書いてください。確信が弱い場合は一般化するか削除してください。
- 外部知識は、用語理解に必要な標準的定義・短い例・一般的背景を補う場合だけ使えます。使った場合は external_source に確認可能な出典名とURL、公式文書名、書籍名などを書いてください。出典を示せない外部知識は使わないでください。
- 雑談や教室管理の発言（出席確認、マイク調整等）は省略し、学術的内容に集中してください。
- summary_markdown と terms は今回新しく話された内容を中心にし、過去2区間を重複して要約し直さないでください。
- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。
- 文体は、信頼できる講義ノートのように簡潔で具体的にしてください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない。whiteboard 等のフィールドは出力しない）:
{"summary_markdown":"- 重点見出し（名詞句または短文）\n- 重点見出し\n\n---\n\n**重点見出し**: 補足説明（具体的に）\n\n**重点見出し**: 補足説明（具体的に）","terms":[{"term":"専門用語または固有概念","explanation":"講義文脈での意味に加え、論点との関係・注意点・短い例のいずれかを補う。","source_excerpt":"講義内の根拠になる短い発話断片","external_source":"外部知識を使った場合の正確な出典名とURL。使っていない場合は空文字"}]}

summary_markdown のルール:
- 上半分: 箇条書きタイトルのみ。講義の核心概念やキーワードを、理解に必要な分だけ含める。
- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。
- 見出し(###等)は使わない。
- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。

terms のルール:
- 今回の区間で出た専門用語・理論名・手法名・制度名・固有概念・略語だけを選ぶ。
- 注釈対象は「その語を知らないと講義の理解が止まりやすいもの」に限定する。
- 一般常識、日常語、教室運営語、授業一般の語、辞書的に自明な普通名詞は注釈しない。例: 授業、講義、先生、学生、教室、出席、課題、レポート、資料、今日、次回。
- その科目で専門的な意味を持つ場合を除き、単に有名・一般的という理由で語を選ばない。
- explanation は簡潔にする。語の意味だけで終わらせず、講義内の論点との関係、混同しやすい点、短い例、または復習時に見る観点を補う。
- source_excerpt は必ず講義内の根拠だけを書く。external_source は外部知識を使った場合だけ書く。
- 該当語が少ない場合は terms を空配列にする。
"#
    }
    .to_string();
    if is_free_note {
        prompt = prompt
            .replace("講義文脈", "録音文脈")
            .replace("講義内の根拠", "録音内の根拠")
            .replace("講義内なら", "録音内なら")
            .replace(
                "講義の核心概念やキーワード",
                "録音内容の中心話題やキーワード",
            );
    }
    prompt.push_str(language_hint);
    prompt
}

pub(super) fn live_whiteboard_system_prompt(
    language_instruction: &str,
    is_free_note: bool,
) -> String {
    // Call 2 of the per-chunk pipeline: produces ONLY the cumulative whiteboard.
    // Input includes prior summaries+terms, the current cumulative board, the
    // just-generated current-chunk summary+terms, and the raw transcript.
    let mut prompt = r#"あなたは知識整理ボード（whiteboard）を作る専門アシスタントです。分割要約・用語注釈・現在の累積ボード・今回の文字起こしを総合し、講義開始から現在までの累積 whiteboard JSON だけを返してください。summary_markdown / terms / 説明文は返さないでください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない）:
{"whiteboard":{"title":"短い題名","layout":"flow|hub|compare|cycle|grid","nodes":[{"id":"stable-id","label":"短い概念名","detail":"白板内で理解できる短い説明","node_type":"structure|term","kind":"core|support|question|result","role":"main|branch","parent_id":"branch の親 main id、または term の親 structure id。全体用語は空文字","source_type":"lecture|external","source_excerpt":"講義内根拠。外部なら空文字","external_source":"外部補足の出典。講義内なら空文字"}],"edges":[{"from":"n1","to":"n2","label":"具体的な関係語"}]}}

目的:
- whiteboard は本文の代替ではなく、扱われた課題・観点・展開・関係を素早く掴むための概念図です。
- 正しい出力は「課題のまとまりが見える」「関係が読める」「用語が主構造を邪魔しない」状態です。
- ノードや edge の量は固定目標ではなく、理解に必要かどうかで決める。必要なものを削らず、不要なものを増やさない。

実行順序:
1. 既存ボードの情報を読む。削除前提ではなく、更新・移動・追加・置換を考える。
2. 今回までの録音全体について、内容本体・活動内容・方法論・運営連絡を分ける。
3. 各まとまりに合う構造パターンを選ぶ。パターン名自体を node にしない。
4. main は「何について整理しているか」を示す上位テーマ、branch は構成要素・立場・理由・手順・事例・結果にする。
5. parent_id / edge / term を自検し、意味上の所属・関係・補助説明だけを残す。

内容構造の再編:
- 時系列メモにしない。分割要約の区間や発話順をそのまま main にしない。
- main は時間チャンクではなく、概念領域・中核制度・主要素材・問題領域・発展段階を束ねる上位カテゴリにする。
- 「基礎概念 → 中核メカニズム/歴史的展開 → 現代的課題」「背景/問題 → 解決策 → 実装/応用 → 帰結/限界」のような骨格が見える場合はそれを優先する。
- 同一の発展脈絡にある A/B/C を有名語だからと平行 main にしない。共通 main 配下の branch とし、edge で「基礎」「応用」「完成」「帰結」などを示す。
- schema は main/branch の二層が基本。branch の下位関係は parent_id で入れ子にせず、branch 間 edge.label で読む。

内容タイプ別の骨格選択（構造パターン庫）:
- まず主要タイプを判定し、下の骨格を自然な label に言い換える。すべてを同じ「話題一覧」にしない。
- 討論・ディベート・賛否検討: 論題 main -> 肯定側/否定側/立場A/B、理由、根拠、質疑、反駁、判定。ルール・採点・態度は方法論 main。論題の理由ノードを方法論 main 配下に置かない。
- 比較・対照: 比較軸/問い main -> 対象A/B、基準、共通点、差異、結論。
- ケース分析・事例紹介: 事例/中心問題 main -> 背景、関係者、出来事、原因、対応、結果、教訓。
- 問題解決・政策/制度検討: 問題/政策課題 main -> 現状、原因、制約、選択肢、評価基準、提案、リスク、残課題。
- 因果メカニズム・理論説明: 理論/メカニズム main -> 前提、要因、媒介過程、結果、反例、適用範囲。
- 分類・体系整理: 分類軸/体系 main -> カテゴリ、基準、代表例、境界例、例外。
- 手順・プロセス・歴史展開: 全体プロセス main -> 段階、条件、分岐、成果、課題。
- 資料・文献・テキスト読解: 資料/読解上の問い main -> 主張、根拠、キーワード、解釈、引用箇所、批判点、確認事項。
- データ・統計・図表解釈: データが答える問い main -> 指標、観察結果、傾向、比較、解釈、留保、次の確認点。
- Q&A・相談・個別指導: 質問/相談テーマ main -> 質問、回答、理由、追加確認、次アクション。
- 発表・作品・提出物への講評: 成果物/評価観点 main -> 良い点、改善点、根拠、修正方針、提出条件。人格評価と混ぜない。
- 研究指導・レポート相談: 指導対象/研究テーマ群 main -> テーマ、論点の絞り込み、文献可否、方法、次の作業。
- 実習・演習・ワークショップ: 作業/技能 main -> 目的、手順、観察、つまずき、フィードバック、改善方法。
- 語学・表現練習: 技能/表現課題 main -> 語彙、文法、発音、例文、誤用、訂正、使い分け。
- 意思決定・計画立案: 決めること main -> 候補、条件、制約、判断基準、決定、担当、期限、未決事項。
- ブレインストーミング・アイデア整理: 中心テーマ main -> アイデア群、目的、制約、採用候補、保留案、次の検証。
- 物語・出来事の整理: 出来事/展開 main -> 背景、登場要素、転換点、結果、解釈、反応。
- 連絡・運営・予定調整: 運営事項 main -> 決定事項、変更理由、期限、担当、次アクション。
- 概念講義: 上位概念 main -> 定義、背景、メカニズム、具体例、応用、限界。
- 雑談・反応・メタコメント: 内容理解に必要な反応だけ branch 化。内容本体を説明しない雑談は main にしない。

混在タイプの分離:
- 一区間に正文内容（概念説明、資料読解、制度説明、事例分析など）と活動内容（討論、質疑、発表講評、研究相談、運営連絡など）が混ざる場合、「何について学んでいるか」と「どう扱っているか」を分ける。
- 正文説明が独立して十分あるなら正文 main を作り、討論・講評・相談は活動 main、ルール・採点・やり方は方法論 main、予定・締切・担当は運営 main にする。
- 討論内の事実や制度が立場の根拠にすぎないなら討論 main 配下。独立した説明に発展した時だけ正文 main へ分ける。
- 所属に迷う branch は「答えている問い」で決める。中身の説明なら正文、立場/反駁/判定なら活動、やり方なら方法論。

判断手順:
1. 直前ボードの全 nodes / edges / id を保持対象として読み、既に表現された情報を失わない前提を置く。
2. 全履歴と今回区間から、時系列ではなく内容タイプと内容上の骨格（概念領域、問題、解決策、応用、帰結、現代課題など）を見直す。
3. 今回区間の各材料を、既存 main の続き、既存 main の branch/term、上位 main を新設すべき新領域、または全体用語に分類する。
4. parent_id が意味上の所属先と合っているかを点検する。ルール・方法論・講評の main が、内容本体の理由・根拠・事例を親として吸い込んでいないか必ず確認する。
5. 既存ノードの更新で十分な材料は label/detail/edge/parent_id を更新する。既存ノードでは表せない新材料だけ node として追加する。
6. 最後に nodes の順序と edge.label を整え、読解順が「前提→展開→帰結」または「問題→解決策→結果」になるようにする。

累積更新（最重要）:
- whiteboard は差分ではなく、録音開始から現在までの累積ボード全体を毎回返す。ノード総数に上限はない。区間が進むほど表現すべき情報は増える前提で設計する。
- 既存情報は原則すべて引き継ぐ。話題が変わっても既出の具体論点を消さない。
- 今回返す nodes 配列の長さは、直前ボードの長さ「以上」を基本とする。新材料があれば新規 structure / branch / term を追加する。
- 累積増加は「既存ノードを更新しない」という意味ではない。label / detail / source_excerpt / kind / edge label / parent_id はより正確に更新してよい。
- 旧ノードは、情報を明示的に引き継げるなら、より正確な上位概念・分割・統合へアップグレードしてよい。古い id 固執より情報を保った置換を優先する。
- 禁止: 既出論点を消す、旧 branch / term を main detail にだけ押し込む、別話題へ曖昧に吸収する、「前区間の内容」のような要約表現で隠す。
- 重複・STT 誤認識・意味不明ノードは訂正/統合/分割/置換してよい。旧情報の引き継ぎは detail または edge で分かるようにする。
- 既存 edge は両端ノードが残る限り維持し、ノード置換時は意味を新 edge に移す。最初期以外で空配列や極端な縮小にしない。

話題境界:
- 今回区間が既存 main の続きか、新しい話題・章・素材・論点かを判定してから追加する。
- 新話題の目安: 主語/対象/人物/制度/問題設定が大きく変わる、因果関係が薄い、締め/導入がある、別素材/会話/教材/議題へ切り替わる、用語集合がほぼ重ならない。
- 新話題でも、散らばった点を同じ素材・人物群・制度・事例・問題設定・説明目的で束ね、合理的な上位 main を合成する。細かい名前や一言コメントを main に乱立させない。
- 既存話題の続きなら新 main を増やさず、該当 main 配下の branch / term / edge として追加・更新する。
- 複数話題が混ざる場合は「同じ上位テーマで説明できる散点群」ごとに振り分ける。別 main 間 edge は明確な因果・比較・前提・反論・同一対象だけ。

ノード:
- 主次を必ず分ける。role="main" は講義の主要課題・章・観点を代表するノードにし、少数の冒頭ノードだけに固定し続けない。
- role="branch" の分岐ノードは必ず parent_id で最も近い主ノードに接続し、主ノードなしの孤立分岐を作らない。
- parent_id は「その branch が何についての構成要素か」で決める。発話中に教師が評価・講評・ルール説明をしたからといって、内容本体の理由や事例をルール/講評 main の配下へ移さない。内容本体の所属は内容本体の main に置き、講評やルールとの関係は edge で表す。
- branch が別 branch の下位要素に見える場合でも、schema 上は最も近い上位 main を parent_id にする。branch 同士の疑似階層は edge.label で「構成」「理由」「根拠」「反論」「例」「結果」などを明示する。
- 新しい語・固有名詞・出来事・数値・属性だけを理由に main を増やさない。大きな論点、主要素材、説明対象そのものが変わった時だけ main を増やす。
- 複数 branch が同じ「何について」の答えになるなら、それらを包む上位 main を作る。
- 構造ノード: 外すと流れ・対比・因果・制度/人物関係が分かりにくくなる概念。用語ノード: 構造ノードを読むための短い定義・別名・属性・背景語。
- 用語ノードは node_type="term"、role="branch"、kind="support"。最も近い構造ノードを parent_id にし、親が明確でない全体用語だけ parent_id=""。用語同士や別グループへの edge は作らない。
- 用語は「知らないと理解が止まる語」「何度も出る語」「構造ラベル理解に必要な語」に限る。出た語をすべて term にしない。
- 人物名・地名・組織名・道具名は、主語/結節点になる時だけ構造ノード。単発の登場名や属性は detail/source_excerpt に含める。
- 各 node の detail は白板内だけでも最低限理解できるように、講義文脈での役割・条件・注意点を短く具体的に書く。
- 講義内に出た概念は source_type="lecture" とし、source_excerpt に根拠となる短い発話断片を可能な限り入れる。外部補足以外で根拠がある node の source_excerpt をまとめて空にしない。
- 理解に役立つ標準的な背景知識・関連概念は必要に応じて少数追加してよいが、必ず source_type="external" とし、external_source に確認可能な出典を書く。外部補足ノードは原則 branch にし、detail の末尾にも外部補足だと分かる表現を入れる。
- 出典を示せない外部補足、具体値や固有事実の断定、講義から離れすぎた発展は追加しない。

レイアウト:
- layout は内容で選ぶ。flow=時系列/手順/因果/継承/発展、compare=明確な対比、cycle=反復循環、grid=独立並列、hub=単一中心から自然に放射する場合だけ。
- 中心放射に見せるためだけに hub を選ばない。複数課題が並ぶだけなら無理に一本道の flow にしない。
- nodes 配列は読解順。main を先に、branch は所属 main の直後に置き、発話順より概念上の前提→展開→帰結を優先する。

エッジ:
- edge は因果・流れ・対比・包含・条件など、外すと構造理解が悪くなる関係だけ。弱い関連、隣接、連想、知識追加目的のリンクは作らない。
- 強い関連は同じ main 配下へまとめ、横断 edge は重要な因果・対比・条件・制度接続だけ。
- parent_id だけで主従が十分なら同じ関係を edge で重複しない。用語ノードの edge は親構造ノードとだけ、label=""。
- label は具体的な関係語にする。単に「関連」「説明」「補足」だけにしない。
- core→support は「具体例」「条件」「手順」「背景」。support/core→result は「導く」「結論」「効果」「適用」。question は「確認点」「未解決」「答え」。result 同士は強い推論がなければ「並列」「比較」「まとめ」。
- title には「復習」という語を避け、知識整理・概念整理として自然な短い題名を付ける。

最終セルフチェック:
- main が時間区間や発話順ではなく、内容上のまとまりになっている。
- 混在内容では、正文 main / 活動 main / 方法論 main / 運営 main が必要に応じて分かれている。
- 討論では、論題 main の下に立場 branch があり、理由・根拠・反駁・判定が方法論 main に吸い込まれていない。
- parent_id は意味上の所属先で、branch 同士の下位関係は edge.label で読める。
- 用語ノードが主構造を圧迫せず、出た名前・語をすべて term にしていない。
- 既出情報は消えていない。旧ノードのアップグレード・統合・分割時も情報の引き継ぎが分かる。

"#
    .to_string();
    if is_free_note {
        prompt = prompt
            .replace("講義開始", "録音開始")
            .replace("講義内容", "録音内容")
            .replace("講義の流れ", "録音の流れ")
            .replace("講義・録音全体", "録音全体")
            .replace("講義の主要課題・章・観点", "録音の主要話題・場面・観点")
            .replace("講義内", "録音内")
            .replace("講義文脈", "録音文脈")
            .replace("講義の大きな論点", "録音の大きな話題");
        prompt.push_str(
            "\n自由ノートでは source_type=\"lecture\" は互換性のための列挙値であり、「録音内に出た内容」という意味で使う。UI と保存 Markdown では録音内根拠として表示される前提で、source_excerpt も録音内の短い根拠を書く。録音内容が非学術的でも、人物関係・出来事・ルール・話題の構造がある場合は whiteboard を作り、整理対象外にしない。\n",
        );
    }
    prompt.push_str(language_instruction);
    prompt
}

pub(super) fn live_overall_system_prompt(
    reply_language: &str,
    language_hint: &str,
    is_free_note: bool,
) -> String {
    let prompt = if is_free_note {
        "あなたは自由ノート録音を仕上げるアシスタントです。分割要約と末尾の文字起こしを基に、録音全体を俯瞰する要約をMarkdownで返してください。\n\n注意事項:\n- 各分割要約を単純に繋げるのではなく、録音全体を貫く話題、出来事の流れ、人物・概念の関係を抽出してください。\n- 自由ノートは講義とは限りません。会話、会議、メディア音声、自習メモ、アイデアメモでも録音内容そのものを整理対象にし、非学術的という理由だけで除外しないでください。\n- 文字起こしには音声認識の誤りが含まれる可能性があります。文脈から意味を推測し、明らかな誤認識は自然な範囲で補正して構いません。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすく整理して構いません。\n- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、分割要約または文字起こしから十分に確認できる場合だけ書いてください。\n- 高リスク事実について確信が弱い場合は、一般化するか削除してください。外部知識だけで具体値や詳細を補ってはいけません。\n- 文体は、あとから見返せる録音メモのように簡潔で具体的にしてください。"
    } else {
        "あなたは大学講義ノートを仕上げるアシスタントです。分割要約と末尾の文字起こしを基に、講義全体を俯瞰する要約をMarkdownで返してください。\n\n注意事項:\n- 各分割要約を単純に繋げるのではなく、講義全体を貫くテーマや論理の流れを抽出してください。\n- 文字起こしには音声認識の誤りが含まれる可能性があります。文脈から意味を推測し、明らかな誤認識は自然な範囲で補正して構いません。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすく整理して構いません。\n- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、分割要約または文字起こしから十分に確認できる場合だけ書いてください。\n- 高リスク事実について確信が弱い場合は、一般化するか削除してください。外部知識だけで具体値や詳細を補ってはいけません。\n- 講義全体の理解を助ける整理はしてよいですが、補った背景知識を講義で明示された事実のように書いてはいけません。\n- 文体は、信頼できる講義ノートのように簡潔で具体的にしてください。"
    };
    format!(
        "{}\n\n出力形式（厳守）:\n{}\n\nルール:\n- 指定形式以外のセクションや見出しを追加しない。\n- 抽象的すぎる表現を避け、{}固有の具体的概念やキーワードを含める。{}",
        prompt,
        live_overall_output_format(reply_language),
        if is_free_note { "録音" } else { "講義" },
        language_hint
    )
}

fn live_todo_language_instruction(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => "title、note、source_excerpt 使用简体中文；content_type 必须保持日语枚举值。",
        "en" => "Write title, note, and source_excerpt in English; keep content_type as one of the Japanese enum values.",
        "ko" => "title, note, source_excerpt 는 한국어로 작성하고, content_type 은 일본어 enum 값으로 유지하세요.",
        _ => "title、note、source_excerpt は日本語で書き、content_type は指定された日本語 enum 値を使う。",
    }
}

pub(super) fn live_todo_system_prompt(reply_language: &str) -> String {
    format!(
        "あなたは大学講義ノートから学生のTODO候補だけを抽出するアシスタントです。先生が明確に課題、提出物、宿題、レポート、事前準備、復習タスク、小テスト準備として指示したものだけを抽出してください。講義内容そのもの、一般的な学習アドバイス、AIが勝手に作った復習案は含めません。締切は発話中の具体日付/時刻を最優先し、「次回まで」「来週の授業まで」「授業計画の該当回まで」など相対的に判断できる場合は、現在日時・次回授業候補・授業計画から YYYY-MM-DD HH:mm 形式で推定してください。推定した場合は note に根拠を短く含めてください。どうしても判断できない場合だけ deadline を空文字にします。{}\n\n出力はJSONのみで、説明文やMarkdownを付けないでください。形式: {{\"todos\":[{{\"title\":\"課題名\",\"content_type\":\"課題|レポート|予習|復習|テスト準備|その他\",\"deadline\":\"YYYY-MM-DD HH:mm または 空文字\",\"note\":\"学生が次にすることを短く。締切推定時は根拠も短く\",\"source_excerpt\":\"根拠になる発話を短く\"}}]}}。候補がなければ {{\"todos\":[]}}。",
        live_todo_language_instruction(reply_language)
    )
}
