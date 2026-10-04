use super::theme::{sanitize_component, theme_of};
use super::types::{OrganizeCandidate, PlannedGroup};
use std::collections::{BTreeMap, HashSet};

/// Splits candidates into what the heuristic can place *confidently* (an explicit
/// 第N回 session or a topic marker in title/filename/summary/findings) and the ids
/// it can only guess at (no marker → would fall to a kind folder). The confident
/// half is deterministic and free; returning the ambiguous remainder lets the
/// caller spend the AI only on the few files that actually need semantic/date
/// reasoning, instead of paying to re-derive placements the markers already give.
/// Iteration is in id order, so the split is stable.
pub fn confident_plan(
    candidates: &BTreeMap<String, OrganizeCandidate>,
) -> (Vec<PlannedGroup>, Vec<String>) {
    let mut order: Vec<String> = Vec::new();
    let mut clusters: BTreeMap<String, (String, String, Vec<String>)> = BTreeMap::new();
    let mut ambiguous: Vec<String> = Vec::new();
    for (id, cand) in candidates {
        let (key, label) = theme_of(&cand.title, &cand.summary, &cand.filename, &cand.kind);
        // Only a session/topic marker counts as confident; a bare kind fallback is
        // a guess the AI (with notices/schedule) can place better.
        if key.starts_with("kind:") {
            ambiguous.push(id.clone());
            continue;
        }
        let entry = clusters
            .entry(key.clone())
            .or_insert_with(|| (label, cand.kind.clone(), Vec::new()));
        if entry.2.is_empty() {
            order.push(key);
        }
        entry.2.push(id.clone());
    }
    let plan = order
        .into_iter()
        .filter_map(|key| clusters.remove(&key))
        .map(|(label, kind, doc_ids)| PlannedGroup {
            label,
            kind,
            doc_ids,
        })
        .collect();
    (plan, ambiguous)
}

/// Heuristic grouping from each candidate's title/summary/kind, used to sweep up
/// files the AI planner left unfiled (`exclude` holds the ids it already placed).
/// Iteration is in id order, so the same input always yields the same plan.
pub fn heuristic_plan(
    candidates: &BTreeMap<String, OrganizeCandidate>,
    exclude: &HashSet<String>,
) -> Vec<PlannedGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut clusters: BTreeMap<String, (String, String, Vec<String>)> = BTreeMap::new();
    for (id, cand) in candidates {
        if exclude.contains(id) {
            continue;
        }
        let (key, label) = theme_of(&cand.title, &cand.summary, &cand.filename, &cand.kind);
        let entry = clusters
            .entry(key.clone())
            .or_insert_with(|| (label, cand.kind.clone(), Vec::new()));
        if entry.2.is_empty() {
            order.push(key);
        }
        entry.2.push(id.clone());
    }
    order
        .into_iter()
        .filter_map(|key| clusters.remove(&key))
        .map(|(label, kind, doc_ids)| PlannedGroup {
            label,
            kind,
            doc_ids,
        })
        .collect()
}

/// Merges a secondary plan (the heuristic sweep) into a primary one (the AI
/// groups): same-folder groups combine their members so the heuristic feeds the
/// session folders the AI already opened instead of forking near-duplicates. An
/// id placed by the primary plan is never re-added by the secondary.
pub fn merge_plans(
    mut primary: Vec<PlannedGroup>,
    secondary: Vec<PlannedGroup>,
) -> Vec<PlannedGroup> {
    let mut by_folder: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (index, group) in primary.iter().enumerate() {
        by_folder
            .entry(sanitize_component(&group.label))
            .or_insert(index);
        for id in &group.doc_ids {
            seen.insert(id.clone());
        }
    }
    for group in secondary {
        let ids: Vec<String> = group
            .doc_ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if ids.is_empty() {
            continue;
        }
        let folder = sanitize_component(&group.label);
        if let Some(&index) = by_folder.get(&folder) {
            primary[index].doc_ids.extend(ids);
        } else {
            by_folder.insert(folder, primary.len());
            primary.push(PlannedGroup {
                label: group.label,
                kind: group.kind,
                doc_ids: ids,
            });
        }
    }
    primary
}
