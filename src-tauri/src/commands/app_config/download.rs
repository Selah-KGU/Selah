use super::json::{load_json_config, save_json_config};
use crate::client;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub download_dir: String,
    pub classify_by_course: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            classify_by_course: true,
        }
    }
}

fn download_config_path() -> std::path::PathBuf {
    client::data_dir().join("download_config.json")
}

pub fn load_download_config() -> DownloadConfig {
    load_json_config(&download_config_path())
}

#[tauri::command]
pub fn get_download_config() -> DownloadConfig {
    load_download_config()
}

#[tauri::command]
pub fn save_download_config(config: DownloadConfig) -> Result<(), String> {
    if !config.download_dir.is_empty() {
        let p = std::path::Path::new(&config.download_dir);
        if !p.is_absolute() {
            return Err("ダウンロードディレクトリは絶対パスで指定してください".into());
        }
        std::fs::create_dir_all(p)
            .map_err(|e| format!("ディレクトリの作成に失敗しました: {}", e))?;
    }
    let classify = config.classify_by_course;
    save_json_config(&download_config_path(), &config, "download config")?;
    if classify {
        super::super::downloads::migrate_uncategorized_to_other();
    }
    Ok(())
}

#[tauri::command]
pub async fn select_download_dir() -> Result<String, String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("ダウンロードフォルダを選択")
        .pick_folder()
        .await;

    match result {
        Some(handle) => Ok(handle.path().to_string_lossy().to_string()),
        None => Err("cancelled".into()),
    }
}
