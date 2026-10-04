//! OS and app theme detection for the subtitle overlay.

use super::*;
use tauri::Manager;

pub(super) fn prefers_dark(app: &AppHandle) -> bool {
    let theme = app.state::<crate::ThemeState>();
    let mode = theme.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match mode.as_str() {
        "light" => false,
        "dark" => true,
        // "system" (or anything unknown) follows the OS apps theme.
        _ => system_apps_use_dark_theme(),
    }
}

/// Reads HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize
/// `AppsUseLightTheme` (DWORD): 0 = dark, 1 = light. Defaults to dark when
/// the value is missing or unreadable, matching the previous behaviour.
fn system_apps_use_dark_theme() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_DWORD,
    };

    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let mut hkey: HKEY = std::ptr::null_mut();
    let open = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if open != ERROR_SUCCESS {
        return true;
    }

    let value_name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut data: u32 = 0;
    let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
    let mut data_type: u32 = 0;
    let result = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut data_type,
            &mut data as *mut u32 as *mut u8,
            &mut data_size,
        )
    };
    let _ = unsafe { RegCloseKey(hkey) };
    if result != ERROR_SUCCESS || data_type != REG_DWORD {
        return true;
    }
    data == 0
}
