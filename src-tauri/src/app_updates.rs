#[cfg(feature = "self-updater")]
pub fn init<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    app.plugin(tauri_plugin_updater::Builder::new().build())?;

    Ok(())
}

#[cfg(not(feature = "self-updater"))]
pub fn init<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let _ = app;
    Ok(())
}
