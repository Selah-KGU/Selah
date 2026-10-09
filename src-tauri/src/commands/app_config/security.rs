pub use crate::keychain::config::{load_security_config, SecurityConfig};

#[tauri::command]
pub fn get_security_config() -> SecurityConfig {
    load_security_config()
}

#[tauri::command]
pub fn save_security_config(config: SecurityConfig) -> Result<(), String> {
    // Commit the destination vault before switching the persisted preference.
    crate::keychain::migrate_storage(&config.secret_store)
}
