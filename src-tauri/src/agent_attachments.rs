//! Document uploads are decoded on a worker and carried as typed text parts.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

mod extract;

pub(crate) const MAX_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_CHARS: usize = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPart {
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SavedDocuments {
    pub content: String,
    pub documents: Vec<DocumentPart>,
}

pub(crate) fn validate_documents(documents: &[DocumentPart]) -> Result<(), String> {
    if documents.len() > 4 {
        return Err("添付は最大4件までです".into());
    }
    for document in documents {
        if document.name.is_empty()
            || document.name.chars().count() > 255
            || document.size > MAX_BYTES
            || document.text.trim().is_empty()
            || document.text.chars().count() > MAX_CHARS
        {
            return Err("添付ファイルの内容またはサイズが正しくありません".into());
        }
    }
    Ok(())
}

pub(crate) fn model_content(content: &str, documents: &[DocumentPart]) -> String {
    let mut out = content.to_owned();
    for document in documents {
        out.push_str("\n\n[添付資料: ");
        out.push_str(&document.name);
        out.push_str("]\n");
        out.push_str(&document.text);
        if document.truncated {
            out.push_str("\n[この資料は一部のみ読み取られています]");
        }
        out.push_str("\n[添付資料ここまで]");
    }
    out
}

pub(crate) fn read_document(name: String, data_base64: &str) -> Result<DocumentPart, String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or("").to_owned();
    if name.is_empty() || name.chars().count() > 255 {
        return Err("ファイル名が正しくありません".into());
    }
    // Check encoded length before allocating the decoded buffer.
    if data_base64.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err("添付は1件10MBまでです".into());
    }
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|_| "添付データが正しくありません")?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("添付ファイルが空か、10MBを超えています".into());
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    let (mime, mut text, mut truncated) = extract::extract(&extension, &bytes)?;
    let boundary = text.char_indices().nth(MAX_CHARS).map(|(index, _)| index);
    if let Some(boundary) = boundary {
        text.truncate(boundary);
        truncated = true;
    }
    if text.trim().is_empty() {
        return Err("ファイルから本文を読み取れませんでした".into());
    }
    Ok(DocumentPart {
        name,
        mime: mime.into(),
        size: bytes.len(),
        text,
        truncated,
    })
}

#[tauri::command]
pub async fn agent_read_document_attachment(
    name: String,
    data_base64: String,
) -> Result<DocumentPart, String> {
    crate::background_ipc::run(
        "添付ファイルの読み取りに失敗しました",
        move || read_document(name, &data_base64),
    )
    .await
}

#[cfg(test)]
mod tests;
