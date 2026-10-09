use super::*;

#[path = "download/install.rs"]
mod install;
use install::{check_canceled, copy_download, extract_model, ModelStaging};

static STT_DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cancel_stt_download() {
    STT_DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}

fn download_canceled() -> bool {
    STT_DOWNLOAD_CANCEL.load(Ordering::SeqCst) || STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

fn transfer_percent(downloaded: u64, total: u64, scale: f64, offset: f64) -> u32 {
    let fraction = if total == 0 {
        0.0
    } else {
        downloaded.min(total) as f64 / total as f64
    };
    // Reserve 100% for validated files successfully placed in the model store.
    (offset + fraction * scale * 100.0).clamp(0.0, 99.0) as u32
}

fn emit_download_progress(app: &tauri::AppHandle, downloaded: u64, total: u64, percent: u32) {
    let _ = app.emit(
        "stt-model-download-progress",
        serde_json::json!({ "downloaded": downloaded, "total": total, "percent": percent }),
    );
}

fn download_file_blocking(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    progress_scale: f64,
    progress_offset: f64,
) -> Result<(), String> {
    check_canceled(&download_canceled)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3600))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let response = client.get(url).send().map_err(|e| {
        if download_canceled() {
            "cancelled".into()
        } else {
            format!("ダウンロード開始失敗: {e}")
        }
    })?;
    check_canceled(&download_canceled)?;
    if !response.status().is_success() {
        return Err(format!("ダウンロードエラー ({})", response.status()));
    }
    let expected = response.content_length();
    let total = expected.unwrap_or(0);
    // Every destination is inside this operation's private staging directory.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|e| format!("ファイル作成失敗: {e}"))?;
    let mut last_emit = Instant::now();
    let downloaded = copy_download(
        response,
        &mut file,
        expected,
        &download_canceled,
        |downloaded| {
            if last_emit.elapsed() > Duration::from_millis(200) {
                emit_download_progress(
                    app,
                    downloaded,
                    total,
                    transfer_percent(downloaded, total, progress_scale, progress_offset),
                );
                last_emit = Instant::now();
            }
        },
    )?;
    emit_download_progress(
        app,
        downloaded,
        total,
        transfer_percent(downloaded, total, progress_scale, progress_offset),
    );
    Ok(())
}

pub fn download_stt_model_blocking(
    app: &tauri::AppHandle,
    model: &SttModelInfo,
) -> Result<(), String> {
    let _reservation = super::commands::reserve_model_operation()?;
    STT_DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);
    let mut staging = ModelStaging::new(stt_models_dir())?;
    let archive_path = staging.root.join("model.tar.bz2");
    download_file_blocking(app, &model.download_url, &archive_path, 0.98, 0.0)?;
    let archive_file =
        File::open(&archive_path).map_err(|e| format!("圧縮ファイルを開けません: {e}"))?;
    let unpacked = staging.root.join("unpacked");
    extract_model(
        BzDecoder::new(archive_file),
        &unpacked,
        model,
        &download_canceled,
    )?;
    let mut replacements = vec![(unpacked.join(&model.folder_name), stt_model_dir(model))];
    let vad_path = vad_model_path();
    if !super::model::file_exists(&vad_path) {
        let staged_vad = staging.root.join(VAD_MODEL_FILE);
        download_file_blocking(app, VAD_MODEL_URL, &staged_vad, 0.02, 98.0)?;
        replacements.push((staged_vad, vad_path));
    }
    // Cancellation remains effective during decompression and preparation.
    // Publication itself completes or rolls back while starts remain blocked.
    check_canceled(&download_canceled)?;
    staging.publish(&replacements)?;
    // Compressed input is temporary. Remove an obsolete archive from older
    // builds only after successfully installing the complete model.
    let old_archive = stt_archive_path(model);
    if old_archive.exists() {
        if let Err(error) = std::fs::remove_file(&old_archive) {
            log::warn!("[stt] obsolete model archive cleanup failed: {error}");
        }
    }
    drop(staging);
    let _ = app.emit(
        "stt-model-download-progress",
        serde_json::json!({ "percent": 100, "done": true }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::transfer_percent;

    #[test]
    fn progress_uses_transfer_bytes_and_reserves_completion_for_publication() {
        assert_eq!(transfer_percent(50, 100, 0.98, 0.0), 49);
        assert_eq!(transfer_percent(100, 100, 0.98, 0.0), 98);
        assert_eq!(transfer_percent(50, 100, 0.02, 98.0), 99);
        assert_eq!(transfer_percent(100, 100, 0.02, 98.0), 99);
        assert_eq!(transfer_percent(50, 0, 0.02, 98.0), 98);
    }
}
