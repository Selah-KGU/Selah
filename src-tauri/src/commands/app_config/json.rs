use serde::de::DeserializeOwned;
use serde::Serialize;

pub(in crate::commands::app_config) fn load_json_config<T: Default + DeserializeOwned>(
    path: &std::path::Path,
) -> T {
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    T::default()
}

pub(in crate::commands::app_config) fn save_json_config<T: Serialize>(
    path: &std::path::Path,
    config: &T,
    label: &str,
) -> Result<(), String> {
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(path, &data).map_err(|e| format!("Failed to write {}: {}", label, e))?;
    Ok(())
}
