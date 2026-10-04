//! Tauri commands for speech-to-text config, models, and streaming.

use super::*;

#[tauri::command]
pub fn get_stt_config() -> SttConfig {
    load_config()
}

#[tauri::command]
pub fn save_stt_config(app: tauri::AppHandle, mut config: SttConfig) -> Result<(), String> {
    config.selected_model = normalize_stt_model_id(&config.selected_model);
    config.language = normalize_stt_language(&config.language);
    config.execution_backend = validate_stt_execution_backend(&config.execution_backend)?;
    config.partial_mode = normalize_stt_partial_mode(&config.partial_mode);
    config.sensitivity = normalize_stt_sensitivity(&config.sensitivity);
    if stt_model_catalog()
        .iter()
        .all(|m| m.id != config.selected_model)
    {
        return Err("不明な STT モデルです".into());
    }
    save_config(&config)?;
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn list_stt_execution_backends() -> Vec<SttExecutionBackendInfo> {
    stt_execution_backend_catalog()
}

#[tauri::command]
pub fn list_stt_models() -> Vec<serde_json::Value> {
    stt_model_catalog()
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "size_label": m.size_label,
                "file_size_mb": m.file_size_mb,
                "downloaded": is_stt_model_downloaded(m),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn download_stt_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let model = stt_model_catalog()
        .iter()
        .find(|m| m.id == model_id)
        .cloned()
        .ok_or_else(|| format!("不明な STT モデル: {}", model_id))?;
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || download_stt_model_blocking(&app_clone, &model))
        .await
        .map_err(|e| format!("タスク実行エラー: {}", e))??;
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn delete_stt_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let model = stt_model_catalog()
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("不明な STT モデル: {}", model_id))?;

    let model_dir = stt_model_dir(model);
    if model_dir.exists() {
        std::fs::remove_dir_all(&model_dir).map_err(|e| format!("削除失敗: {}", e))?;
    }
    let archive = stt_archive_path(model);
    if archive.exists() {
        let _ = std::fs::remove_file(&archive);
    }
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn cancel_stt_model_download() {
    cancel_stt_download();
}

#[tauri::command]
pub fn stt_test_model(app: tauri::AppHandle) -> Result<String, String> {
    let model = selected_model_from_config()?;
    ensure_stt_model_downloaded(&model)?;
    if !is_stt_model_downloaded(&model) {
        return Err("STT モデルを先にダウンロードしてください".into());
    }
    let recognizer_init = create_recognizer_with_fallback(&model)?;
    update_runtime_debug_state(
        "test-ok",
        None,
        Some(&recognizer_init.execution_backend),
        recognizer_init.fallback_from.as_deref(),
    );
    let _recognizer = recognizer_init.recognizer;

    if let Some(fallback_from) = recognizer_init.fallback_from {
        let message = format!(
            "OK: {} ({}) / {}",
            model.name,
            stt_execution_backend_label(&recognizer_init.execution_backend),
            stt_fallback_message(&fallback_from)
        );
        update_runtime_debug_message(Some(message.clone()), None);
        emit_runtime_debug_changed(&app);
        return Ok(message);
    }

    let message = format!(
        "OK: {} ({})",
        model.name,
        stt_execution_backend_label(&recognizer_init.execution_backend)
    );
    update_runtime_debug_message(Some(message.clone()), None);
    emit_runtime_debug_changed(&app);
    Ok(message)
}

#[tauri::command]
pub fn stt_is_running() -> bool {
    STT_SESSION.lock().map(|s| s.is_some()).unwrap_or(false)
}

#[tauri::command]
pub fn stt_get_active_caller() -> Option<String> {
    STT_SESSION
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|sess| sess.caller.clone()))
}

#[tauri::command]
pub fn stt_start_stream(
    app: tauri::AppHandle,
    caller: String,
    preempt: Option<bool>,
) -> Result<Option<String>, String> {
    STT_SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    let caller = if caller.is_empty() {
        "unknown".to_string()
    } else {
        caller
    };
    let mut lock = STT_SESSION
        .lock()
        .map_err(|_| "STT state lock failed".to_string())?;

    let previous_caller = if let Some(session) = lock.as_ref() {
        if preempt.unwrap_or(false) {
            let prev = session.caller.clone();
            let _ = session.stop_tx.send(());
            *lock = None;
            // Give the previous session a moment to clean up
            Some(prev)
        } else {
            return Err(format!("音声入力は「{}」で使用中です", session.caller));
        }
    } else {
        None
    };

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
    *lock = Some(ActiveSttSession {
        id: session_id,
        caller: caller.clone(),
        stop_tx,
    });
    drop(lock);

    let app_clone = app.clone();
    std::thread::spawn(move || run_stt_session(app_clone, session_id, stop_rx, &caller));
    Ok(previous_caller)
}

#[tauri::command]
pub fn stt_stop_stream() -> Result<(), String> {
    let mut lock = STT_SESSION
        .lock()
        .map_err(|_| "STT state lock failed".to_string())?;
    if let Some(session) = lock.take() {
        let _ = session.stop_tx.send(());
    }
    Ok(())
}

pub(crate) fn stt_shutdown_for_exit(timeout: Duration) {
    STT_SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    let had_session = if let Ok(lock) = STT_SESSION.lock() {
        if let Some(session) = lock.as_ref() {
            let _ = session.stop_tx.send(());
            true
        } else {
            false
        }
    } else {
        log::warn!("[stt] shutdown: STT state lock failed");
        false
    };

    if !had_session {
        STT_SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
        return;
    }

    let start = Instant::now();
    while start.elapsed() < timeout {
        if !stt_is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    log::warn!("[stt] shutdown: timed out waiting for STT session to stop");
}
