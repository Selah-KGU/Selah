//! 自動検知 file organization.
//!
//! Groups a course's already-downloaded documents by theme (same session /
//! same assignment topic) and files each group into a theme subfolder next to
//! where the files already live. Best-effort and fully reversible: every move
//! is recorded so the whole batch can be undone in one step.
//!
//! The grouping reuses the per-document AI analysis (title / kind) the agent
//! already produced — the "smart" part is the perception upstream; here we turn
//! that into a deterministic, predictable filing so the same input always files
//! the same way.

#[path = "organize/apply.rs"]
mod apply;
#[path = "organize/plan.rs"]
mod plan;
#[path = "organize/theme.rs"]
mod theme;
#[path = "organize/types.rs"]
mod types;

pub use apply::{apply_groups, undo_organize};
pub use plan::{confident_plan, heuristic_plan, merge_plans};
pub use theme::{canonical_session_label, detect_session};
#[allow(unused_imports)]
pub use types::OrganizeFile;
pub use types::{OrganizeCandidate, OrganizeGroup, OrganizeMove, PlannedGroup};

#[cfg(test)]
#[path = "organize/tests.rs"]
mod tests;
