//! Prepare model files away from the installed directory; publish only after
//! validation. The caller holds the model-operation reservation throughout.

use super::super::{model::model_files_ready, SttModelInfo};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

pub(super) fn check_canceled(canceled: &impl Fn() -> bool) -> Result<(), String> {
    if canceled() {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}

pub(super) fn copy_download(
    mut input: impl Read,
    output: &mut fs::File,
    expected: Option<u64>,
    canceled: &impl Fn() -> bool,
    mut progress: impl FnMut(u64),
) -> Result<u64, String> {
    use std::io::Write;
    let mut buffer = vec![0; 256 * 1024];
    let mut downloaded = 0;
    loop {
        check_canceled(canceled)?;
        let count = input.read(&mut buffer).map_err(|error| {
            if canceled() {
                "cancelled".into()
            } else {
                format!("ダウンロード読み取りエラー: {error}")
            }
        })?;
        check_canceled(canceled)?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("ファイル書き込みエラー: {error}"))?;
        downloaded += count as u64;
        progress(downloaded);
    }
    if downloaded == 0 || expected.is_some_and(|length| downloaded != length) {
        return Err("ダウンロードが不完全です。もう一度お試しください".into());
    }
    output
        .sync_data()
        .map_err(|error| format!("ファイル同期エラー: {error}"))?;
    check_canceled(canceled)?;
    Ok(downloaded)
}

struct CancelReader<'a, R, F> {
    input: R,
    canceled: &'a F,
}

impl<R: Read, F: Fn() -> bool> Read for CancelReader<'_, R, F> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if (self.canceled)() {
            return Err(io::Error::other("cancelled"));
        }
        let count = self.input.read(output)?;
        if (self.canceled)() {
            return Err(io::Error::other("cancelled"));
        }
        Ok(count)
    }
}

pub(super) fn extract_model(
    input: impl Read,
    destination: &Path,
    model: &SttModelInfo,
    canceled: &impl Fn() -> bool,
) -> Result<(), String> {
    check_canceled(canceled)?;
    let mut archive = tar::Archive::new(CancelReader { input, canceled });
    let result = (|| -> io::Result<()> {
        fs::create_dir_all(destination)?;
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;
            let mut components = path.components().filter(|part| *part != Component::CurDir);
            if components.next() != Some(Component::Normal(model.folder_name.as_ref()))
                || components.any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected model archive path",
                ));
            }
            let kind = entry.header().entry_type();
            if kind.is_dir() {
                // Avoid applying read-only directory modes before children.
                fs::create_dir_all(destination.join(path.as_ref()))?;
            } else if !kind.is_file() || !entry.unpack_in(destination)? {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected model archive entry",
                ));
            }
        }
        Ok(())
    })();
    check_canceled(canceled)?;
    result.map_err(|error| format!("モデル展開失敗: {error}"))?;
    let directory = destination.join(&model.folder_name);
    if !model_files_ready(model, &directory) {
        return Err("展開した STT モデルが不完全です。もう一度ダウンロードしてください".into());
    }
    for file in [&model.model_file, &model.tokens_file] {
        fs::OpenOptions::new()
            .write(true)
            .open(directory.join(file))
            .and_then(|file| file.sync_data())
            .map_err(|error| format!("モデルファイル同期エラー: {error}"))?;
    }
    check_canceled(canceled)
}

pub(super) struct ModelStaging {
    pub(super) root: PathBuf,
    preserve: bool,
}

impl ModelStaging {
    pub(super) fn new(parent: &Path) -> Result<Self, String> {
        fs::create_dir_all(parent).map_err(|error| format!("モデル保存先の作成失敗: {error}"))?;
        let root = parent.join(format!(".selah-stt-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).map_err(|error| format!("モデル準備先の作成失敗: {error}"))?;
        Ok(Self {
            root,
            preserve: false,
        })
    }

    pub(super) fn publish(&mut self, replacements: &[(PathBuf, PathBuf)]) -> Result<(), String> {
        self.publish_with(replacements, |from, to| fs::rename(from, to))
    }

    fn publish_with(
        &mut self,
        replacements: &[(PathBuf, PathBuf)],
        mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    ) -> Result<(), String> {
        // Record each move before the following operation can fail. Reverse
        // moves restore both existing installations and first-time installs.
        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        // A panic during publication must not let Drop delete old backups.
        self.preserve = true;
        let result = (|| -> io::Result<()> {
            for (index, (source, destination)) in replacements.iter().enumerate() {
                if destination.try_exists()? {
                    let backup = self.root.join(format!("previous-{index}"));
                    rename(destination, &backup)?;
                    moves.push((destination.clone(), backup));
                }
                rename(source, destination)?;
                moves.push((source.clone(), destination.clone()));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (from, to) in moves.iter().rev() {
                if let Err(rollback) = rename(to, from) {
                    rollback_errors.push(rollback.to_string());
                }
            }
            if !rollback_errors.is_empty() {
                // Never clean up the only remaining copy of an old model.
                return Err(format!(
                    "モデル配置失敗: {error}。復元できなかったファイルは {} に保持しました ({})",
                    self.root.display(),
                    rollback_errors.join(" / ")
                ));
            }
            self.preserve = false;
            return Err(format!("モデル配置失敗: {error}"));
        }
        self.preserve = false;
        Ok(())
    }
}

impl Drop for ModelStaging {
    fn drop(&mut self) {
        if !self.preserve {
            if let Err(error) = fs::remove_dir_all(&self.root) {
                log::warn!(
                    "[stt] staging cleanup failed at {}: {error}",
                    self.root.display()
                );
            }
        } else {
            log::warn!(
                "[stt] retained installation staging at {}",
                self.root.display()
            );
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
