//! Remove app-owned files on the next process start, before SQLite/WebView
//! handles can lock them. The receipt survives partial cleanup and crashes.
use std::path::{Path, PathBuf};

fn roots() -> Vec<PathBuf> {
    let mut paths = vec![crate::client::data_dir()];
    if let Some(base) = dirs::data_dir() {
        let roaming = base.join("com.kgu.selah");
        if !paths.contains(&roaming) {
            paths.push(roaming);
        }
    }
    paths
}

fn receipt() -> PathBuf {
    crate::client::data_dir().with_file_name("com.kgu.selah.reset-pending")
}

pub(crate) fn schedule() -> Result<(), String> {
    crate::keychain::file::atomic_write(&receipt(), b"reset-on-restart\n")
}

fn remove_pending(receipt: &Path, roots: &[PathBuf]) -> Result<(), String> {
    if !receipt.exists() {
        return Ok(());
    }
    for root in roots {
        match std::fs::remove_dir_all(root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Complete local data deletion: {e}")),
        }
    }
    // Retain only non-secret sign-out intent. Old state cannot be restored if
    // a backup tool later recreates an obsolete vault or WebView store.
    let root = roots.first().ok_or("No application data directory")?;
    std::fs::create_dir_all(root).map_err(|e| format!("Create application data directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Protect application data directory: {e}"))?;
    }
    for name in ["university-signed-out", "university-signed-out.dev"] {
        crate::keychain::file::atomic_write(&root.join(name), b"signed-out\n")?;
    }
    std::fs::remove_file(receipt).map_err(|e| format!("Finish local data deletion: {e}"))
}

pub(crate) fn finish_pending() -> Result<(), String> {
    if receipt().exists() {
        crate::keychain::erase_persisted_for_reset()?;
    }
    remove_pending(&receipt(), &roots())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_purges_local_and_roaming_files_before_opening_clients() {
        let base = std::env::temp_dir().join(format!("selah-reset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let roots = [base.join("local"), base.join("roaming")];
        for root in &roots {
            std::fs::create_dir(root).unwrap();
            std::fs::write(root.join("fake.db"), b"old-data").unwrap();
        }
        let receipt = base.join("reset");
        std::fs::write(&receipt, b"pending").unwrap();
        remove_pending(&receipt, &roots).unwrap();
        assert!(!receipt.exists());
        assert!(!roots[1].exists());
        assert!(!roots[0].join("fake.db").exists());
        assert!(roots[0].join("university-signed-out").exists());
        remove_pending(&receipt, &roots).unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn partial_cleanup_keeps_receipt_for_retry() {
        let base = std::env::temp_dir().join(format!("selah-reset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let root = base.join("locked");
        std::fs::write(&root, b"not-a-directory").unwrap();
        let receipt = base.join("reset");
        std::fs::write(&receipt, b"pending").unwrap();
        assert!(remove_pending(&receipt, &[root.clone()]).is_err());
        assert!(receipt.exists());
        std::fs::remove_file(&root).unwrap();
        remove_pending(&receipt, &[root]).unwrap();
        assert!(!receipt.exists());
        std::fs::remove_dir_all(base).unwrap();
    }
}
