//! Application-owned filesystem locations, independent of HTTP and IPC.
pub(crate) fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let dir = base.join("com.kgu.selah");
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(unix)]
        {
            let _ =
                std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        }
        dir
    })
    .clone()
}
