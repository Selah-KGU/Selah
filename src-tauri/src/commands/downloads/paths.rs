//! Download directory paths and course-name normalization.

use super::*;
use regex::Regex;

pub const OTHER_CATEGORY: &str = "その他";

/// Sanitize a string to be safe as a directory/file name component.
pub(super) fn sanitize_path_component(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .collect();
    let trimmed = s.trim().trim_matches('.');
    if trimmed.is_empty() {
        "_".into()
    } else {
        trimmed.to_string()
    }
}

/// Simplify a course name for use as a folder name.
pub fn simplify_course_name(name: &str) -> String {
    static RE_DEPT_CODE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^.+\s\d{7,8}\s+").unwrap());
    static RE_BRACKET: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"[\[［]\d+[\]］]").unwrap());
    static RE_PAREN_SUFFIX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"[（(][^)）]*(?:学期|限|クラス|組|セメスター|Quarter|Semester)[^)）]*[)）]\s*$")
            .unwrap()
    });
    // Strip trailing bare year / year-range, e.g. " 2025", "_2024-2025",
    // "（2025年度）", "(2025)" that don't contain a term keyword.
    static RE_YEAR_SUFFIX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"[\s_\-–・]*[（(]?\d{4}(?:[\-–]\d{2,4})?(?:年度?)?[)）]?\s*$").unwrap()
    });
    // Strip leading schedule prefix like "水４・金２ " or "月３金４ ".
    // Pattern: one or more (day-char + half/full-width digit) joined by ・/・/space,
    // followed by whitespace.
    static RE_SCHEDULE_PREFIX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^(?:[月火水木金土日][０-９0-9][\s・・]*)+").unwrap()
    });

    let s = RE_DEPT_CODE.replace(name, "");
    let s = RE_SCHEDULE_PREFIX.replace(&s, "");
    let s = RE_BRACKET.replace_all(&s, "");
    let s = RE_PAREN_SUFFIX.replace_all(&s, "");
    let s = RE_YEAR_SUFFIX.replace_all(&s, "");
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim().to_string();
    if s.is_empty() {
        name.trim().to_string()
    } else {
        s
    }
}

/// Default download base directory: ~/Documents/Selah (created if needed).
pub fn default_download_dir() -> std::path::PathBuf {
    let doc = dirs::document_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Documents"))
            .unwrap_or_else(std::env::temp_dir)
    });
    let dir = doc.join("Selah");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Resolve the download directory with optional course classification.
pub fn resolve_download_dir(course_name: Option<&str>) -> std::path::PathBuf {
    let config = load_download_config();
    let base = if config.download_dir.is_empty() {
        default_download_dir()
    } else {
        std::path::PathBuf::from(&config.download_dir)
    };

    if config.classify_by_course {
        let folder = match course_name.map(str::trim).filter(|s| !s.is_empty()) {
            Some(course) => sanitize_path_component(&simplify_course_name(course)),
            None => OTHER_CATEGORY.to_string(),
        };
        let dir = base.join(&folder);
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }

    base
}

/// Download base directory (configured, else default). The course-classified
/// layout under it is `base/<course>/[<theme>/...]<file>`.
pub(super) fn download_base() -> std::path::PathBuf {
    let config = load_download_config();
    if config.download_dir.is_empty() {
        default_download_dir()
    } else {
        std::path::PathBuf::from(&config.download_dir)
    }
}

/// The theme subfolder a file sits in within its course folder: the path
/// components between the course folder and the filename, relative to `base` and
/// "/"-joined. Empty when the file is at the course root (or outside `base`).
pub(super) fn theme_subfolder(path: &str, base: &std::path::Path) -> String {
    let Ok(rel) = std::path::Path::new(path).strip_prefix(base) else {
        return String::new();
    };
    let comps: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    // comps = [course, theme.., file]; drop the course head and the file tail.
    if comps.len() <= 2 {
        return String::new();
    }
    comps[1..comps.len() - 1].join("/")
}

/// Validate that `path` exists and resolves under one of the allowed download
/// roots (app default `~/Documents/Selah`, OS Downloads, or the user's custom
/// download dir). Returns the canonical path on success.
pub(super) fn validate_downloads_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err("ファイルが見つかりません".into());
    }
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("パスが無効です: {}", e))?;
    let app_default = default_download_dir()
        .canonicalize()
        .unwrap_or_else(|_| default_download_dir());
    let sys_downloads_raw = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Downloads"))
            .unwrap_or_else(std::env::temp_dir)
    });
    // Canonicalize sys_downloads so the starts_with comparison works correctly
    // on Windows where canonicalize() adds the \\?\\ extended-path prefix.
    let sys_downloads = sys_downloads_raw
        .canonicalize()
        .unwrap_or(sys_downloads_raw);
    let dl_config = load_download_config();
    let custom_dir = if dl_config.download_dir.is_empty() {
        None
    } else {
        std::path::Path::new(&dl_config.download_dir)
            .canonicalize()
            .ok()
    };
    let allowed = canonical.starts_with(&app_default)
        || canonical.starts_with(&sys_downloads)
        || custom_dir
            .as_ref()
            .is_some_and(|d| canonical.starts_with(d));
    if !allowed {
        return Err("ダウンロードフォルダ外のファイルは開けません".into());
    }
    Ok(canonical)
}
