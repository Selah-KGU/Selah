//! Runs SenseA filing after a course cycle.
//!
//! Builds organize candidates from the ledger and the course folder, asks
//! the model only for the ambiguous remainder, then applies the combined
//! plan. Deterministic grouping stays in \`organize\`.

use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizeInput<'a> {
    course_name: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    course_schedule: &'a str,
    /// Announcement notices that pin a 第NN回 to a date/topic — the date↔回 bridge
    /// the schedule lacks, so the planner can file date-named files (live notes,
    /// 座席表) into the right session.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notices: Vec<&'a str>,
    documents: Vec<OrganizeDocInput<'a>>,
}

/// Compact announcement notices that pin a session (第NN回): these carry the
/// date↔回 pairing absent from the syllabus, drawn from the per-announcement
/// analysis already generated. Bounded in count so the planner prompt stays lean.
pub(super) fn organize_notices(status: &CourseAutomationStatus) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for doc in &status.document_analyses {
        if doc.status != "done" || doc.kind != "announcement" {
            continue;
        }
        let probe = format!("{} {} {}", doc.title, doc.summary, doc.findings.join(" "));
        if organize::detect_session(&probe).is_none() {
            continue;
        }
        let line = organize_signal(&format!("{}｜{}", doc.title, doc.summary), &doc.findings);
        if line.trim().is_empty() {
            continue;
        }
        if seen.insert(line.clone()) {
            out.push(line);
        }
        if out.len() >= 24 {
            break;
        }
    }
    out
}

/// Best-effort 授業計画 for the course as compact "第NN回: テーマ" lines, drawn
/// from the cached KGC syllabus (matched to the luna course by simplified name).
/// Empty when no syllabus is on hand — the grouping then works from titles and
/// summaries alone. DB-only, no network.
pub(super) fn course_schedule_text(db: &Database, course_name: &str) -> String {
    let target = crate::commands::simplify_course_name(course_name);
    if target.trim().is_empty() {
        return String::new();
    }
    let Ok(by_name) = db.get_planned_sessions_by_name() else {
        return String::new();
    };
    let plan = by_name
        .into_iter()
        .filter(|(name, sessions)| {
            !sessions.is_empty() && crate::commands::simplify_course_name(name) == target
        })
        .max_by_key(|(_, sessions)| sessions.len());
    let Some((_, mut sessions)) = plan else {
        return String::new();
    };
    sessions.sort_by_key(|(num, _, _)| *num);
    sessions
        .iter()
        .filter(|(num, topic, _)| *num > 0 && !topic.trim().is_empty())
        .map(|(num, topic, _)| format!("第{:02}回: {}", num, topic.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizeDocInput<'a> {
    id: &'a str,
    kind: &'a str,
    /// The actual filename — it usually carries the clearest session marker
    /// (第03回資料配付.pdf), which the title/summary may have dropped. Without it
    /// the planner is blind to the single most reliable grouping signal.
    filename: &'a str,
    title: &'a str,
    summary: &'a str,
}

#[derive(Deserialize, Default)]
struct OrganizeAgentOutput {
    #[serde(default)]
    groups: Vec<OrganizeAgentGroup>,
}

#[derive(Deserialize, Default)]
struct OrganizeAgentGroup {
    #[serde(default)]
    label: String,
    #[serde(default, alias = "ids", alias = "documentIds", alias = "docIds")]
    file_ids: Vec<String>,
}

/// Files the course's documents into theme folders after a successful cycle.
/// Prefers an AI grouping over the per-document summaries (the "smart" filing
/// the user asked for); falls back to the deterministic heuristic if the model
/// is unavailable or returns nothing usable. Best-effort: failures are silent
/// and leave the files where they are. Returns how many files were moved.
pub(super) async fn organize_course_documents(
    luna_id: &str,
    status: &mut CourseAutomationStatus,
    schedule: &str,
    force: bool,
) -> usize {
    // Candidates = tracked ledger documents + loose Live notes physically in the
    // course folder. Nothing to do for fewer than two (no grouping possible).
    let course_root = crate::commands::resolve_download_dir(Some(&status.course_name));
    let candidates = build_organize_candidates(status, &course_root);
    if candidates.len() < 2 {
        return 0;
    }

    // Skip the AI planner unless the candidate set itself changed since we last
    // planned. Filing is idempotent (already-filed files don't move), so re-
    // planning the same set just reproduces the same placement at the cost of one
    // request. The signature is over the candidate *ids* (stable content
    // fingerprints / "loose:<name>"), so it flips only when a file is added or
    // removed — a newly-downloaded document or a fresh loose note — and never on
    // a mere path change from a previous filing. This precisely gates the call on
    // genuinely new inputs, with no re-fire for an ungroupable lone file. `force`
    // (the user's manual "整理" button) bypasses the gate but still refreshes the
    // signature so the next automatic cycle doesn't redundantly re-plan.
    let signature = organize_candidate_signature(&candidates);
    if !force && signature == status.organize_signature {
        return 0;
    }
    status.organize_signature = signature;

    // Cost-first: place everything the free, deterministic heuristic is confident
    // about (explicit 第N回 / topic markers) up front, and only spend the AI on the
    // ambiguous remainder (date-named notes, markerless files) — handing it just
    // those few files plus the announcement notices that bridge dates to 第NN回.
    // When nothing is ambiguous, the AI is never called at all.
    let (confident, ambiguous) = organize::confident_plan(&candidates);

    let ai_plan = if ambiguous.is_empty() {
        Vec::new()
    } else {
        let subset: std::collections::BTreeMap<String, organize::OrganizeCandidate> = ambiguous
            .iter()
            .filter_map(|id| candidates.get(id).map(|cand| (id.clone(), cand.clone())))
            .collect();
        let notices = organize_notices(status);
        match plan_organize_with_agent(luna_id, &subset, &status.course_name, schedule, &notices)
            .await
        {
            Ok(plan) => plan,
            Err(error) => {
                log::warn!(
                    "[course_automation] organize planning failed, using heuristic: {error}"
                );
                Vec::new()
            }
        }
    };

    // Confident heuristic groups are primary; the AI's placements for the ambiguous
    // files merge in; a final heuristic sweep gives any still-unplaced file a kind
    // folder so nothing is left loose at the course root.
    let mut combined = organize::merge_plans(confident, ai_plan);
    let assigned: HashSet<String> = combined
        .iter()
        .flat_map(|group| group.doc_ids.iter().cloned())
        .collect();
    let sweep = organize::heuristic_plan(&candidates, &assigned);
    combined = organize::merge_plans(combined, sweep);
    organize::apply_groups(status, &candidates, &combined, &course_root)
}

/// Builds the unified candidate set, driven by the files actually on disk so the
/// organizer can never go blind to a document. The SenseA ledger often holds an
/// empty `path` (the unified items model regenerates analyses without re-binding
/// the download location), so keying candidate discovery on `doc.path` alone
/// silently drops most files. Instead we walk the course folder for every real
/// file and enrich each with the ledger's AI title/summary/kind matched by
/// filename — so the LLM planner still gets full semantic signal, while filing
/// is anchored to what truly exists. Tracked docs whose path is still valid are
/// added directly; everything else (handouts with a lost path, loose Live notes)
/// is picked up by the disk sweep.
fn build_organize_candidates(
    status: &CourseAutomationStatus,
    course_root: &Path,
) -> std::collections::BTreeMap<String, organize::OrganizeCandidate> {
    let mut candidates: std::collections::BTreeMap<String, organize::OrganizeCandidate> =
        std::collections::BTreeMap::new();
    let mut tracked_paths: HashSet<String> = HashSet::new();

    // Live notes whose recording is still live (their day-cache sidecar exists):
    // the Live writer keeps rewriting them at a FIXED root path, so moving one now
    // would split-brain (a stale copy in a theme folder + a fresh one re-created at
    // the root on the next save). Leave them at the root until the session finishes
    // (`remove_day_cache` clears the sidecar), then they file normally.
    let in_progress = active_live_notes(course_root);

    // Ledger lookup by filename, so a swept file recovers its AI analysis even
    // when the stored path is empty/stale. First done analysis per name wins.
    let mut ledger_by_name: HashMap<&str, &DocumentAnalysis> = HashMap::new();
    for doc in &status.document_analyses {
        if doc.status != "done" {
            continue;
        }
        if !doc.filename.trim().is_empty() {
            ledger_by_name.entry(doc.filename.as_str()).or_insert(doc);
        }
        // Directly usable: a ledger doc whose path still points at a real file.
        if !doc.path.trim().is_empty()
            && Path::new(&doc.path).is_file()
            && !in_progress.contains(&doc.filename)
        {
            tracked_paths.insert(doc.path.clone());
            candidates
                .entry(doc.id.clone())
                .or_insert_with(|| organize::OrganizeCandidate {
                    path: doc.path.clone(),
                    filename: doc.filename.clone(),
                    title: doc.title.clone(),
                    summary: organize_signal(&doc.summary, &doc.findings),
                    kind: doc.kind.clone(),
                });
        }
    }

    collect_disk_files(
        course_root,
        &tracked_paths,
        &in_progress,
        &ledger_by_name,
        &mut candidates,
        0,
    );
    candidates
}

/// Coarse fallback kind for a swept file with no ledger match, from its
/// extension: text notes read as Live notes, everything else as course material.
/// Used only when the AI analysis (richer) is unavailable.
fn default_candidate_kind(ext: &str) -> &'static str {
    match ext {
        "md" | "txt" => "ライブノート",
        _ => "material",
    }
}

/// Filenames of Live notes whose recording is still live — derived from the
/// day-cache sidecars (`.<date>_<course>_live*.cache.json` / `.lines.ndjson`) the
/// Live module keeps at the course root while a session is open or resumable, and
/// deletes on finish. The organizer skips these so it never moves a note the Live
/// writer will re-create at the fixed root path. Maps each sidecar back to its
/// `…_live.md` name (tolerating the build-cache tag between `_live` and the suffix).
pub(super) fn active_live_notes(course_root: &Path) -> HashSet<String> {
    let mut active = HashSet::new();
    let Ok(entries) = std::fs::read_dir(course_root) else {
        return active;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !name.starts_with('.') {
            continue;
        }
        let stem = match name
            .strip_suffix(".cache.json")
            .or_else(|| name.strip_suffix(".lines.ndjson"))
        {
            Some(stem) => stem.trim_start_matches('.'),
            None => continue,
        };
        // stem = "<date>_<course>_live<tag>" → the markdown is "<…_live>.md".
        if let Some(idx) = stem.rfind("_live") {
            active.insert(format!("{}_live.md", &stem[..idx]));
        }
    }
    active
}

/// Recursively gathers every real file under `dir` into `candidates`, bounded in
/// depth so a pathological tree can't stall the sweep. Hidden files, Office lock
/// files (`~$…`), and paths already added as tracked candidates are skipped. Each
/// file is enriched from `ledger_by_name` (matched by filename) so it carries the
/// AI title/summary/kind when available; otherwise it falls back to the filename
/// and an extension-inferred kind. The id reuses the ledger doc's stable content
/// id when matched (keeps signature / reuse consistent), else `loose:<name>`.
fn collect_disk_files(
    dir: &Path,
    tracked_paths: &HashSet<String>,
    in_progress: &HashSet<String>,
    ledger_by_name: &HashMap<&str, &DocumentAnalysis>,
    candidates: &mut std::collections::BTreeMap<String, organize::OrganizeCandidate>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Skip hidden files/folders (.DS_Store, .{date}_live.cache.json) and
        // Office lock/owner files (~$report.docx) — transient, not real docs.
        if name.starts_with('.') || name.starts_with("~$") {
            continue;
        }
        if path.is_dir() {
            collect_disk_files(
                &path,
                tracked_paths,
                in_progress,
                ledger_by_name,
                candidates,
                depth + 1,
            );
            continue;
        }
        if !path.is_file() {
            continue;
        }
        // A Live note still being recorded must stay put — the Live writer keeps
        // rewriting it at the fixed root path, so moving it would split-brain.
        if in_progress.contains(name) {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if tracked_paths.contains(&path_str) {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let ledger = ledger_by_name.get(name).copied();
        let id = ledger
            .map(|doc| doc.id.clone())
            .unwrap_or_else(|| format!("loose:{name}"));
        let title = ledger
            .map(|doc| doc.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| name.to_string());
        let kind = ledger
            .map(|doc| doc.kind.clone())
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_else(|| default_candidate_kind(&ext).to_string());
        let summary = match ledger {
            // Use the full per-document analysis (summary + findings), not just
            // the one-liner, so classification leans on everything already extracted.
            Some(doc) if !doc.summary.trim().is_empty() || !doc.findings.is_empty() => {
                organize_signal(&doc.summary, &doc.findings)
            }
            _ if ext == "md" || ext == "txt" => read_text_preview(&path),
            _ => String::new(),
        };
        candidates.entry(id).or_insert(organize::OrganizeCandidate {
            path: path_str,
            filename: name.to_string(),
            title,
            summary,
            kind,
        });
    }
}

/// Fingerprint of the organize candidate set, over its ids only. The map is a
/// BTreeMap so key iteration is sorted and deterministic; the digest flips only
/// when a candidate is added or removed, never when a previously-filed file's
/// path changes (the id is stable). Used to skip the AI planner for a set we
/// have already planned.
pub(super) fn organize_candidate_signature(
    candidates: &std::collections::BTreeMap<String, organize::OrganizeCandidate>,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for id in candidates.keys() {
        id.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

/// Folds a document's AI summary together with its extracted `findings` into one
/// compact signal for organize, so classification reuses the *whole* per-document
/// analysis (the key points the model already pulled out) rather than only the
/// one-line summary. Whitespace-collapsed and length-bounded to keep the planner
/// prompt small.
pub(super) fn organize_signal(summary: &str, findings: &[String]) -> String {
    let mut text = summary.to_string();
    for finding in findings {
        if !finding.trim().is_empty() {
            text.push(' ');
            text.push_str(finding);
        }
    }
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(600)
        .collect()
}

/// A compact, whitespace-collapsed preview of a text note for the AI prompt.
fn read_text_preview(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    String::from_utf8_lossy(&bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect()
}

/// Asks the model to cluster the already-summarised documents into theme groups,
/// then maps the returned ids back to a `PlannedGroup` list (dropping ids the
/// model invented or duplicated).
async fn plan_organize_with_agent(
    luna_id: &str,
    candidates: &std::collections::BTreeMap<String, organize::OrganizeCandidate>,
    course_name: &str,
    schedule: &str,
    notices: &[String],
) -> Result<Vec<organize::PlannedGroup>, String> {
    let provider = AgentProvider::resolve().map_err(|error| error.to_string())?;
    let documents: Vec<OrganizeDocInput> = candidates
        .iter()
        .map(|(id, cand)| OrganizeDocInput {
            id: id.as_str(),
            kind: cand.kind.as_str(),
            filename: cand.filename.as_str(),
            title: cand.title.as_str(),
            summary: cand.summary.as_str(),
        })
        .collect();
    let kind_by_id: HashMap<&str, &str> = candidates
        .iter()
        .map(|(id, cand)| (id.as_str(), cand.kind.as_str()))
        .collect();
    let known_ids: HashSet<&str> = candidates.keys().map(|id| id.as_str()).collect();

    let input = OrganizeInput {
        course_name,
        course_schedule: schedule,
        notices: notices.iter().map(String::as_str).collect(),
        documents,
    };
    let mut hasher = Sha256::new();
    hasher.update(b"course-automation-organize");
    hasher.update([0]);
    hasher.update(luna_id.as_bytes());
    hasher.update([0]);
    hasher.update(schedule.as_bytes());
    for notice in &input.notices {
        hasher.update([0]);
        hasher.update(notice.as_bytes());
    }
    for doc in &input.documents {
        hasher.update([0]);
        hasher.update(doc.id.as_bytes());
    }
    let gen_id = format!("course-organize-{:x}", hasher.finalize());

    let response: PlusJsonResponse<OrganizeAgentOutput> = request_plus_json(
        &provider,
        context::ORGANIZE_SYSTEM_PROMPT,
        serde_json::to_string(&input).map_err(|error| error.to_string())?,
        Vec::new(),
        PLUS_DOCUMENT_MAX_TOKENS,
        10,
        &gen_id,
        "資料のテーマ整理",
    )
    .await?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut plan: Vec<organize::PlannedGroup> = Vec::new();
    for group in response.value.groups {
        // Canonicalize so the model's session labels merge with the heuristic's
        // (第3回 / 第１０回 → 第03回 / 第10回) instead of forking a parallel folder.
        let label = organize::canonical_session_label(&group.label);
        if label.is_empty() {
            continue;
        }
        let doc_ids: Vec<String> = group
            .file_ids
            .into_iter()
            .filter(|id| known_ids.contains(id.as_str()) && seen.insert(id.clone()))
            .collect();
        if doc_ids.len() < 2 {
            continue;
        }
        let kind = doc_ids
            .first()
            .and_then(|id| kind_by_id.get(id.as_str()).copied())
            .unwrap_or("")
            .to_string();
        plan.push(organize::PlannedGroup {
            label,
            kind,
            doc_ids,
        });
    }
    Ok(plan)
}
