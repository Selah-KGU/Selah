use crate::client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2::{AnyThread, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSSharingServicePicker, NSView};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSRectEdge, NSURL};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::Manager;
#[cfg(target_os = "windows")]
use windows::core::HSTRING;
#[cfg(target_os = "windows")]
use windows::ApplicationModel::DataTransfer::{
    DataPackageOperation, DataRequestedEventArgs, DataTransferManager,
};
#[cfg(target_os = "windows")]
use windows::Foundation::TypedEventHandler;
#[cfg(target_os = "windows")]
use windows::Storage::{IStorageItem, StorageFile};
#[cfg(target_os = "windows")]
use windows::Win32::System::WinRT::{RoGetActivationFactory, RoInitialize, RO_INIT_MULTITHREADED};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::IDataTransferManagerInterop;
#[cfg(target_os = "windows")]
use windows_core::Interface;

#[cfg(target_os = "windows")]
struct SendSyncDtm(DataTransferManager, i64);
#[cfg(target_os = "windows")]
unsafe impl Send for SendSyncDtm {}
#[cfg(target_os = "windows")]
unsafe impl Sync for SendSyncDtm {}

#[cfg(target_os = "windows")]
static WINDOWS_SHARE_HANDLER: std::sync::LazyLock<std::sync::Mutex<Option<SendSyncDtm>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

fn load_json_config<T: Default + DeserializeOwned>(path: &std::path::Path) -> T {
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    T::default()
}

fn save_json_config<T: Serialize>(
    path: &std::path::Path,
    config: &T,
    label: &str,
) -> Result<(), String> {
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(path, &data).map_err(|e| format!("Failed to write {}: {}", label, e))?;
    Ok(())
}

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
        super::downloads::migrate_uncategorized_to_other();
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub notify_important: bool,
    pub notify_faculty: bool,
    pub notify_class: bool,
    pub notify_class_general: bool,
    pub notify_class_announcement: bool,
    pub notify_class_assignment: bool,
    pub notify_class_exam: bool,
    pub notify_class_discussion: bool,
    pub notify_class_survey: bool,
    pub notify_class_attendance: bool,
    pub notify_other: bool,
    pub notify_mail: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notify_important: true,
            notify_faculty: true,
            notify_class: true,
            notify_class_general: true,
            notify_class_announcement: true,
            notify_class_assignment: true,
            notify_class_exam: true,
            notify_class_discussion: true,
            notify_class_survey: true,
            notify_class_attendance: true,
            notify_other: true,
            notify_mail: true,
        }
    }
}

fn notification_config_path() -> std::path::PathBuf {
    client::data_dir().join("notification_config.json")
}

pub fn load_notification_config() -> NotificationConfig {
    load_json_config(&notification_config_path())
}

#[tauri::command]
pub fn get_notification_config() -> NotificationConfig {
    load_notification_config()
}

#[tauri::command]
pub fn save_notification_config(config: NotificationConfig) -> Result<(), String> {
    save_json_config(&notification_config_path(), &config, "notification config")
}

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
        return crate::macos_subtitle_overlay::open_overlay(&_app);
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
        return crate::macos_subtitle_overlay::close_overlay(&_app);
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
        return crate::macos_subtitle_overlay::is_open();
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::is_open();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarConfig {
    pub spring_start: String,
    pub fall_start: String,
    pub syscal_enabled: bool,
    pub syscal_auto_sync: bool,
    pub gcal_auto_sync: bool,
    pub cal_sync_interval: u32,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            spring_start: String::new(),
            fall_start: String::new(),
            syscal_enabled: false,
            syscal_auto_sync: false,
            gcal_auto_sync: false,
            cal_sync_interval: 12,
        }
    }
}

fn calendar_config_path() -> std::path::PathBuf {
    client::data_dir().join("calendar_config.json")
}

pub fn load_calendar_config() -> CalendarConfig {
    load_json_config(&calendar_config_path())
}

#[tauri::command]
pub fn get_calendar_config() -> CalendarConfig {
    load_calendar_config()
}

#[tauri::command]
pub fn save_calendar_config(config: CalendarConfig) -> Result<(), String> {
    for (label, val) in [
        ("春学期開始日", &config.spring_start),
        ("秋学期開始日", &config.fall_start),
    ] {
        if !val.is_empty() && chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d").is_err() {
            return Err(format!("{}の日付形式が不正です (YYYY-MM-DD)", label));
        }
    }
    save_json_config(&calendar_config_path(), &config, "calendar config")
}

// ============ Image Share ============

/// Save PNG image data to a file using the native file dialog.
#[tauri::command]
pub async fn save_image_file(data: Vec<u8>, default_name: String) -> Result<String, String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("時間割画像を保存")
        .set_file_name(&default_name)
        .add_filter("PNG画像", &["png"])
        .save_file()
        .await;

    match result {
        Some(handle) => {
            let path = handle.path().to_path_buf();
            std::fs::write(&path, &data).map_err(|e| format!("保存に失敗しました: {}", e))?;
            Ok(path.to_string_lossy().to_string())
        }
        None => Err("cancelled".into()),
    }
}

/// Copy PNG image data to the system clipboard using native APIs.
#[tauri::command]
pub async fn copy_image_to_clipboard(data: Vec<u8>) -> Result<(), String> {
    // Write to temp file
    let tmp_dir = std::env::temp_dir().join("selah-share");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("一時ディレクトリの作成に失敗: {}", e))?;
    let tmp_path = tmp_dir.join("clipboard_tmp.png");
    std::fs::write(&tmp_path, &data).map_err(|e| format!("一時ファイルの書き込みに失敗: {}", e))?;

    let result = {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            let script = format!(
                r#"use framework "AppKit"
use framework "Foundation"
set theImage to current application's NSImage's alloc()'s initWithContentsOfFile:"{}"
set pb to current application's NSPasteboard's generalPasteboard()
pb's clearContents()
pb's writeObjects:(current application's NSArray's arrayWithObject:theImage)"#,
                tmp_path.display()
            );
            Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .map_err(|e| format!("クリップボードへのコピーに失敗: {}", e))
                .and_then(|out| {
                    if out.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "クリップボードへのコピーに失敗: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ))
                    }
                })
        }

        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            let path_str = tmp_path.to_string_lossy().replace('\'', "''");
            let script = format!(
                r#"Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $img = [System.Drawing.Image]::FromFile('{}'); [System.Windows.Forms.Clipboard]::SetImage($img); $img.Dispose()"#,
                path_str
            );
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .output()
                .map_err(|e| format!("クリップボードへのコピーに失敗: {}", e))
                .and_then(|out| {
                    if out.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "クリップボードへのコピーに失敗: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ))
                    }
                })
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err::<(), String>("この OS ではクリップボードコピーはサポートされていません".into())
        }
    };

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp_path);

    result
}

/// Share PNG image data via the native OS share sheet.
/// On macOS opens the native share picker.
/// Temp files are cleaned up after the share UI has had time to consume them.
#[tauri::command]
pub async fn share_image_native(
    app: tauri::AppHandle,
    data: Vec<u8>,
    file_name: String,
) -> Result<(), String> {
    // Write to a temp file
    let tmp_dir = std::env::temp_dir().join("selah-share");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("一時ディレクトリの作成に失敗: {}", e))?;
    let tmp_path = tmp_dir.join(&file_name);
    std::fs::write(&tmp_path, &data).map_err(|e| format!("一時ファイルの書き込みに失敗: {}", e))?;
    let result = share_file_path_native(&app, &tmp_path, &file_name);

    // Keep the file around long enough for slower share targets to read it.
    let cleanup_path = tmp_path.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(600));
        let _ = std::fs::remove_file(&cleanup_path);
        // Try to remove the directory if empty
        let _ = std::fs::remove_dir(cleanup_path.parent().unwrap_or(std::path::Path::new("")));
    });

    result
}

pub(crate) fn share_file_path_native(
    app: &tauri::AppHandle,
    path: &std::path::Path,
    file_name: &str,
) -> Result<(), String> {
    share_file_paths_native(app, &[(path.to_path_buf(), file_name.to_string())])
}

pub(crate) fn share_file_paths_native(
    app: &tauri::AppHandle,
    files: &[(std::path::PathBuf, String)],
) -> Result<(), String> {
    if files.is_empty() {
        return Err("共有するファイルが選択されていません".into());
    }
    for (path, _) in files {
        if !path.is_file() {
            return Err("共有するファイルが見つかりません".into());
        }
    }

    #[cfg(target_os = "macos")]
    {
        return open_macos_share_picker(app, files);
    }

    #[cfg(target_os = "windows")]
    {
        return open_windows_file_share_picker(app, files);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        let _ = files;
        Err("この OS では共有機能はサポートされていません".into())
    }
}

#[cfg(target_os = "windows")]
fn open_windows_file_share_picker(
    app: &tauri::AppHandle,
    files: &[(std::path::PathBuf, String)],
) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "共有元のウィンドウが見つかりません".to_string())?;
    let hwnd_raw = window
        .hwnd()
        .map_err(|e| format!("Windows 共有ウィンドウの取得に失敗しました: {}", e))?
        .0 as isize;
    drop(window);

    let share_files: Vec<(String, String)> = files
        .iter()
        .map(|(path, file_name)| {
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(file_name)
                .to_string();
            (path.to_string_lossy().to_string(), title)
        })
        .collect();
    let title = if share_files.len() == 1 {
        share_files[0].1.clone()
    } else {
        format!("{}件のファイル", share_files.len())
    };
    let (tx, rx) = std::sync::mpsc::channel();

    app.run_on_main_thread(move || {
        let result: Result<(), String> = (|| {
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

            let title_for_handler = title.clone();
            let files_for_handler = share_files.clone();
            let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(
                move |_, args| {
                    if let Some(args) = args.as_ref() {
                        let mut storage_items = Vec::with_capacity(files_for_handler.len());
                        for (path, _) in &files_for_handler {
                            let file =
                                StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_str()))?
                                    .get()?;
                            let item: IStorageItem = file.cast()?;
                            storage_items.push(Some(item));
                        }
                        let items: windows_collections::IIterable<IStorageItem> =
                            storage_items.into();
                        let request = args.Request()?;
                        let data = request.Data()?;
                        let properties = data.Properties()?;
                        properties.SetTitle(&HSTRING::from(title_for_handler.as_str()))?;
                        properties.SetDescription(&HSTRING::from("Selah file"))?;
                        data.SetStorageItems(&items, true)?;
                        data.SetRequestedOperation(DataPackageOperation::Copy)?;
                    }
                    Ok(())
                },
            );

            let interop: IDataTransferManagerInterop = unsafe {
                RoGetActivationFactory(&HSTRING::from(
                    "Windows.ApplicationModel.DataTransfer.DataTransferManager",
                ))
            }
            .map_err(|e| format!("Windows 共有機能の初期化に失敗しました: {}", e))?;
            let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }
                .map_err(|e| format!("Windows 共有マネージャーの取得に失敗しました: {}", e))?;
            let token = manager
                .DataRequested(&handler)
                .map_err(|e| format!("共有データの登録に失敗しました: {}", e))?;

            if let Ok(mut previous) = WINDOWS_SHARE_HANDLER.lock() {
                if let Some(SendSyncDtm(old_manager, old_token)) = previous.take() {
                    let _ = old_manager.RemoveDataRequested(old_token);
                }
                *previous = Some(SendSyncDtm(manager.clone(), token));
            }

            unsafe { interop.ShowShareUIForWindow(hwnd) }
                .map_err(|e| format!("Windows 共有 UI の表示に失敗しました: {}", e))?;
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| format!("Windows 共有 UI の起動に失敗しました: {}", e))?;

    rx.recv()
        .map_err(|_| "Windows 共有 UI の結果受信に失敗しました".to_string())??;
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_macos_share_picker(
    app: &tauri::AppHandle,
    files: &[(std::path::PathBuf, String)],
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "共有元のウィンドウが見つかりません".to_string())?;
    let window_for_main = window.clone();
    let paths: Vec<std::path::PathBuf> = files.iter().map(|(path, _)| path.clone()).collect();
    let (tx, rx) = std::sync::mpsc::channel();

    window
        .run_on_main_thread(move || {
            let result: Result<(), String> = (|| {
                let mtm = MainThreadMarker::new().ok_or_else(|| {
                    "共有ピッカーを主スレッドで初期化できませんでした".to_string()
                })?;
                let ns_view_ptr = window_for_main
                    .ns_view()
                    .map_err(|e| format!("共有ビューの取得に失敗: {}", e))?;
                if ns_view_ptr.is_null() {
                    return Err("共有ビューが無効です".into());
                }

                let view = unsafe { &*(ns_view_ptr as *mut NSView) };
                let file_urls = paths
                    .iter()
                    .map(|path| {
                        NSURL::from_file_path(path)
                            .ok_or_else(|| "共有用ファイル URL の作成に失敗しました".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let share_items: Vec<&AnyObject> = file_urls
                    .iter()
                    .map(|url| unsafe { &*(&**url as *const NSURL as *const AnyObject) })
                    .collect();
                let items = NSArray::from_slice(&share_items);
                let picker = unsafe {
                    let _ = mtm;
                    NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items)
                };

                picker.showRelativeToRect_ofView_preferredEdge(
                    view.bounds(),
                    view,
                    NSRectEdge::MinY,
                );
                Ok(())
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("共有ピッカーの起動に失敗: {}", e))?;

    rx.recv()
        .map_err(|_| "共有ピッカーの結果受信に失敗しました".to_string())??;
    Ok(())
}
