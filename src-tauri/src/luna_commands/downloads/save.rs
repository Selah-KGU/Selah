/// Save bytes to the download folder. In materials-management semantics,
/// an existing file with the same name in the same course folder means it has
/// already been downloaded, so we reuse it instead of creating " (1)" copies.
/// If course_name is provided and classify_by_course is enabled, saves into a course subfolder.
pub(crate) fn save_to_downloads(
    filename: &str,
    bytes: &[u8],
    course_name: Option<&str>,
) -> Result<String, String> {
    let downloads = crate::commands::resolve_download_dir(course_name);
    let _ = std::fs::create_dir_all(&downloads);
    let save_path = downloads.join(filename);

    if save_path.exists() {
        let size = std::fs::metadata(&save_path)
            .map(|m| m.len())
            .unwrap_or(bytes.len() as u64);
        let path_str = save_path.to_string_lossy().to_string();
        crate::commands::record_download(filename, &path_str, course_name, "luna", size);
        return Ok(path_str);
    }

    std::fs::write(&save_path, bytes).map_err(|e| format!("ファイル保存失敗: {}", e))?;

    let path_str = save_path.to_string_lossy().to_string();
    crate::commands::record_download(filename, &path_str, course_name, "luna", bytes.len() as u64);

    Ok(path_str)
}

/// application/x-www-form-urlencoded: space -> +, encode other special chars.
pub(crate) fn form_encode(s: &str) -> String {
    let mut result = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || "-._~".contains(ch) {
            result.push(ch);
        } else if ch == ' ' {
            result.push('+');
        } else {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            for b in s.bytes() {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// Replicate Luna's CommonUtil.makeDownFileName JS function:
/// replace fullwidth/halfwidth spaces with _, collapse multiple _, then encodeURI
pub(crate) fn make_down_file_name(file_name: &str) -> String {
    let mut result = file_name.replace(['\u{3000}', ' '], "_");
    while result.contains("__") {
        result = result.replace("__", "_");
    }

    let mut encoded = String::new();
    for ch in result.chars() {
        if ch.is_ascii_alphanumeric() || "-_.!~*'()".contains(ch) || ";,/?:@&=+$#".contains(ch) {
            encoded.push(ch);
        } else {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            for b in s.bytes() {
                encoded.push_str(&format!("%{:02X}", b));
            }
        }
    }
    encoded
}
