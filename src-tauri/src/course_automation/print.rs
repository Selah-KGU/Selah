//! Print dispatch for SenseA course automation.
//!
//! Locates an already-downloaded file, gates it on category approval, and
//! sends it to the default printer. Status persistence stays in the parent.

use super::*;

pub(super) async fn process_print_candidates(
    app: &AppHandle,
    db: &Database,
    status: &mut CourseAutomationStatus,
    downloaded: &[(String, String, String, PathBuf, String)],
    candidates: &[PrintCandidate],
    approved_categories: &[String],
) -> Result<Vec<PrintResult>, String> {
    let mut results = Vec::new();
    for candidate in candidates {
        let category = candidate.category.trim().to_string();
        let category_key = normalize_category(&category);
        if candidate.confidence < PRINT_CONFIDENCE_THRESHOLD {
            results.push(PrintResult {
                filename: candidate.filename.clone(),
                status: "skipped_low_confidence".into(),
                detail: format!("confidence={:.2}", candidate.confidence),
                category,
                ..Default::default()
            });
            continue;
        }
        let Some((filename, path, source_fingerprint)) =
            locate_print_candidate(downloaded, &status.artifacts, &candidate.filename)
        else {
            results.push(PrintResult {
                filename: candidate.filename.clone(),
                status: "not_found".into(),
                detail: "Agent が指定したダウンロード済みファイルを特定できません".into(),
                category,
                ..Default::default()
            });
            continue;
        };
        let path_text = path.to_string_lossy().to_string();
        let action_key = print_action_key(&filename, &path_text, &source_fingerprint, &category);
        let candidate_result = PrintResult {
            action_key: action_key.clone(),
            filename: filename.clone(),
            path: path_text.clone(),
            category: category.clone(),
            ..Default::default()
        };
        if status
            .print_results
            .iter()
            .any(|item| item.status == "printed" && print_results_match(item, &candidate_result))
        {
            results.push(PrintResult {
                action_key,
                filename: filename.clone(),
                path: path_text,
                status: "already_printed".into(),
                detail: "同じ SenseA 履歴で印刷済みです".into(),
                category,
            });
            continue;
        }
        if let Some(existing) = status
            .print_results
            .iter()
            .find(|item| item.status == "unknown" && print_results_match(item, &candidate_result))
        {
            results.push(existing.clone());
            continue;
        }
        if status.print_results.iter().any(|item| {
            item.status == "dispatching" && print_results_match(item, &candidate_result)
        }) {
            let unknown = unknown_print_result(&filename, path_text, category, action_key);
            status.print_results =
                merge_print_results(&status.print_results, vec![unknown.clone()]);
            save_status_and_emit(app, db, status)?;
            results.push(unknown);
            continue;
        }
        // Gate by per-category approval: an un-approved type waits for the user
        // to confirm once; an approved type prints automatically from then on.
        let approved = !category_key.is_empty()
            && approved_categories
                .iter()
                .any(|item| normalize_category(item) == category_key);
        if !approved {
            results.push(PrintResult {
                action_key,
                filename: filename.clone(),
                path: path_text,
                status: "needs_confirmation".into(),
                detail: "このタイプの印刷を許可すると、以降の同種ファイルは自動で印刷されます"
                    .into(),
                category,
            });
            continue;
        }
        let dispatching = dispatching_print_result(
            &filename,
            path_text.clone(),
            category.clone(),
            action_key.clone(),
        );
        status.print_results = merge_print_results(&status.print_results, vec![dispatching]);
        save_status_and_emit(app, db, status)?;
        let result = print_one(&filename, &path, path_text, category, action_key).await;
        status.print_results = merge_print_results(&status.print_results, vec![result.clone()]);
        save_status_and_emit(app, db, status)?;
        results.push(result);
    }
    Ok(results)
}

pub(super) fn locate_print_candidate(
    downloaded: &[(String, String, String, PathBuf, String)],
    artifacts: &[CourseArtifactRecord],
    candidate_filename: &str,
) -> Option<(String, PathBuf, String)> {
    downloaded
        .iter()
        .find(|(_, _, filename, path, _)| {
            print_candidate_matches(filename, path, candidate_filename)
        })
        .map(|(_, _, filename, path, source_fingerprint)| {
            (filename.clone(), path.clone(), source_fingerprint.clone())
        })
        .or_else(|| {
            artifacts
                .iter()
                .find(|artifact| {
                    artifact.status == "downloaded"
                        && !artifact.path.is_empty()
                        && print_candidate_matches(
                            &artifact.filename,
                            Path::new(&artifact.path),
                            candidate_filename,
                        )
                        && Path::new(&artifact.path).is_file()
                })
                .map(|artifact| {
                    (
                        artifact.filename.clone(),
                        PathBuf::from(&artifact.path),
                        artifact.source_fingerprint.clone(),
                    )
                })
        })
}

fn print_candidate_matches(filename: &str, path: &Path, candidate_filename: &str) -> bool {
    filename == candidate_filename
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == candidate_filename)
}

/// Sends one already-located file to the printer and maps the outcome to a
/// `PrintResult`. Shared by the automatic path and the manual confirm command.
pub(super) async fn print_one(
    filename: &str,
    path: &Path,
    path_text: String,
    category: String,
    action_key: String,
) -> PrintResult {
    let path = path.to_path_buf();
    let result = tauri::async_runtime::spawn_blocking(move || print_verified(&path)).await;
    match result {
        Ok(Ok(detail)) => PrintResult {
            action_key,
            filename: filename.to_string(),
            path: path_text,
            status: "printed".into(),
            detail,
            category,
        },
        Ok(Err(error)) => PrintResult {
            action_key,
            filename: filename.to_string(),
            path: path_text,
            status: "error".into(),
            detail: error,
            category,
        },
        Err(error) => PrintResult {
            action_key,
            filename: filename.to_string(),
            path: path_text,
            status: "error".into(),
            detail: format!("印刷タスク失敗: {}", error),
            category,
        },
    }
}

fn print_verified(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("印刷対象ファイルが存在しません".into());
    }
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("ファイル検証失敗: {}", error))?;
    if metadata.len() == 0 {
        return Err("空のファイルは印刷できません".into());
    }
    #[cfg(target_os = "windows")]
    {
        let printer = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg("(Get-CimInstance -ClassName Win32_Printer -Filter 'Default = $true').Name")
            .output()
            .map_err(|error| format!("既定プリンタの確認に失敗: {}", error))?;
        if !printer.status.success() {
            return Err(format!(
                "既定プリンタが確認できません: {}",
                String::from_utf8_lossy(&printer.stderr).trim()
            ));
        }
        let printer_name = String::from_utf8_lossy(&printer.stdout).trim().to_string();
        if printer_name.is_empty() {
            return Err("既定プリンタが設定されていません".into());
        }
        let path_arg = path.to_string_lossy().replace('\'', "''");
        let output = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(format!(
                "Start-Process -FilePath '{}' -Verb Print -PassThru -Wait | Out-Null",
                path_arg
            ))
            .output()
            .map_err(|error| format!("印刷コマンド起動失敗: {}", error))?;
        if !output.status.success() {
            return Err(format!(
                "印刷受付失敗: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(format!("既定プリンタ {} に送信しました", printer_name))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let printer = Command::new("lpstat")
            .arg("-d")
            .output()
            .map_err(|error| format!("既定プリンタの確認に失敗: {}", error))?;
        if !printer.status.success() {
            return Err(format!(
                "既定プリンタが確認できません: {}",
                String::from_utf8_lossy(&printer.stderr).trim()
            ));
        }
        let output = Command::new("lp")
            .arg(path)
            .output()
            .map_err(|error| format!("印刷コマンド起動失敗: {}", error))?;
        if !output.status.success() {
            return Err(format!(
                "印刷受付失敗: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let response = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if response.is_empty() {
            return Err("印刷コマンドから受付確認を取得できませんでした".into());
        }
        Ok(response)
    }
}

pub(super) fn has_retryable_print_failure(results: &[PrintResult]) -> bool {
    results
        .iter()
        .any(|item| matches!(item.status.as_str(), "error" | "not_found"))
}

pub(super) fn print_action_key(
    filename: &str,
    path: &str,
    source_fingerprint: &str,
    category: &str,
) -> String {
    let source_identity = if source_fingerprint.trim().is_empty() {
        path.trim()
    } else {
        source_fingerprint.trim()
    };
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}|{}|{}",
            filename.trim(),
            source_identity,
            normalize_category(category)
        ))
    )
}

pub(super) fn print_result_action_key(result: &PrintResult) -> String {
    if !result.action_key.trim().is_empty() {
        result.action_key.clone()
    } else {
        print_action_key(&result.filename, &result.path, "", &result.category)
    }
}

pub(super) fn print_results_match(left: &PrintResult, right: &PrintResult) -> bool {
    match (
        left.action_key.trim().is_empty(),
        right.action_key.trim().is_empty(),
    ) {
        (false, false) => left.action_key == right.action_key,
        _ => {
            (!left.path.is_empty() && !right.path.is_empty() && left.path == right.path)
                || (!left.filename.is_empty()
                    && !right.filename.is_empty()
                    && left.filename == right.filename)
        }
    }
}

pub(super) fn dispatching_print_result(
    filename: &str,
    path: String,
    category: String,
    action_key: String,
) -> PrintResult {
    PrintResult {
        action_key,
        filename: filename.to_string(),
        path,
        status: "dispatching".into(),
        detail: "プリンタへ送信中です".into(),
        category,
    }
}

pub(super) fn unknown_print_result(
    filename: &str,
    path: String,
    category: String,
    action_key: String,
) -> PrintResult {
    PrintResult {
        action_key,
        filename: filename.to_string(),
        path,
        status: "unknown".into(),
        detail:
            "前回の印刷送信後に完了状態を確認できませんでした。重複を避けるため自動再印刷しません"
                .into(),
        category,
    }
}

pub(super) fn settle_stale_print_dispatches(status: &mut CourseAutomationStatus) -> bool {
    let mut changed = false;
    for result in &mut status.print_results {
        if result.status != "dispatching" {
            continue;
        }
        result.status = "unknown".into();
        result.detail =
            "前回の印刷送信後に完了状態を確認できませんでした。重複を避けるため自動再印刷しません"
                .into();
        if result.action_key.trim().is_empty() {
            result.action_key = print_result_action_key(result);
        }
        changed = true;
    }
    changed
}

pub(super) fn merge_print_results(
    existing: &[PrintResult],
    current: Vec<PrintResult>,
) -> Vec<PrintResult> {
    let mut merged = existing.to_vec();
    for result in current {
        let matching_index = merged
            .iter()
            .position(|item| print_results_match(item, &result));
        match matching_index {
            Some(index) if merged[index].status == "printed" => {}
            Some(index) => merged[index] = result,
            None => merged.push(result),
        }
    }
    merged
}

pub(super) fn merge_print_candidates(
    existing: &[PrintCandidate],
    current: Vec<PrintCandidate>,
) -> Vec<PrintCandidate> {
    let mut merged = existing.to_vec();
    for candidate in current {
        if let Some(existing) = merged
            .iter_mut()
            .find(|item| item.filename == candidate.filename)
        {
            *existing = candidate;
        } else {
            merged.push(candidate);
        }
    }
    merged
}
