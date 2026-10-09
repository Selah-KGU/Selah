use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Where app secrets (API keys, tokens, cookies) are stored:
    ///   "keychain" (default) — OS-protected random key + encrypted vault.
    ///   "file"               — a weaker machine-derived encrypted file;
    ///                          normal reads/writes do not use the OS keychain.
    pub secret_store: String,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            secret_store: "keychain".to_string(),
        }
    }
}

fn security_config_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("security_config.json")
}

pub fn load_security_config() -> SecurityConfig {
    std::fs::read_to_string(security_config_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}
