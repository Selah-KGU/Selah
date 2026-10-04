use super::*;

static STT_DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cancel_stt_download() {
    STT_DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}

fn emit_download_progress(app: &tauri::AppHandle, downloaded: u64, total: u64) {
    let _ = app.emit(
        "stt-model-download-progress",
        serde_json::json!({
            "downloaded": downloaded,
            "total": total,
            "percent": if total > 0 { (downloaded as f64 / total as f64 * 100.0) as u32 } else { 0 }
        }),
    );
}

fn download_file_blocking(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    progress_scale: f64,
    progress_offset: f64,
) -> Result<(), String> {
    let partial = dest.with_extension("part");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3600))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("ダウンロード開始失敗: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("ダウンロードエラー ({})", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file =
        std::fs::File::create(&partial).map_err(|e| format!("ファイル作成失敗: {}", e))?;
    let mut downloaded = 0u64;
    let mut last_emit = Instant::now();
    let mut buf = vec![0u8; 256 * 1024];

    loop {
        if STT_DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&partial);
            return Err("cancelled".into());
        }
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("ダウンロード読み取りエラー: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;
        downloaded += n as u64;
        if last_emit.elapsed() > Duration::from_millis(200) {
            let scaled_downloaded = (progress_offset + downloaded as f64 * progress_scale) as u64;
            let scaled_total = (progress_offset + total as f64 * progress_scale) as u64;
            emit_download_progress(app, scaled_downloaded, scaled_total);
            last_emit = Instant::now();
        }
    }

    file.flush()
        .map_err(|e| format!("ファイルフラッシュエラー: {}", e))?;
    std::fs::rename(&partial, dest).map_err(|e| format!("ファイルリネームエラー: {}", e))?;
    let scaled_downloaded = (progress_offset + total as f64 * progress_scale) as u64;
    let scaled_total = (progress_offset + total as f64 * progress_scale) as u64;
    emit_download_progress(app, scaled_downloaded, scaled_total);
    Ok(())
}

pub fn download_stt_model_blocking(
    app: &tauri::AppHandle,
    model: &SttModelInfo,
) -> Result<(), String> {
    STT_DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    let archive_path = stt_archive_path(model);
    let model_dir = stt_model_dir(model);
    if model_dir.exists() {
        let _ = std::fs::remove_dir_all(&model_dir);
    }
    let _ = std::fs::create_dir_all(stt_models_dir());

    download_file_blocking(app, &model.download_url, &archive_path, 0.98, 0.0)?;

    let archive_file =
        File::open(&archive_path).map_err(|e| format!("圧縮ファイルを開けません: {}", e))?;
    let decoder = BzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(stt_models_dir())
        .map_err(|e| format!("モデル展開失敗: {}", e))?;

    let vad_path = vad_model_path();
    if !vad_path.exists() {
        download_file_blocking(app, VAD_MODEL_URL, &vad_path, 0.02, 98.0)?;
    }

    let _ = app.emit(
        "stt-model-download-progress",
        serde_json::json!({
            "downloaded": 100,
            "total": 100,
            "percent": 100,
            "done": true
        }),
    );

    Ok(())
}
