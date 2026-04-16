use keyring::Entry;

const SERVICE: &str = "com.kgu.selah";

fn entry(key: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, key).map_err(|e| format!("Keychain entry error: {}", e))
}

/// Store a secret in the OS keychain.
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    entry(key)?
        .set_password(value)
        .map_err(|e| format!("Keychain set error: {}", e))
}

/// Retrieve a secret from the OS keychain. Returns None if not found.
pub fn get_secret(key: &str) -> Option<String> {
    entry(key).ok()?.get_password().ok()
}

/// Delete a secret from the OS keychain.
pub fn delete_secret(key: &str) {
    if let Ok(e) = entry(key) {
        let _ = e.delete_credential();
    }
}
