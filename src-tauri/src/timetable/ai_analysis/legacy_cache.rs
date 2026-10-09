// Frozen loader before reusing the snapshot metadata read. Test only.
use super::*;

pub(super) fn load_ai_cache_before(
    db: &Database,
) -> Result<(Option<AiScheduleResult>, bool), String> {
    let snapshot = db.get_snapshot_state()?;
    load_ai_cache_inner(db).map(|opt| match opt {
        Some((mut result, ts)) => {
            if let Some(snapshot) = snapshot.as_ref() {
                let scope = crate::academic_period::visible_weeks(
                    &snapshot.current_week_label,
                    &snapshot.next_week_label,
                    &snapshot.luna_year,
                    &snapshot.luna_term,
                    chrono::Local::now().date_naive(),
                );
                let current_mismatch =
                    week_label_mismatch(&scope.current, &result.current_week_label);
                let next_mismatch = week_label_mismatch(&scope.next, &result.next_week_label);
                if current_mismatch {
                    result.current_week.clear();
                    result.current_week_label.clear();
                }
                if next_mismatch {
                    result.next_week.clear();
                    result.next_week_label.clear();
                }
                if current_mismatch || next_mismatch {
                    log::info!(
                        "load_ai_cache: week label mismatch, ignoring cached AI schedule for that week"
                    );
                }
                if result.current_week.is_empty() && result.next_week.is_empty() {
                    return (None, true);
                }
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let stale = now - ts > AI_CACHE_MAX_AGE;
            (Some(result), stale)
        }
        None => (None, true),
    })
}
