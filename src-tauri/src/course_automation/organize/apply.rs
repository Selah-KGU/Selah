use super::super::CourseAutomationStatus;
use super::theme::sanitize_component;
use super::types::{OrganizeCandidate, OrganizeFile, OrganizeGroup, OrganizeMove, PlannedGroup};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Files documents into theme folders per `planned`, mutating `status` (paths,
/// groups, undo log). Every resolved file is filed into its theme folder —
/// including a lone file in a session/topic of its own — so the end state is
/// that *every* document lives under a category, none stranded loose at the
/// course root. Already-filed files are left in place. Returns how many moved.
///
/// `course_root` is the course's top-level folder. Every destination is computed
/// as `course_root/<theme>/<filename>` regardless of where the file currently
/// sits, so filing is a pure function of (root, theme, name): it never nests a
/// file under its own theme folder, and re-running flattens any prior nesting
/// back to the one-level scheme.
pub fn apply_groups(
    status: &mut CourseAutomationStatus,
    candidates: &BTreeMap<String, OrganizeCandidate>,
    planned: &[PlannedGroup],
    course_root: &Path,
) -> usize {
    let mut groups: Vec<OrganizeGroup> = Vec::new();
    let mut moves: Vec<OrganizeMove> = Vec::new();
    let mut path_map: BTreeMap<String, String> = BTreeMap::new();
    let mut used_folders: Vec<String> = Vec::new();

    for group in planned {
        // Resolve members to candidates whose file still exists on disk.
        let members: Vec<&OrganizeCandidate> = group
            .doc_ids
            .iter()
            .filter_map(|id| candidates.get(id))
            .filter(|cand| Path::new(&cand.path).is_file())
            .collect();
        // File every document that resolves on disk, lone files included: the
        // goal is that nothing is left uncategorized at the course root. A single
        // 第NN回 handout still belongs in its 第NN回 folder.
        if members.is_empty() {
            continue;
        }
        let mut folder = sanitize_component(&group.label);
        if folder.is_empty() {
            continue;
        }
        // Disambiguate a folder name the planner reused for two distinct groups.
        while used_folders.contains(&folder) {
            folder.push('_');
        }
        used_folders.push(folder.clone());

        let mut files: Vec<OrganizeFile> = Vec::new();
        for cand in members {
            let from = PathBuf::from(&cand.path);
            let mut file = OrganizeFile {
                filename: cand.filename.clone(),
                title: cand.title.clone(),
                from_path: cand.path.clone(),
                to_path: cand.path.clone(),
                moved: false,
            };
            if let Some(to) = plan_destination(&from, &folder, course_root) {
                let to_str = to.to_string_lossy().to_string();
                if move_file(&from, &to) {
                    path_map.insert(cand.path.clone(), to_str.clone());
                    moves.push(OrganizeMove {
                        current: to_str.clone(),
                        original: cand.path.clone(),
                    });
                    file.to_path = to_str;
                    file.moved = true;
                }
            }
            files.push(file);
        }
        groups.push(OrganizeGroup {
            id: folder.clone(),
            label: group.label.clone(),
            kind: group.kind.clone(),
            folder,
            files,
        });
    }

    // Re-point the ledger at the new locations so reuse / re-analysis still find
    // the files after they move.
    if !path_map.is_empty() {
        for doc in status.document_analyses.iter_mut() {
            if let Some(next) = path_map.get(&doc.path) {
                doc.path = next.clone();
            }
        }
        for artifact in status.artifacts.iter_mut() {
            if let Some(next) = path_map.get(&artifact.path) {
                artifact.path = next.clone();
            }
        }
    }

    if groups.is_empty() {
        status.organize_groups.clear();
    } else {
        status.organize_groups = groups;
    }
    if !moves.is_empty() {
        status.organize_undo = moves;
        status.organize_can_undo = true;
    }

    // Sweep away any theme folders the moves just emptied — including the deep
    // nested chains a self-heal pulled files out of — so filing never leaves
    // hollow directories behind.
    prune_empty_dirs(course_root);
    path_map.len()
}

/// Removes empty subdirectories under `root` (never `root` itself), bottom-up.
/// A directory counts as empty when it holds only OS junk (.DS_Store / Thumbs.db
/// / desktop.ini) and subdirectories that were themselves pruned; the junk is
/// deleted so the folder can go. Unknown files (incl. other hidden files) keep a
/// directory, so nothing with real content is ever removed.
pub(in crate::course_automation) fn prune_empty_dirs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune_dir(&path);
        }
    }
}

/// Recursively prunes `dir`; returns whether it was (now) removed.
fn prune_dir(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut removable = true;
    let mut junk: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !prune_dir(&path) {
                removable = false;
            }
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini") {
                junk.push(path);
            } else {
                removable = false;
            }
        }
    }
    if !removable {
        return false;
    }
    for path in junk {
        let _ = std::fs::remove_file(path);
    }
    std::fs::remove_dir(dir).is_ok()
}

/// Reverts the last organize batch: moves the filed files back and removes the
/// now-empty theme folders. Returns how many files were restored. `course_root`
/// scopes the empty-folder sweep so every vacated theme folder is cleared, not
/// just each file's immediate parent.
pub fn undo_organize(status: &mut CourseAutomationStatus, course_root: &Path) -> usize {
    let moves = std::mem::take(&mut status.organize_undo);
    let mut back_map: BTreeMap<String, String> = BTreeMap::new();
    let mut restored = 0usize;
    for mv in moves.iter().rev() {
        let current = PathBuf::from(&mv.current);
        let original = PathBuf::from(&mv.original);
        if !current.is_file() || original.exists() {
            continue;
        }
        if let Some(parent) = original.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(&current, &original).is_ok() {
            restored += 1;
            back_map.insert(mv.current.clone(), mv.original.clone());
        }
    }

    if !back_map.is_empty() {
        for doc in status.document_analyses.iter_mut() {
            if let Some(next) = back_map.get(&doc.path) {
                doc.path = next.clone();
            }
        }
        for artifact in status.artifacts.iter_mut() {
            if let Some(next) = back_map.get(&artifact.path) {
                artifact.path = next.clone();
            }
        }
    }

    // Clear every theme folder the restore emptied (incl. nested chains), not
    // only the leaf parents.
    prune_empty_dirs(course_root);
    status.organize_groups.clear();
    status.organize_can_undo = false;
    restored
}

/// Destination is always `<course_root>/<folder>/<filename>`, independent of
/// where the file currently sits. This makes filing a pure, convergent function:
/// a file at the root moves in, a file already at the destination stays put
/// (`dest == from` → None, idempotent), and a file buried in a nested theme
/// folder (`第01回/第01回/…`) is pulled back up to the single-level location —
/// so re-running self-heals any historical nesting instead of deepening it.
fn plan_destination(from: &Path, folder: &str, course_root: &Path) -> Option<PathBuf> {
    let filename = from.file_name()?;
    let dest = course_root.join(folder).join(filename);
    if dest == from {
        return None;
    }
    Some(dest)
}

/// Moves `from` → `to`, creating the folder. Never overwrites an existing file.
fn move_file(from: &Path, to: &Path) -> bool {
    if to.exists() {
        return false;
    }
    if let Some(dir) = to.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    std::fs::rename(from, to).is_ok()
}
