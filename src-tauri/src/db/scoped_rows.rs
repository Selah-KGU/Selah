//! Read body rows only for keys selected from lightweight metadata. Keep input
//! values bound, and limit parameter counts independently of the SQLite build.
use super::*;

const KEYS_PER_QUERY: usize = 256;

pub(super) fn query_selected<T>(
    conn: &Connection,
    keys: &[&str],
    select: &str,
    column: &str,
    order: &str,
    decode: fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, String> {
    // SQL fragments are internal constants. User-supplied keys are parameters.
    let mut keys = keys.to_vec();
    keys.sort_unstable();
    keys.dedup();
    let mut result = Vec::new();
    for index in 0..keys.len().max(1).div_ceil(KEYS_PER_QUERY) {
        let start = index * KEYS_PER_QUERY;
        let chunk = &keys[start..keys.len().min(start + KEYS_PER_QUERY)];
        // Even with no visible keys, prepare the projection so a missing table
        // or column remains a query error, rather than an empty success.
        let predicate = if chunk.is_empty() {
            "0".to_string()
        } else {
            format!("{column} IN ({})", vec!["?"; chunk.len()].join(","))
        };
        let mut stmt = conn
            .prepare_cached(&format!("{select} WHERE {predicate} {order}"))
            .map_err(|e| format!("DB query: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(chunk.iter()), decode)
            .map_err(|e| format!("DB map: {e}"))?;
        // Preserve the full readers' per-row malformed-data fallback.
        result.extend(rows.filter_map(Result::ok));
    }
    Ok(result)
}

pub(super) fn visible_luna_ids(
    conn: &Connection,
    table: &str,
    year: &str,
    term: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare_cached(&format!("SELECT DISTINCT luna_id FROM {table}"))
        .map_err(|e| format!("DB query: {e}"))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("DB map: {e}"))?;
    // Retain Rust's byte offsets and unknown/sentinel/year-long ID behavior.
    Ok(ids
        .filter_map(Result::ok)
        .filter(|id| luna_course_matches_snapshot(id, year, term))
        .collect())
}
