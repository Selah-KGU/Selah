use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttModelInfo {
    pub id: String,
    pub name: String,
    pub size_label: String,
    pub archive_name: String,
    pub folder_name: String,
    pub download_url: String,
    pub file_size_mb: u64,
    pub model_file: String,
    pub tokens_file: String,
}

static STT_MODEL_CATALOG: LazyLock<Vec<SttModelInfo>> = LazyLock::new(|| {
    vec![SttModelInfo {
        id: "sensevoice-ja-en".into(),
        name: "SenseVoice 標準".into(),
        size_label: "228 MB".into(),
        archive_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".into(),
        folder_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17".into(),
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".into(),
        file_size_mb: 228,
        model_file: "model.int8.onnx".into(),
        tokens_file: "tokens.txt".into(),
    }]
});

pub fn stt_model_catalog() -> &'static [SttModelInfo] {
    &STT_MODEL_CATALOG
}

pub(in crate::stt) fn stt_models_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = crate::client::data_dir().join("models").join("stt");
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

pub(in crate::stt) fn stt_config_path() -> PathBuf {
    crate::client::data_dir().join("stt_config.json")
}

pub(in crate::stt) fn stt_model_dir(model: &SttModelInfo) -> PathBuf {
    stt_models_dir().join(&model.folder_name)
}

pub(in crate::stt) fn stt_archive_path(model: &SttModelInfo) -> PathBuf {
    stt_models_dir().join(&model.archive_name)
}

pub(in crate::stt) fn vad_model_path() -> PathBuf {
    stt_models_dir().join(VAD_MODEL_FILE)
}

pub(super) fn file_exists(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

fn file_exists_with_min_size(path: &Path, min_bytes: u64) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.len() >= min_bytes.max(1))
        .unwrap_or(false)
}

pub(super) fn model_files_ready(model: &SttModelInfo, dir: &Path) -> bool {
    let min_model_bytes = model.file_size_mb.saturating_mul(1024 * 1024);
    file_exists_with_min_size(&dir.join(&model.model_file), min_model_bytes)
        && file_exists(&dir.join(&model.tokens_file))
}

pub fn is_stt_model_downloaded(model: &SttModelInfo) -> bool {
    model_files_ready(model, &stt_model_dir(model)) && file_exists(&vad_model_path())
}

fn stt_model_missing_message(model: &SttModelInfo) -> String {
    let model_path = stt_model_dir(model).join(&model.model_file);
    let min_model_bytes = model.file_size_mb.saturating_mul(1024 * 1024);
    if file_exists(&model_path) && !file_exists_with_min_size(&model_path, min_model_bytes) {
        return format!(
            "{} のダウンロードが不完全です。削除して再ダウンロードしてください。",
            model.name
        );
    }
    format!(
        "{} がダウンロードされていません。設定画面からダウンロードしてください。",
        model.name
    )
}

pub(in crate::stt) fn ensure_stt_model_downloaded(model: &SttModelInfo) -> Result<(), String> {
    if is_stt_model_downloaded(model) {
        Ok(())
    } else {
        Err(stt_model_missing_message(model))
    }
}
