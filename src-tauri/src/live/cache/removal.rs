use std::io;
use std::path::Path;

pub(in crate::live) fn remove_files(path: &Path, deltas: &Path) -> Result<(), String> {
    remove_with(path, deltas, |path| std::fs::remove_file(path))
}

fn remove_with(
    path: &Path,
    deltas: &Path,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
) -> Result<(), String> {
    // A failed base deletion must not also remove the journal's newer speech.
    // Two removals are not a filesystem transaction: a later journal failure
    // can follow an already deleted base, and must still reach the caller.
    for (file, context) in [
        (path, "LIVEキャッシュの削除失敗"),
        (deltas, "LIVE字幕ログの削除失敗"),
    ] {
        if let Err(error) = remove(file) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(format!("{context}: {error}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::live) fn denied_fixture() -> Result<(), String> {
    remove_with(Path::new("cache"), Path::new("journal"), |_| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fixture deletion denied",
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("selah-cache-removal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn cache(&self) -> PathBuf {
            self.0.join("day.cache.json")
        }
        fn journal(&self) -> PathBuf {
            self.0.join("lines.ndjson")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn actual_files_missing_combinations_and_repeat_clear_are_idempotent() {
        for base in [false, true] {
            for journal in [false, true] {
                let dir = Directory::new();
                if base {
                    std::fs::write(dir.cache(), "完全な base 🌙").unwrap();
                }
                if journal {
                    std::fs::write(dir.journal(), "追加した完全な発話 🌙").unwrap();
                }
                std::fs::write(dir.0.join("formal.md"), "archived recording").unwrap();
                remove_files(&dir.cache(), &dir.journal()).unwrap();
                remove_files(&dir.cache(), &dir.journal()).unwrap();
                assert!(!dir.cache().exists());
                assert!(!dir.journal().exists());
                assert_eq!(
                    std::fs::read_to_string(dir.0.join("formal.md")).unwrap(),
                    "archived recording"
                );
            }
        }
    }

    #[test]
    fn real_base_failure_preserves_journal_and_retry_removes_both_files() {
        let dir = Directory::new();
        std::fs::create_dir(dir.cache()).unwrap();
        std::fs::write(dir.journal(), b"new speech not yet in base").unwrap();
        assert!(remove_files(&dir.cache(), &dir.journal())
            .unwrap_err()
            .starts_with("LIVEキャッシュの削除失敗:"));
        assert_eq!(
            std::fs::read(dir.journal()).unwrap(),
            b"new speech not yet in base"
        );
        // Frozen predecessor silently deleted the journal after the same base
        // error, while its caller acknowledged success.
        let _ = std::fs::remove_file(dir.cache());
        let _ = std::fs::remove_file(dir.journal());
        assert!(dir.cache().is_dir());
        assert!(!dir.journal().exists());
        std::fs::remove_dir(dir.cache()).unwrap();
        std::fs::write(dir.cache(), b"base").unwrap();
        std::fs::write(dir.journal(), b"journal").unwrap();
        remove_files(&dir.cache(), &dir.journal()).unwrap();
        assert!(!dir.cache().exists());
        assert!(!dir.journal().exists());
    }

    #[test]
    fn journal_failures_are_reported_even_if_the_base_was_already_removed() {
        let dir = Directory::new();
        std::fs::write(dir.cache(), b"base").unwrap();
        std::fs::create_dir(dir.journal()).unwrap();
        assert!(remove_files(&dir.cache(), &dir.journal())
            .unwrap_err()
            .starts_with("LIVE字幕ログの削除失敗:"));
        assert!(!dir.cache().exists());
        assert!(dir.journal().is_dir());
    }

    #[test]
    fn only_missing_files_are_ignored_and_a_base_error_stops_journal_work() {
        let base = Path::new("base");
        let journal = Path::new("journal");
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Interrupted,
            io::ErrorKind::InvalidData,
            io::ErrorKind::Other,
        ] {
            let mut visited = Vec::new();
            let result = remove_with(base, journal, |path| {
                visited.push(path.to_owned());
                Err(io::Error::new(kind, "test failure"))
            });
            assert_eq!(
                result.unwrap_err(),
                "LIVEキャッシュの削除失敗: test failure"
            );
            assert_eq!(visited, vec![base.to_owned()]);
        }
        let mut visited = Vec::new();
        assert_eq!(
            remove_with(base, journal, |path| {
                visited.push(path.to_owned());
                Err(io::Error::new(
                    if path == base {
                        io::ErrorKind::NotFound
                    } else {
                        io::ErrorKind::PermissionDenied
                    },
                    "test failure",
                ))
            })
            .unwrap_err(),
            "LIVE字幕ログの削除失敗: test failure"
        );
        assert_eq!(visited, vec![base.to_owned(), journal.to_owned()]);
    }
}
