#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2::{AnyThread, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSSharingServicePicker, NSView};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSRectEdge, NSURL};
#[cfg(target_os = "macos")]
use tauri::Manager;

#[cfg(target_os = "macos")]
pub(in crate::commands::app_config) fn open_macos_share_picker(
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
