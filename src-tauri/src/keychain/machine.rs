// Legacy file-only key derivation, retained for compatibility. This mode does
// not protect against another process running as the same OS user.
pub(super) fn machine_key() -> [u8; 32] {
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
