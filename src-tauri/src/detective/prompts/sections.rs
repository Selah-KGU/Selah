use super::super::{DetectiveCampaign, DetectiveMemory};

/// Render the campaign bible into a compact prompt section. When no campaign
/// exists yet (None), the case is generated world-free.
pub(crate) fn format_campaign_section(
    campaign: Option<&DetectiveCampaign>,
    arc_focus: Option<u8>,
) -> String {
    let Some(c) = campaign else {
        return "(この科目にはまだ世界観が設定されていない。中立的な学習シーンで構成すること)"
            .to_string();
    };
    let cast = if c.cast.is_empty() {
        "(未設定)".to_string()
    } else {
        c.cast
            .iter()
            .map(|m| {
                let mut block = format!("- {}（{}）", m.name, m.role);
                if !m.bond.trim().is_empty() {
                    block.push_str(&format!(" — {}", m.bond.trim()));
                }
                if !m.background.trim().is_empty() {
                    block.push_str(&format!("\n    背景: {}", m.background.trim()));
                }
                if !m.motivation.trim().is_empty() {
                    block.push_str(&format!("\n    動機: {}", m.motivation.trim()));
                }
                if !m.stake.trim().is_empty() {
                    block.push_str(&format!("\n    利害: {}", m.stake.trim()));
                }
                if !m.voice.trim().is_empty() {
                    block.push_str(&format!("\n    口調: {}", m.voice.trim()));
                }
                block
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let cast_log = if c.canon.cast_log.is_empty() {
        "(まだなし)".to_string()
    } else {
        c.canon
            .cast_log
            .iter()
            .rev()
            .take(8)
            .map(|e| format!("- {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let played = if c.chapters.is_empty() {
        "(まだ章は進んでいない — 序章にあたる)".to_string()
    } else {
        c.chapters
            .iter()
            .rev()
            .take(5)
            .map(|ch| format!("- {}", ch.title))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let relationships = if c.relationships.is_empty() {
        "(未設定)".to_string()
    } else {
        c.relationships
            .iter()
            .map(|r| {
                let tension = if r.tension.trim().is_empty() {
                    String::new()
                } else {
                    format!("（{}）", r.tension.trim())
                };
                format!("- {} ⇄ {}: {}{}", r.from, r.to, r.relation, tension)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    // The 暗线 stage this chapter should plant toward. Prefer the chapter's own
    // arc position (`arc_focus`, derived from its place in the season) so the
    // serialized story stays coherent regardless of play order; fall back to the
    // lowest not-yet-unlocked stage when no position is supplied.
    let current_stage = match arc_focus {
        Some(n) => c
            .meta_arc
            .iter()
            .find(|r| r.stage == n)
            .or_else(|| c.meta_arc.last()),
        None => c
            .meta_arc
            .iter()
            .find(|r| !r.unlocked)
            .or_else(|| c.meta_arc.last()),
    };
    let arc_total = c.meta_arc.len().max(1);
    let arc_section = match current_stage {
        Some(stage) => {
            // Antagonist presence escalates across the season: early chapters
            // only leave traces, late chapters bring the black hand to the fore.
            let presence = match (stage.stage as usize * 4) / arc_total {
                0 => "黒幕の存在はまだ痕跡のみ（噂・物・名前が一度かすめる程度）。",
                1 => "黒幕の影が近づく（代理人や利害関係者が一人、脇で動く）。",
                2 => "黒幕の手が事件に絡む（その思惑が今回の出来事に直接影響する）。",
                _ => "黒幕（またはその代理人）が前面に出て探偵と対峙しうる段階。",
            };
            let mut block = format!(
                "今、進めるべき暗线の段階: 第{}/{}段階「{}」\n    {}",
                stage.stage, arc_total, stage.title, presence
            );
            if !stage.setup.trim().is_empty() {
                block.push_str(&format!("\n    埋めるべき布石: {}", stage.setup.trim()));
            }
            if !stage.misdirection.trim().is_empty() {
                block.push_str(&format!(
                    "\n    効かせる誤導: {}",
                    stage.misdirection.trim()
                ));
            }
            block
        }
        None => "(段階未設定 — meta-mystery を一度だけ匂わせる)".to_string(),
    };
    // Already-dropped hooks so chapters vary their hints instead of repeating.
    let dropped = if c.canon.dropped_hooks.is_empty() {
        "(まだなし)".to_string()
    } else {
        c.canon
            .dropped_hooks
            .iter()
            .rev()
            .take(8)
            .map(|h| format!("- {}", h.hook))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let facts = if c.canon.facts.is_empty() {
        "(まだなし)".to_string()
    } else {
        c.canon
            .facts
            .iter()
            .rev()
            .take(12)
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "世界観ラベル: {label}\n舞台設定: {setting}\nキャッチコピー: {tagline}\n登場人物（この章でも最低1人を証人として再登場させ、既定の動機/利害/口調に従わせること）:\n{cast}\n人物相関:\n{relationships}\nこれまでの登場履歴（続きとして矛盾なく描く）:\n{cast_log}\n\n大きな暗线（meta-mystery — 全章を貫く隠された真相。本章では解決せず、下記の“今進めるべき段階”だけを布石として織り込む）:\n{meta}\n{arc}\n\n世界の正典（これと矛盾してはならない既定事実）:\n{facts}\nすでに投下済みの伏線（繰り返さず、新しい角度で）:\n{dropped}\n\n進行度: {progress}/100\nこれまでの章:\n{played}",
        label = c.world_label,
        setting = c.setting,
        tagline = if c.tagline.trim().is_empty() { "(なし)" } else { c.tagline.trim() },
        cast = cast,
        relationships = relationships,
        cast_log = cast_log,
        meta = c.meta_mystery,
        arc = arc_section,
        facts = facts,
        dropped = dropped,
        progress = c.meta_progress,
        played = played,
    )
}

/// Render the persisted memory into a compact prompt section. Keeps only
/// recent entries so the AI isn't drowned.
pub(crate) fn format_memory_section(memory: &DetectiveMemory) -> String {
    if memory.mistakes.is_empty()
        && memory.mastered.is_empty()
        && memory.recent_evidence_titles.is_empty()
    {
        return "(まだ過去のセッション記録はない)".to_string();
    }
    let mistakes = memory
        .mistakes
        .iter()
        .rev()
        .take(6)
        .map(|m| format!("- {}（{}）", m.topic, m.course_name))
        .collect::<Vec<_>>()
        .join("\n");
    let mastered = memory
        .mastered
        .iter()
        .rev()
        .take(8)
        .map(|m| format!("- {}（{}）", m.topic, m.course_name))
        .collect::<Vec<_>>()
        .join("\n");
    let recent = memory
        .recent_evidence_titles
        .iter()
        .rev()
        .take(10)
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mistakes_block = if mistakes.is_empty() {
        "(なし)".to_string()
    } else {
        mistakes
    };
    let mastered_block = if mastered.is_empty() {
        "(なし)".to_string()
    } else {
        mastered
    };
    let recent_block = if recent.is_empty() {
        "(なし)".to_string()
    } else {
        recent
    };
    format!(
        "Recently failed topics (RE-EMPHASIZE — at least one lie should touch these):\n{}\n\nRecently mastered topics (DE-EMPHASIZE — don't repeat as the lie):\n{}\n\nRecently used evidence titles (find new angles, don't reuse verbatim):\n{}",
        mistakes_block, mastered_block, recent_block,
    )
}
