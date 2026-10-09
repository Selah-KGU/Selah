use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use rand::RngCore;
use std::{fs::OpenOptions, io::Write, path::Path};

const HEADER: &[u8] = b"SELAH-VAULT\x02";

pub(super) fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "Invalid vault key")?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: HEADER,
            },
        )
        .map_err(|_| "Vault encryption failed")?;
    Ok([HEADER, &nonce, &encrypted].concat())
}

pub(super) fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, String> {
    if !blob.starts_with(HEADER) || blob.len() < HEADER.len() + 12 + 16 {
        return Err("Unsupported or damaged secret vault".into());
    }
    let nonce = &blob[HEADER.len()..HEADER.len() + 12];
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "Invalid vault key")?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &blob[HEADER.len() + 12..],
                aad: HEADER,
            },
        )
        .map_err(|_| "Secret vault could not be authenticated".into())
}

/// Read the v1 nonce || ciphertext format only during migration.
pub(super) fn decrypt_legacy(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < 28 {
        return Err("Damaged legacy secret file".into());
    }
    Aes256Gcm::new_from_slice(key)
        .map_err(|_| "Invalid legacy key")?
        .decrypt(Nonce::from_slice(&blob[..12]), &blob[12..])
        .map_err(|_| "Legacy secret file could not be authenticated".into())
}

/// Same-directory replacement keeps the previous file intact until the entire
/// new ciphertext has been written and synced. The temp is private from birth.
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Secret file has no parent directory")?;
    let temporary = parent.join(format!(".selah-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|e| format!("Create secret file: {e}"))?;
        file.write_all(contents)
            .map_err(|e| format!("Write secret file: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Sync secret file: {e}"))?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|e| format!("Commit secret file: {e}"))?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("Sync secret directory: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_format_roundtrips_large_payload_and_rejects_damage() {
        let key = [9; 32];
        let payload = vec![b'x'; 128 * 1024];
        let blob = encrypt(&key, &payload).unwrap();
        assert_eq!(decrypt(&key, &blob).unwrap(), payload);
        assert_ne!(blob, encrypt(&key, &payload).unwrap());
        assert!(decrypt(&[8; 32], &blob).is_err());
        for index in [0, HEADER.len(), blob.len() - 1] {
            let mut damaged = blob.clone();
            damaged[index] ^= 1;
            assert!(decrypt(&key, &damaged).is_err());
        }
        assert!(decrypt(&key, &blob[..10]).is_err());
    }

    #[test]
    fn atomic_replace_preserves_private_permissions_and_leaves_no_temps() {
        let dir = std::env::temp_dir().join(format!("selah-vault-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("vault.enc");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // A failed rename cannot replace or truncate the previous destination.
        let blocked = dir.join("directory");
        std::fs::create_dir(&blocked).unwrap();
        assert!(atomic_write(&blocked, b"failed").is_err());
        assert!(blocked.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
