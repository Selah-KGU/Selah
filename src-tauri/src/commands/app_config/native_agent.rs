use super::json::{load_json_config, save_json_config};
use crate::client;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NativeAgentConfig {
    #[serde(alias = "floating_orb_enabled")]
    pub voice_shortcut_enabled: bool,
    pub voice_shortcut: String,
    pub subtitle_overlay_enabled: bool,
}

impl Default for NativeAgentConfig {
    fn default() -> Self {
        Self {
            voice_shortcut_enabled: false,
            voice_shortcut: if cfg!(target_os = "windows") {
                "lalt".into()
            } else {
                "fn".into()
            },
            subtitle_overlay_enabled: false,
        }
    }
}

fn native_agent_config_path() -> std::path::PathBuf {
    client::data_dir().join("native_agent_config.json")
}

pub fn load_native_agent_config() -> NativeAgentConfig {
    load_json_config(&native_agent_config_path())
}

#[tauri::command]
pub fn get_native_agent_config() -> NativeAgentConfig {
    load_native_agent_config()
}

#[tauri::command]
pub fn save_native_agent_config(
    _app: tauri::AppHandle,
    config: NativeAgentConfig,
) -> Result<(), String> {
    save_json_config(&native_agent_config_path(), &config, "native agent config")?;

    #[cfg(target_os = "macos")]
    {
        // Propagate shortcut registration failures so the UI can tell the
        // user their chosen combination conflicts (e.g. with a system
        // shortcut) rather than silently leaving them without a working
        // hotkey.
        crate::macos_native_agent::apply_config(&_app, &config)?;
        if config.subtitle_overlay_enabled {
            let _ = crate::macos_subtitle_overlay::open_overlay(&_app);
        } else {
            let _ = crate::macos_subtitle_overlay::close_overlay(&_app);
        }
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_native_agent::apply_config(&_app, &config)?;
        if config.subtitle_overlay_enabled {
            crate::windows_subtitle_overlay::open_overlay(&_app)?;
        } else {
            crate::windows_subtitle_overlay::close_overlay(&_app)?;
        }
    }

    Ok(())
}

/// Open the real-time subtitle floating overlay and start STT.
#[tauri::command]
pub fn open_subtitle_overlay(_app: tauri::AppHandle) -> Result<(), String> {
    if !load_native_agent_config().subtitle_overlay_enabled {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_subtitle_overlay::open_overlay(&_app)
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::open_overlay(&_app);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("Real-time subtitle overlay is not supported on this platform".into())
}

/// Stop STT and close the subtitle overlay.
#[tauri::command]
pub fn close_subtitle_overlay(_app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::macos_subtitle_overlay::close_overlay(&_app)
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::close_overlay(&_app);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(())
}

/// Returns whether the subtitle overlay is currently open.
#[tauri::command]
pub fn subtitle_overlay_is_open() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos_subtitle_overlay::is_open()
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::is_open();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}
