// ---------------------------------------------------------------------------
// Secret store — ONE bundle, fetched in one shot.
//
// Every app secret (AI key, OAuth tokens) and every cookie blob lives in a
// single JSON map stored as a SINGLE keychain item (account = BUNDLE_ACCOUNT)
// with a SINGLE machine-bound encrypted-file fallback. The bundle is read once
// per process into memory and reused, so:
//
//   * "一次全取": one SecItemCopyMatching returns all secrets → at most ONE
//     keychain authorization prompt for the entire app, ever.
//   * complete coverage: secrets + cookies (namespaced "cookie.<key>") share
//     the one bundle, so there is no second service that can prompt separately.
//
// Identical in debug and release: both read the keychain bundle (one cached
// read per process) and fall back to the encrypted file only when the keychain
// is unavailable.
//
// The machine-bound encrypted file uses AES-256-GCM keyed off a per-machine
// identifier: the ciphertext is useless if copied to another machine and never
// readable in plaintext. A process running as this user can re-derive the key
// from the binary, so that tier is theft/leak resistance, not secrecy against
// local malware — the keychain is for that.
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

const SERVICE: &str = "com.kgu.selah";
// Dev and release share one data dir + login keychain, so the bundle slot is
// namespaced by build: a dev (ad-hoc-signed) run never reads, overwrites, or
// hits an ACL conflict with the installed Developer ID build's secrets.
#[cfg(debug_assertions)]
const BUNDLE_ACCOUNT: &str = "secret_bundle_v1_dev";
#[cfg(not(debug_assertions))]
const BUNDLE_ACCOUNT: &str = "secret_bundle_v1";

/// In-memory copy of the whole secret bundle. None = not loaded yet.
static BUNDLE: LazyLock<Mutex<Option<HashMap<String, String>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Set when the keychain bundle existed but could not be read this session
/// (e.g. the access prompt was denied). The in-memory bundle is then empty
/// *but not authoritative*, so persisting it would clobber the real keychain
/// copy — `persist_bundle` refuses to write while this is set.
static BUNDLE_LOAD_FAILED: AtomicBool = AtomicBool::new(false);

/// Outcome of a keychain bundle read, distinguishing "no item yet" (safe to
/// create a fresh bundle) from "item exists but unlock failed" (must not
/// overwrite it).
enum BundleRead {
    Found(String),
    NotFound,
    Failed,
}

fn with_bundle<R>(f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
    let mut guard = BUNDLE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(load_bundle_from_store());
    }
    f(guard.as_mut().expect("bundle loaded"))
}

// ---- public API -----------------------------------------------------------

pub fn get_secret(key: &str) -> Option<String> {
    with_bundle(|bundle| bundle.get(key).cloned())
}

pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    with_bundle(|bundle| {
        bundle.insert(key.to_string(), value.to_string());
        persist_bundle(bundle)
    })
}

pub fn delete_secret(key: &str) {
    with_bundle(|bundle| {
        if bundle.remove(key).is_some() {
            let _ = persist_bundle(bundle);
        }
    });
}

pub fn get_cookie_secret(key: &str) -> Option<String> {
    get_secret(&cookie_ns(key))
}

pub fn set_cookie_secret(key: &str, value: &str) -> Result<(), String> {
    set_secret(&cookie_ns(key), value)
}

pub fn delete_cookie_secret(key: &str) {
    delete_secret(&cookie_ns(key));
}

fn cookie_ns(key: &str) -> String {
    format!("cookie.{key}")
}

// ---- load / persist the whole bundle --------------------------------------

/// User-chosen backend: store secrets only in the encrypted file, never the
/// OS keychain. Default is keychain (see SecurityConfig).
fn store_is_file() -> bool {
    crate::commands::load_security_config().secret_store == "file"
}

fn load_bundle_from_store() -> HashMap<String, String> {
    // File-only mode: read the encrypted file, never touch the keychain.
    if store_is_file() {
        return enc_read(&bundle_enc_path())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
    }
    // Keychain mode: read the keychain bundle once per process (cached after).
    match bundle_kc_read() {
        BundleRead::Found(json) => match serde_json::from_str::<HashMap<String, String>>(&json) {
            Ok(map) => return map,
            Err(_) => {
                // Corrupt payload: don't let an empty map overwrite it.
                BUNDLE_LOAD_FAILED.store(true, Ordering::Relaxed);
                return HashMap::new();
            }
        },
        BundleRead::Failed => {
            BUNDLE_LOAD_FAILED.store(true, Ordering::Relaxed);
            log::warn!("[keychain] secret bundle read failed — secrets unavailable until restart");
            return HashMap::new();
        }
        BundleRead::NotFound => {}
    }
    // Keychain empty: one-time import of any leftover encrypted file into the
    // keychain, then drop the file (keychain mode keeps no file fallback).
    if let Some(json) = enc_read(&bundle_enc_path()) {
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&json) {
            let _ = persist_bundle(&map);
            return map;
        }
    }
    HashMap::new()
}

fn persist_bundle(map: &HashMap<String, String>) -> Result<(), String> {
    if BUNDLE_LOAD_FAILED.load(Ordering::Relaxed) {
        return Err(
            "secret store locked: keychain read failed this session — restart to retry".into(),
        );
    }
    let json = serde_json::to_string(map).map_err(|e| format!("serialize bundle: {e}"))?;
    if store_is_file() {
        // File-only: write the file and make sure no copy lingers in the keychain.
        enc_write(&bundle_enc_path(), &json)?;
        bundle_kc_delete();
        Ok(())
    } else {
        // Keychain-only: no encrypted-file fallback. Write the keychain and drop
        // any leftover file. A keychain write failure surfaces as an error
        // rather than silently falling back to a weaker on-disk copy.
        bundle_kc_write(&json)?;
        let _ = std::fs::remove_file(bundle_enc_path());
        Ok(())
    }
}

/// Pre-load the secret bundle once, off the main thread at app launch, so the
/// single keychain read isn't paid lazily on the first secret access.
pub fn prewarm() {
    std::thread::spawn(|| {
        let _ = get_secret("__prewarm__");
    });
}

/// Re-write the current bundle so a changed storage preference (keep vs. drop
/// the encrypted-file fallback) takes effect now, rather than on the next
/// secret change.
pub fn reapply_storage_policy() {
    with_bundle(|bundle| {
        let _ = persist_bundle(bundle);
    });
}

// ---- Windows credential store (plain; no biometric equivalent wired) ------

#[cfg(target_os = "windows")]
fn kc_set(service: &str, account: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new(service, account)
        .map_err(|e| format!("Keychain entry error: {}", e))?
        .set_password(value)
        .map_err(|e| format!("Credential set error: {}", e))
}

// ---- bundle item: single keychain entry on macOS --------------------------
//
// Plain generic-password item in the login keychain. Reads are silent for a
// matching code signature; a different signature prompts once.
//
// Touch ID gating was tried (data-protection keychain + SecAccessControl
// biometry) and abandoned: it requires the keychain-access-groups entitlement,
// which AMFI rejects at exec for a Developer ID build with no provisioning
// profile — the app gets SIGKILLed on launch. Not viable for this distribution.

#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(target_os = "macos")]
fn bundle_kc_read() -> BundleRead {
    match security_framework::passwords::get_generic_password(SERVICE, BUNDLE_ACCOUNT) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(json) => BundleRead::Found(json),
            Err(_) => BundleRead::Failed,
        },
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => BundleRead::NotFound,
        Err(e) => {
            log::warn!(
                "[keychain] bundle read failed (code {}): {}",
                e.code(),
                e.message().unwrap_or_default()
            );
            BundleRead::Failed
        }
    }
}

#[cfg(target_os = "macos")]
fn bundle_kc_write(json: &str) -> Result<(), String> {
    // Delete then add so the item is recreated owned by the *current* code
    // signature. Plain `set` on an item created by a different identity (a prior
    // build, or a dev rebuild — ad-hoc cdhash changes every build) takes the
    // SecItemUpdate path, which prompts on write and never takes ownership, so
    // every read+write keeps prompting. Recreating it makes writes — and later
    // reads — silent. Delete does not require the read ACL, so it is itself silent.
    let _ = security_framework::passwords::delete_generic_password(SERVICE, BUNDLE_ACCOUNT);
    security_framework::passwords::set_generic_password(SERVICE, BUNDLE_ACCOUNT, json.as_bytes())
        .map_err(|e| format!("keychain set: {e}"))
}

#[cfg(target_os = "macos")]
fn bundle_kc_delete() {
    let _ = security_framework::passwords::delete_generic_password(SERVICE, BUNDLE_ACCOUNT);
}

#[cfg(target_os = "windows")]
fn bundle_kc_read() -> BundleRead {
    let entry = match keyring::Entry::new(SERVICE, BUNDLE_ACCOUNT) {
        Ok(entry) => entry,
        Err(error) => {
            log::warn!("[keychain] Windows credential entry failed: {error}");
            return BundleRead::Failed;
        }
    };

    classify_windows_credential_read(entry.get_password())
}

#[cfg(target_os = "windows")]
fn classify_windows_credential_read(result: Result<String, keyring::Error>) -> BundleRead {
    match result {
        Ok(json) => BundleRead::Found(json),
        Err(keyring::Error::NoEntry) => BundleRead::NotFound,
        Err(error) => {
            log::warn!("[keychain] Windows credential read failed: {error}");
            BundleRead::Failed
        }
    }
}

#[cfg(target_os = "windows")]
fn bundle_kc_write(json: &str) -> Result<(), String> {
    kc_set(SERVICE, BUNDLE_ACCOUNT, json)
}

#[cfg(target_os = "windows")]
fn bundle_kc_delete() {
    if let Ok(entry) = keyring::Entry::new(SERVICE, BUNDLE_ACCOUNT) {
        let _ = entry.delete_credential();
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_credential_tests {
    use super::{classify_windows_credential_read, BundleRead};

    #[test]
    fn distinguishes_missing_credential_from_read_failure() {
        assert!(matches!(
            classify_windows_credential_read(Err(keyring::Error::NoEntry)),
            BundleRead::NotFound
        ));

        let platform_error = keyring::Error::PlatformFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "credential store denied access",
        )));
        assert!(matches!(
            classify_windows_credential_read(Err(platform_error)),
            BundleRead::Failed
        ));
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn bundle_kc_read() -> BundleRead {
    BundleRead::NotFound
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn bundle_kc_write(_json: &str) -> Result<(), String> {
    Err("no OS keychain on this platform".into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn bundle_kc_delete() {}

// ---- machine-bound encrypted file (fallback) ------------------------------

fn secrets_dir() -> std::path::PathBuf {
    let dir = crate::client::data_dir().join("secrets");
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    }
    dir
}

fn bundle_enc_path() -> std::path::PathBuf {
    // Namespaced by build for the same reason as BUNDLE_ACCOUNT — dev and
    // release share the data dir, so the file fallback must not clobber either.
    #[cfg(debug_assertions)]
    let name = "bundle.v1.dev.enc";
    #[cfg(not(debug_assertions))]
    let name = "bundle.v1.enc";
    secrets_dir().join(name)
}

/// 32-byte AES key derived from a stable per-machine identifier plus a static
/// salt. Not stored anywhere — re-derived on each call.
fn machine_key() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"com.kgu.selah/secret-box/v1");
    hasher.update(machine_entropy());
    hasher.finalize().into()
}

#[cfg(target_os = "macos")]
fn machine_entropy() -> Vec<u8> {
    // gethostuuid(): per-machine UUID, available inside the App Sandbox (no
    // subprocess, unlike `ioreg`). Declared directly to avoid a libc dep.
    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }
    extern "C" {
        fn gethostuuid(id: *mut u8, wait: *const Timespec) -> std::os::raw::c_int;
    }
    let mut uuid = [0u8; 16];
    let wait = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let rc = unsafe { gethostuuid(uuid.as_mut_ptr(), &wait) };
    if rc == 0 {
        uuid.to_vec()
    } else {
        // Fall back to the per-user data dir if the syscall fails.
        crate::client::data_dir()
            .to_string_lossy()
            .into_owned()
            .into_bytes()
    }
}

#[cfg(not(target_os = "macos"))]
fn machine_entropy() -> Vec<u8> {
    // Per-user install path; binds the ciphertext to this account/machine.
    crate::client::data_dir()
        .to_string_lossy()
        .into_owned()
        .into_bytes()
}

fn enc_write(path: &std::path::Path, value: &str) -> Result<(), String> {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
    use rand::RngCore;

    let cipher = Aes256Gcm::new_from_slice(&machine_key())
        .map_err(|e| format!("cipher init failed: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), value.as_bytes())
        .map_err(|e| format!("encrypt failed: {e}"))?;

    // File layout: nonce(12) || ciphertext+tag.
    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);

    std::fs::write(path, &blob).map_err(|e| format!("Failed to write secret file: {}", e))?;
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|e| format!("Failed to protect secret file: {}", e))?;
    }
    Ok(())
}

fn enc_read(path: &std::path::Path) -> Option<String> {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};

    let blob = std::fs::read(path).ok()?;
    if blob.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(&machine_key()).ok()?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .ok()?;
    String::from_utf8(plaintext).ok()
}
