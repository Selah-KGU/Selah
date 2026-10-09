use super::*;

#[test]
fn failed_and_panicked_stream_writers_keep_original_bytes_and_clean_staging() {
    let dir = std::env::temp_dir().join(format!("selah-atomic-failure-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("history.json");
    let original = "[\"全文 👩🏽‍💻\"]".as_bytes();
    atomic_write(&path, original).unwrap();
    let result = atomic_write_with(&path, |file| {
        file.write_all(&vec![b'x'; BUFFER_BYTES + 10]).unwrap();
        Err::<(), _>("fixture encode failure")
    });
    assert!(matches!(
        result,
        Err(WriteError::Content("fixture encode failure"))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let panic = std::panic::catch_unwind(|| {
        atomic_write_with(&path, |file| -> io::Result<()> {
            file.write_all(b"partial")?;
            panic!("fixture writer panic");
        })
    });
    assert!(panic.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    atomic_write(&path, b"[]").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"[]");
    std::fs::remove_dir_all(dir).unwrap();
}
