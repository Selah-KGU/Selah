use super::super::*;

/// Extract a knowledge-point checklist from one Live note (cheap AI pass).
/// Live notes typically already contain list-like structures (「今日のポイント」/
/// 「まとめ」/「要点」/箇条書き) — the prompt is wired to USE THEM FIRST and
/// only then augment from prose. The prompt still asks for 12–20 points; the
/// hard floor is `KNOWLEDGE_FLOOR`, and `gen_targets` scales a thinner chapter
/// down to fit whatever the note actually yielded.
pub(crate) async fn extract_knowledge_points(
    course_key: &str,
    course_name: &str,
    live_topic_hint: &str,
    live_content: &str,
) -> Result<Vec<KnowledgePoint>, String> {
    let cfg = crate::ai::load_ai_config();
    if !cfg.ai_enabled {
        return Err(
            "AI is disabled. Knowledge-point extraction requires an AI provider.".to_string(),
        );
    }
    let provider = crate::agent_provider::AgentProvider::resolve()
        .map_err(|e| format!("AI provider unavailable: {e}"))?;

    let user_prompt = format!(
        r#"Course: {course}
{topic_line}
═══ LIVE LECTURE NOTE (full body) ═══
{content}

═══ TASK ═══
この講義から testable な知識点を網羅的に抽出してください。

抽出の手順:
1. 本文中に既に「今日のポイント」「まとめ」「要点」「ねらい」「キーワード」のような節、または箇条書き（・/-/●/1./2./など）がある場合、それらを **最優先のヒント** として拾う。教員自身が抽出してくれた知識点リストだから。
2. その後、本文の散文部分から、定義・例・対比・分類・数値・固有概念・名前付きプロセス・教員が強調した説明をさらに加える。
3. 事務連絡（提出方法・教室・出欠など）は除外。

各知識点には:
- `id`: "k1", "k2", ... と連番。
- `label`: 8–30字の日本語で「何の論点か」を端的に。
- `gist`: 1文で「学習者が必ず分かるべき核心」を述べる。
- `mustCover`: 教員が中心として扱った概念・定義・分類・代表例 → true。周辺・補助・余談的なら false。

合計 **12〜20件** 出してください（章生成側は 10件以上の被覆を要求する仕様なので、必ず余裕を持って）。
出力は単一の JSON のみ、コードフェンスや前置きなし:

{{
  "points": [
    {{ "id": "k1", "label": "…", "gist": "…", "mustCover": true }},
    {{ "id": "k2", "label": "…", "gist": "…", "mustCover": false }}
  ]
}}"#,
        course = course_name,
        topic_line = if live_topic_hint.is_empty() {
            String::new()
        } else {
            format!("授業計画該当回の topic: {live_topic_hint}\n")
        },
        content = live_content,
    );
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".to_string(),
            content: "あなたは大学講義のライブメモから testable 知識点を抽出する専門家。出力は単一の JSON オブジェクトのみ。前置き・コードフェンス・余分なテキスト禁止。"
                .to_string(),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
            images: Vec::new(),
        },
    ];
    eprintln!(
        "[detective] knowledge-point extract dispatching (max_tokens={})",
        cfg.max_tokens
    );
    let raw = provider
        .plan(
            messages,
            cfg.max_tokens,
            0.3,
            "",
            10,
            &format!("detective-knowledge:{course_key}"),
        )
        .await
        .map_err(|e| {
            eprintln!("[detective] knowledge extract FAILED: {}", e);
            format!("AI call failed: {e}")
        })?;
    eprintln!("[detective] knowledge extract raw ===\n{}\n=== END", raw);

    #[derive(Deserialize)]
    struct PointsEnvelope {
        points: Option<Vec<KnowledgePointDraft>>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct KnowledgePointDraft {
        id: Option<String>,
        label: Option<String>,
        gist: Option<String>,
        must_cover: Option<bool>,
    }

    let json = extract_json_object(&raw).ok_or_else(|| {
        let preview = truncate_chars(raw.trim(), 300);
        format!("Knowledge extraction returned non-JSON. Begins with: \"{preview}\"")
    })?;
    let envelope: PointsEnvelope = serde_json::from_str(&json)
        .map_err(|e| format!("Knowledge points JSON parse failed: {e}"))?;
    let drafts = envelope.points.unwrap_or_default();

    let mut out: Vec<KnowledgePoint> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for (i, d) in drafts.into_iter().enumerate() {
        let label = d.label.unwrap_or_default().trim().to_string();
        if label.is_empty()
            || looks_like_metadata_leak(&label)
            || looks_like_admin_trivia(&label)
            || looks_like_generic_label(&label)
        {
            continue;
        }
        let id =
            d.id.filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("k{}", i + 1));
        if !seen_ids.insert(id.clone()) {
            continue;
        }
        let gist = d
            .gist
            .unwrap_or_default()
            .trim()
            .chars()
            .take(160)
            .collect::<String>();
        out.push(KnowledgePoint {
            id,
            label: label.chars().take(60).collect(),
            gist,
            must_cover: d.must_cover.unwrap_or(false),
        });
    }
    if out.len() < KNOWLEDGE_FLOOR {
        return Err(format!(
            "Knowledge extraction yielded only {} usable points; need at least {KNOWLEDGE_FLOOR}",
            out.len(),
        ));
    }
    eprintln!(
        "[detective] knowledge extract ACCEPTED: {} points ({} must-cover)",
        out.len(),
        out.iter().filter(|p| p.must_cover).count()
    );
    Ok(out)
}

/// Load or build (cache-through) the knowledge-point checklist for one Live note.
pub(crate) async fn ensure_knowledge_points(
    db: &Database,
    course: &DetectiveCourse,
    live_id: &str,
    topic_hint: &str,
) -> Result<Vec<KnowledgePoint>, String> {
    if let Some(existing) = load_knowledge_points(db, &course.key, live_id) {
        if existing.len() >= KNOWLEDGE_FLOOR {
            return Ok(existing);
        }
    }
    let record = course
        .live_records
        .iter()
        .find(|r| r.id == live_id)
        .ok_or_else(|| "そのライブメモが見つかりませんでした。".to_string())?;
    let content = if record.excerpt.trim().is_empty() {
        "(本文未抽出)".to_string()
    } else {
        record.excerpt.clone()
    };
    let pts = extract_knowledge_points(&course.key, &course.name, topic_hint, &content).await?;
    save_knowledge_points(db, &course.key, live_id, &pts);
    Ok(pts)
}
