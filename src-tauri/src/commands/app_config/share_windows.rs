#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
static WINDOWS_SHARE_RO_INIT: std::sync::Once = std::sync::Once::new();

#[cfg(target_os = "windows")]
pub(in crate::commands::app_config) fn open_windows_file_share_picker(
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
            WINDOWS_SHARE_RO_INIT.call_once(|| {
                let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            });

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
