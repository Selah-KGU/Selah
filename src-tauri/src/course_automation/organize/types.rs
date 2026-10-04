use serde::{Deserialize, Serialize};

/// One document filed under a theme group (paths kept for provenance / undo).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeFile {
    pub filename: String,
    pub title: String,
    pub from_path: String,
    pub to_path: String,
    pub moved: bool,
}

/// A theme cluster — its files share a session marker (第3回) or an assignment
/// topic (レポート), else they fall back to a kind folder (教材 / 課題).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeGroup {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub folder: String,
    pub files: Vec<OrganizeFile>,
}

/// One reversible move: where the file is now, and where to put it back.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeMove {
    pub current: String,
    pub original: String,
}

/// A grouping decision (from the AI planner or the heuristic fallback): a theme
/// label and the ids of the documents that belong under it. Decoupled from disk
/// work so the same `apply_groups` machinery files either source.
#[derive(Debug, Clone, Default)]
pub struct PlannedGroup {
    pub label: String,
    pub kind: String,
    pub doc_ids: Vec<String>,
}

/// One file the organizer can move — either a tracked ledger document or a loose
/// file sitting in the course folder (e.g. a Live note the agent never tracked).
/// Unifying both lets the AI place them together and the same machinery file them.
/// Ledger documents and loose files share this shape. On move, `apply_groups`
/// patches any matching ledger path; a loose file simply has no entry to patch.
#[derive(Debug, Clone)]
pub struct OrganizeCandidate {
    pub path: String,
    pub filename: String,
    pub title: String,
    pub summary: String,
    pub kind: String,
}
