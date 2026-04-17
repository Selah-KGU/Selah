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
// Release macOS: SecItem API via security-framework v3
// Unlike the legacy SecKeychainAddGenericPassword (used by keyring crate),
// SecItemAdd/SecItemCopyMatching/SecItemUpdate bind ACL to the code signing
// identity, so a properly signed app won't get repeated keychain prompts.
// ---------------------------------------------------------------------------

#[cfg(all(not(debug_assertions), target_os = "macos"))]
const SERVICE: &str = "com.kgu.selah";

#[cfg(all(not(debug_assertions), target_os = "macos"))]
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(SERVICE, key, value.as_bytes())
        .map_err(|e| format!("Keychain set error: {}", e))
}

#[cfg(all(not(debug_assertions), target_os = "macos"))]
pub fn get_secret(key: &str) -> Option<String> {
    security_framework::passwords::get_generic_password(SERVICE, key)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(all(not(debug_assertions), target_os = "macos"))]
pub fn delete_secret(key: &str) {
    let _ = security_framework::passwords::delete_generic_password(SERVICE, key);
}

// ---------------------------------------------------------------------------
// Release Windows: keyring crate (Windows Credential Manager)
// ---------------------------------------------------------------------------

#[cfg(all(not(debug_assertions), target_os = "windows"))]
const SERVICE: &str = "com.kgu.selah";

#[cfg(all(not(debug_assertions), target_os = "windows"))]
fn entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, key).map_err(|e| format!("Keychain entry error: {}", e))
}

#[cfg(all(not(debug_assertions), target_os = "windows"))]
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    entry(key)?
        .set_password(value)
        .map_err(|e| format!("Credential set error: {}", e))
}

#[cfg(all(not(debug_assertions), target_os = "windows"))]
pub fn get_secret(key: &str) -> Option<String> {
    entry(key).ok()?.get_password().ok()
}

#[cfg(all(not(debug_assertions), target_os = "windows"))]
pub fn delete_secret(key: &str) {
    if let Ok(e) = entry(key) {
        let _ = e.delete_credential();
    }
}
