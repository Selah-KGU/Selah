use super::super::*;

/// Generate the campaign bible (世界観 layer) for one course. AI-required.
/// The world/era is DERIVED FROM the lecture subject matter — an American
/// Revolution course yields an 18th-century colonial setting, a statistics
/// course yields a "probability-ruled" world, etc. Generated once per course,
/// then read + advanced by each session.
pub(crate) async fn generate_campaign_bible(
    course_key: String,
    course_name: String,
    input: Vec<EvidenceInputEntry>,
) -> Result<DetectiveCampaign, String> {
    let cfg = crate::ai::load_ai_config();
    eprintln!(
        "[detective] generate_campaign_bible: course={} ai_enabled={} input_sources={}",
        course_name,
        cfg.ai_enabled,
        input.len()
    );
    if !cfg.ai_enabled {
        return Err(
            "AI is disabled. Campaign worlds require an AI provider — enable AI in Selah settings."
                .to_string(),
        );
    }
    if input.is_empty() {
        return Err(
            "この科目にはまだライブメモがありません。先にライブを記録してから世界観を生成してください。"
                .to_string(),
        );
    }

    let provider = crate::agent_provider::AgentProvider::resolve()
        .map_err(|e| format!("AI provider unavailable: {e}"))?;

    let user_prompt = campaign_bible_user_prompt(&course_name, &input);
    eprintln!(
        "[detective] campaign bible prompt ready ({} chars)",
        user_prompt.chars().count()
    );
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".to_string(),
            content: campaign_bible_system_prompt().to_string(),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
            images: Vec::new(),
        },
    ];
    eprintln!(
        "[detective] campaign bible plan() dispatching (max_tokens={})",
        cfg.max_tokens
    );
    let raw = provider
        .plan(
            messages,
            cfg.max_tokens,
            0.5,
            "",
            20,
            &format!("detective-bible:{course_key}"),
        )
        .await
        .map_err(|e| {
            eprintln!("[detective] campaign bible plan() FAILED: {}", e);
            format!("AI call failed: {e}")
        })?;
    eprintln!(
        "[detective] campaign bible raw BEGIN ===\n{}\n=== END raw response",
        raw
    );

    let json = extract_json_object(&raw).ok_or_else(|| {
        let preview = truncate_chars(raw.trim(), 300);
        format!("AI did not return JSON for the campaign bible. Begins with: \"{preview}\"")
    })?;
    let draft: CampaignBibleDraft = serde_json::from_str(&json).map_err(|e| {
        let preview = truncate_chars(json.trim(), 300);
        format!("Campaign bible JSON parse failed: {e}. JSON begins with: \"{preview}\"")
    })?;

    let world_label = draft.world_label.unwrap_or_default().trim().to_string();
    let setting = draft.setting.unwrap_or_default().trim().to_string();
    let tagline = draft.tagline.unwrap_or_default().trim().to_string();
    let meta_mystery = draft.meta_mystery.unwrap_or_default().trim().to_string();
    if world_label.is_empty() || setting.is_empty() || meta_mystery.is_empty() {
        return Err(
            "Campaign bible was missing worldLabel/setting/metaMystery — retry generation."
                .to_string(),
        );
    }
    let cast: Vec<CampaignCharacter> = draft
        .cast
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            let name = c.name.unwrap_or_default().trim().to_string();
            let role = c.role.unwrap_or_default().trim().to_string();
            if name.is_empty() || role.is_empty() {
                return None;
            }
            Some(CampaignCharacter {
                name,
                role,
                bond: c.bond.unwrap_or_default().trim().to_string(),
                background: c.background.unwrap_or_default().trim().to_string(),
                motivation: c.motivation.unwrap_or_default().trim().to_string(),
                stake: c.stake.unwrap_or_default().trim().to_string(),
                voice: c.voice.unwrap_or_default().trim().to_string(),
            })
        })
        .take(3)
        .collect();

    // Build the staged reveal arc: distribute thresholds evenly across 0–100,
    // so the final stage lands at 100 (full payoff).
    let arc_drafts: Vec<CampaignArcDraft> = draft
        .meta_arc
        .unwrap_or_default()
        .into_iter()
        .filter(|a| {
            a.reveal
                .as_deref()
                .map(|r| !r.trim().is_empty())
                .unwrap_or(false)
        })
        .take(4)
        .collect();
    let arc_len = arc_drafts.len().max(1);
    let meta_arc: Vec<CampaignRevelation> = arc_drafts
        .into_iter()
        .enumerate()
        .map(|(i, a)| {
            let stage = (i + 1) as u8;
            let threshold = (((i + 1) as f32 / arc_len as f32) * 100.0).round() as u8;
            CampaignRevelation {
                stage,
                threshold: threshold.min(100),
                title: a
                    .title
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .take(40)
                    .collect(),
                reveal: a.reveal.unwrap_or_default().trim().to_string(),
                unlocked: false,
                setup: a.setup.unwrap_or_default().trim().to_string(),
                misdirection: a.misdirection.unwrap_or_default().trim().to_string(),
                session_band: a.session_band.unwrap_or_default().trim().to_string(),
            }
        })
        .collect();

    let relationships: Vec<CampaignRelationship> = draft
        .relationships
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| {
            let from = r.from.unwrap_or_default().trim().to_string();
            let to = r.to.unwrap_or_default().trim().to_string();
            let relation = r.relation.unwrap_or_default().trim().to_string();
            if from.is_empty() || to.is_empty() || relation.is_empty() {
                return None;
            }
            Some(CampaignRelationship {
                from,
                to,
                relation,
                tension: r.tension.unwrap_or_default().trim().to_string(),
            })
        })
        .take(5)
        .collect();

    let finale = draft.finale.unwrap_or_default().trim().to_string();

    let now = crate::db::epoch_secs();
    eprintln!(
        "[detective] campaign bible ACCEPTED: world={} cast={} arc={}",
        world_label,
        cast.len(),
        meta_arc.len()
    );
    Ok(DetectiveCampaign {
        course_key,
        course_name,
        world_label,
        setting,
        tagline,
        cast,
        meta_mystery,
        meta_progress: 0,
        meta_arc,
        finale,
        chapters: Vec::new(),
        relationships,
        canon: CampaignCanon::default(),
        created_at: now,
        updated_at: now,
    })
}

/// Load the cached campaign for a course, or generate + persist a fresh one.
pub(crate) async fn ensure_campaign(
    db: &Database,
    course: &DetectiveCourse,
) -> Result<DetectiveCampaign, String> {
    if let Some(existing) = load_campaign(db, &course.key) {
        return Ok(existing);
    }
    let input = build_evidence_input(course);
    let campaign = generate_campaign_bible(course.key.clone(), course.name.clone(), input).await?;
    save_campaign(db, &campaign);
    Ok(campaign)
}
