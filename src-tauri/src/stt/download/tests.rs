use super::*;
use std::io::{Cursor, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("selah-stt-install-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn model() -> SttModelInfo {
    SttModelInfo {
        id: "fixture".into(),
        name: "fixture".into(),
        size_label: "fixture".into(),
        archive_name: "fixture.tar.bz2".into(),
        folder_name: "fixture".into(),
        download_url: "unused".into(),
        file_size_mb: 0,
        model_file: "model.onnx".into(),
        tokens_file: "tokens.txt".into(),
    }
}

fn installed(root: &Path, contents: &[u8]) -> PathBuf {
    let directory = root.join("fixture");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("model.onnx"), contents).unwrap();
    fs::write(directory.join("tokens.txt"), b"old tokens").unwrap();
    directory
}

fn archive(model_bytes: &[u8], token_bytes: &[u8]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in [("model.onnx", model_bytes), ("tokens.txt", token_bytes)] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("fixture/{name}"), bytes)
            .unwrap();
    }
    builder.into_inner().unwrap()
}

fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn prepare(
    staging: &ModelStaging,
    bytes: &[u8],
    expected: Option<u64>,
    canceled: &impl Fn() -> bool,
) -> Result<PathBuf, String> {
    let archive_path = staging.root.join("download.bz2");
    let mut file = fs::File::create(&archive_path).unwrap();
    copy_download(Cursor::new(bytes), &mut file, expected, canceled, |_| {})?;
    drop(file);
    let destination = staging.root.join("unpacked");
    let decoder = bzip2::read::BzDecoder::new(fs::File::open(archive_path).unwrap());
    extract_model(decoder, &destination, &model(), canceled)?;
    Ok(destination.join("fixture"))
}

#[test]
fn complete_download_is_validated_before_replacing_the_model_and_cleanup_discards_the_archive() {
    let root = TestDirectory::new();
    let destination = installed(&root.0, b"old model");
    let vad = root.0.join("vad.onnx");
    fs::write(&vad, b"old vad").unwrap();
    let bytes = compressed(&archive(b"new model", b"new tokens"));
    let mut staging = ModelStaging::new(&root.0).unwrap();
    let staged_model = prepare(&staging, &bytes, Some(bytes.len() as u64), &|| false).unwrap();
    assert_eq!(
        fs::read(destination.join("model.onnx")).unwrap(),
        b"old model"
    );
    assert!(model_files_ready(&model(), &staged_model));
    staging
        .publish(&[(staged_model, destination.clone())])
        .unwrap();
    drop(staging);
    assert_eq!(
        fs::read(destination.join("model.onnx")).unwrap(),
        b"new model"
    );
    assert_eq!(
        fs::read(destination.join("tokens.txt")).unwrap(),
        b"new tokens"
    );
    assert_eq!(fs::read(vad).unwrap(), b"old vad");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 2);
}

#[test]
fn truncated_transfer_corrupt_archive_and_incomplete_models_preserve_the_existing_installation() {
    let valid = compressed(&archive(b"new model", b"new tokens"));
    let empty_model = compressed(&archive(b"", b"tokens"));
    let empty_tokens = compressed(&archive(b"model", b""));
    for (bytes, expected) in [
        (valid.as_slice(), Some(valid.len() as u64 + 1)),
        (b"not a bzip2 archive".as_slice(), None),
        (empty_model.as_slice(), None),
        (empty_tokens.as_slice(), None),
        (b"".as_slice(), None),
    ] {
        let root = TestDirectory::new();
        let destination = installed(&root.0, b"old model");
        let staging = ModelStaging::new(&root.0).unwrap();
        assert!(prepare(&staging, bytes, expected, &|| false).is_err());
        drop(staging);
        assert_eq!(
            fs::read(destination.join("model.onnx")).unwrap(),
            b"old model"
        );
        assert_eq!(
            fs::read(destination.join("tokens.txt")).unwrap(),
            b"old tokens"
        );
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }
}

#[test]
fn missing_or_directory_tokens_and_an_undersized_model_are_never_ready() {
    let root = TestDirectory::new();
    let directory = installed(&root.0, b"too small");
    let mut definition = model();
    definition.file_size_mb = 1;
    assert!(!model_files_ready(&definition, &directory));
    fs::remove_file(directory.join("tokens.txt")).unwrap();
    fs::create_dir(directory.join("tokens.txt")).unwrap();
    assert!(!model_files_ready(&model(), &directory));
    fs::remove_dir(directory.join("tokens.txt")).unwrap();
    assert!(!model_files_ready(&model(), &directory));
}

#[test]
fn cancellation_during_transfer_and_a_large_archive_entry_cleans_only_staged_files() {
    struct CancelAfterRead {
        input: Cursor<Vec<u8>>,
        reads: AtomicUsize,
        cancel: Arc<AtomicBool>,
    }
    impl Read for CancelAfterRead {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let count = self.input.read(output)?;
            if self.reads.fetch_add(1, Ordering::SeqCst) == 2 {
                self.cancel.store(true, Ordering::SeqCst);
            }
            Ok(count)
        }
    }
    let root = TestDirectory::new();
    let destination = installed(&root.0, b"old model");
    for extract in [false, true] {
        let staging = ModelStaging::new(&root.0).unwrap();
        let canceled = Arc::new(AtomicBool::new(false));
        let reader = CancelAfterRead {
            input: Cursor::new(if extract {
                archive(&vec![b'm'; 1024 * 1024], b"tokens")
            } else {
                vec![b'm'; 1024 * 1024]
            }),
            reads: AtomicUsize::new(0),
            cancel: Arc::clone(&canceled),
        };
        let check = || canceled.load(Ordering::SeqCst);
        let result = if extract {
            extract_model(reader, &staging.root.join("unpacked"), &model(), &check)
        } else {
            let mut file = fs::File::create(staging.root.join("download")).unwrap();
            copy_download(reader, &mut file, None, &check, |_| {}).map(|_| ())
        };
        assert_eq!(result.unwrap_err(), "cancelled");
        drop(staging);
        assert_eq!(
            fs::read(destination.join("model.onnx")).unwrap(),
            b"old model"
        );
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }
}

#[test]
fn archive_links_are_rejected_without_writing_outside_staging() {
    let root = TestDirectory::new();
    let outside = root.0.join("outside");
    fs::write(&outside, b"keep").unwrap();
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_mode(0o600);
    builder
        .append_link(&mut header, "fixture/model.onnx", "../../outside")
        .unwrap();
    let staging = ModelStaging::new(&root.0).unwrap();
    assert!(extract_model(
        Cursor::new(builder.into_inner().unwrap()),
        &staging.root.join("unpacked"),
        &model(),
        &|| false
    )
    .is_err());
    drop(staging);
    assert_eq!(fs::read(outside).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn a_failed_vad_publish_restores_every_previous_file_and_model_directory() {
    let root = TestDirectory::new();
    let destination = installed(&root.0, b"old model");
    let vad = root.0.join("vad.onnx");
    fs::write(&vad, b"old vad").unwrap();
    let mut staging = ModelStaging::new(&root.0).unwrap();
    let next = installed(&staging.root, b"new model");
    let next_vad = staging.root.join("vad.onnx");
    fs::write(&next_vad, b"new vad").unwrap();
    let mut calls = 0;
    let result = staging.publish_with(
        &[(next, destination.clone()), (next_vad, vad.clone())],
        |from, to| {
            calls += 1;
            if calls == 4 {
                Err(io::Error::other("injected VAD rename failure"))
            } else {
                fs::rename(from, to)
            }
        },
    );
    assert!(result.is_err());
    drop(staging);
    assert_eq!(
        fs::read(destination.join("model.onnx")).unwrap(),
        b"old model"
    );
    assert_eq!(fs::read(vad).unwrap(), b"old vad");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 2);
}

#[test]
fn a_failed_first_install_leaves_no_partial_model_in_the_store() {
    let root = TestDirectory::new();
    let destination = root.0.join("fixture");
    let mut staging = ModelStaging::new(&root.0).unwrap();
    let next = installed(&staging.root, b"new model");
    let next_vad = staging.root.join("vad.onnx");
    fs::write(&next_vad, b"new vad").unwrap();
    let mut calls = 0;
    assert!(staging
        .publish_with(
            &[
                (next, destination.clone()),
                (next_vad, root.0.join("vad.onnx"))
            ],
            |from, to| {
                calls += 1;
                if calls == 2 {
                    Err(io::Error::other("injected VAD rename failure"))
                } else {
                    fs::rename(from, to)
                }
            }
        )
        .is_err());
    drop(staging);
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn a_failed_rollback_preserves_the_old_model_backup_and_reports_its_location() {
    let root = TestDirectory::new();
    let destination = installed(&root.0, b"old model");
    let mut staging = ModelStaging::new(&root.0).unwrap();
    let next = installed(&staging.root, b"new model");
    let retained = staging.root.clone();
    let mut calls = 0;
    let error = staging
        .publish_with(&[(next, destination)], |from, to| {
            calls += 1;
            if calls == 2 || calls == 3 {
                Err(io::Error::other("injected rename failure"))
            } else {
                fs::rename(from, to)
            }
        })
        .unwrap_err();
    assert!(error.contains(&retained.display().to_string()));
    drop(staging);
    assert_eq!(
        fs::read(retained.join("previous-0/model.onnx")).unwrap(),
        b"old model"
    );
}

#[test]
fn unwinding_during_publication_does_not_delete_the_only_old_model_copy() {
    let root = TestDirectory::new();
    let destination = installed(&root.0, b"old model");
    let mut staging = ModelStaging::new(&root.0).unwrap();
    let next = installed(&staging.root, b"new model");
    let retained = staging.root.clone();
    let mut calls = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        staging.publish_with(&[(next, destination)], |from, to| {
            calls += 1;
            if calls == 2 {
                panic!("injected publication panic");
            }
            fs::rename(from, to)
        })
    }));
    assert!(result.is_err());
    drop(staging);
    assert_eq!(
        fs::read(retained.join("previous-0/model.onnx")).unwrap(),
        b"old model"
    );
}
