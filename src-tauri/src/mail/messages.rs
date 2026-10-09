use crate::config;

use super::types::{GraphListResponse, MailAttachment, MailDetail, MailMessage, MailProfile};

/// Validate a Graph API message ID.
/// Outlook item IDs are base64-like and can include path-sensitive characters,
/// so callers must URL-encode them before putting them in a Graph path segment.
pub(crate) fn validate_message_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 512
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_=.+/".contains(c))
    {
        return Err("無効なメッセージIDです".into());
    }
    Ok(())
}

fn encode_graph_path_segment(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

/// Validate a Graph API attachment ID.
fn validate_attachment_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 600
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_=.+/".contains(c))
    {
        return Err("無効な添付ファイルIDです".into());
    }
    Ok(())
}

impl super::MailClient {
    /// Fetch user's mail profile
    pub async fn fetch_profile(&mut self) -> Result<MailProfile, String> {
        let body = self
            .graph_get(&format!(
                "{}/me?$select=displayName,mail,userPrincipalName",
                config::GRAPH_BASE
            ))
            .await?;
        serde_json::from_value(body).map_err(|e| format!("プロフィール解析失敗: {}", e))
    }

    /// Fetch inbox messages
    pub async fn fetch_inbox(&mut self, top: u32, skip: u32) -> Result<Vec<MailMessage>, String> {
        let url = format!(
            "{}/me/mailFolders/inbox/messages?$top={}&$skip={}&$orderby=receivedDateTime desc&$select=id,subject,bodyPreview,body,from,receivedDateTime,isRead,hasAttachments",
            config::GRAPH_BASE, top, skip,
        );
        let body = self
            .graph_get_with_headers(&url, &[("Prefer", "outlook.body-content-type=\"text\"")])
            .await?;
        let resp: GraphListResponse<MailMessage> =
            serde_json::from_value(body).map_err(|e| format!("メール解析失敗: {}", e))?;
        Ok(resp.value)
    }

    /// Fetch a single message detail
    pub async fn fetch_message(&mut self, message_id: &str) -> Result<MailDetail, String> {
        validate_message_id(message_id)?;
        let encoded_message_id = encode_graph_path_segment(message_id);
        let url = format!(
            "{}/me/messages/{}?$select=id,subject,body,from,receivedDateTime,isRead,hasAttachments,toRecipients,ccRecipients",
            config::GRAPH_BASE, encoded_message_id,
        );
        let body = self.graph_get(&url).await?;
        serde_json::from_value(body).map_err(|e| format!("メール詳細解析失敗: {}", e))
    }

    /// Mark a message as read
    pub async fn mark_as_read(&mut self, message_id: &str) -> Result<(), String> {
        validate_message_id(message_id)?;
        let access_token = self.ensure_token().await?;
        let encoded_message_id = encode_graph_path_segment(message_id);
        let url = format!("{}/me/messages/{}", config::GRAPH_BASE, encoded_message_id);
        let body = serde_json::json!({"isRead": true});
        let resp = self
            .http
            .send(
                self.http
                    .client
                    .patch(&url)
                    .bearer_auth(&access_token)
                    .json(&body),
            )
            .await
            .map_err(|e| format!("既読設定失敗: {}", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text();
            log::warn!("mark_as_read failed: HTTP {} - {}", status, body);
            return Err(format!("既読設定失敗: HTTP {}", status));
        }
        Ok(())
    }

    /// Fetch attachment metadata for a message (no content bytes)
    pub async fn fetch_attachments(
        &mut self,
        message_id: &str,
    ) -> Result<Vec<MailAttachment>, String> {
        validate_message_id(message_id)?;
        let encoded_message_id = encode_graph_path_segment(message_id);
        let url = format!(
            "{}/me/messages/{}/attachments?$select=id,name,contentType,size",
            config::GRAPH_BASE,
            encoded_message_id,
        );
        let body = self.graph_get(&url).await?;
        let resp: GraphListResponse<MailAttachment> =
            serde_json::from_value(body).map_err(|e| format!("添付ファイル解析失敗: {}", e))?;
        Ok(resp.value)
    }

    /// Download a single attachment and save it to the Downloads folder.
    /// Returns the saved file path as a string.
    pub async fn download_attachment(
        &mut self,
        message_id: &str,
        attachment_id: &str,
        file_name: &str,
    ) -> Result<String, String> {
        validate_message_id(message_id)?;
        validate_attachment_id(attachment_id)?;
        let encoded_message_id = encode_graph_path_segment(message_id);

        // Sanitize file name: keep only the basename, replace dangerous chars
        let safe_name: String = std::path::Path::new(file_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || ".-_ ()[]".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe_name = if safe_name.is_empty() {
            "attachment".to_string()
        } else {
            safe_name
        };

        let url = format!(
            "{}/me/messages/{}/attachments/{}/$value",
            config::GRAPH_BASE,
            encoded_message_id,
            urlencoding::encode(attachment_id),
        );
        let downloads_dir = crate::commands::resolve_download_dir(None);
        let dest = downloads_dir.join(&safe_name);

        if dest.exists() {
            let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            let path_str = dest.to_string_lossy().to_string();
            crate::commands::record_download(&safe_name, &path_str, None, "mail", size);
            log::info!("Attachment already exists: {}", path_str);
            return Ok(path_str);
        }

        let data = self.graph_get_bytes(&url).await?;

        std::fs::write(&dest, &data).map_err(|e| format!("ファイル保存失敗: {}", e))?;

        let path_str = dest.to_string_lossy().to_string();
        crate::commands::record_download(&safe_name, &path_str, None, "mail", data.len() as u64);
        log::info!("Attachment saved to: {}", path_str);
        Ok(path_str)
    }
}
