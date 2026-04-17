#[cfg(not(debug_assertions))]
use keyring::Entry;

#[cfg(not(debug_assertions))]
const SERVICE: &str = "com.kgu.selah";

#[cfg(not(debug_assertions))]
fn entry(key: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, key).map_err(|e| format!("Keychain entry error: {}", e))
}

// ---------------------------------------------------------------------------
// Debug builds: plain files in data_dir/secrets/ instead of OS keychain
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
fn secrets_dir() -> std::path::PathBuf {
    let dir = crate::client::data_dir().join("secrets");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(debug_assertions)]
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    let path = secrets_dir().join(key);
    std::fs::write(&path, value)
        .map_err(|e| format!("Failed to write secret file: {}", e))
}

#[cfg(debug_assertions)]
pub fn get_secret(key: &str) -> Option<String> {
    std::fs::read_to_string(secrets_dir().join(key)).ok()
}

#[cfg(debug_assertions)]
pub fn delete_secret(key: &str) {
    let _ = std::fs::remove_file(secrets_dir().join(key));
}

// ---------------------------------------------------------------------------
// Release builds: real OS keychain
// ---------------------------------------------------------------------------

#[cfg(not(debug_assertions))]
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    entry(key)?
        .set_password(value)
        .map_err(|e| format!("Keychain set error: {}", e))
}

#[cfg(not(debug_assertions))]
pub fn get_secret(key: &str) -> Option<String> {
    entry(key).ok()?.get_password().ok()
}

#[cfg(not(debug_assertions))]
pub fn delete_secret(key: &str) {
    if let Ok(e) = entry(key) {
        let _ = e.delete_credential();
    }
}
