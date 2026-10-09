use super::vault::StoreError;

const SERVICE: &str = "com.kgu.selah";

pub(super) trait CredentialStore {
    fn read(&self, account: &str) -> Result<Option<String>, StoreError>;
    fn write(&self, account: &str, value: &str) -> Result<(), StoreError>;
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

pub(super) struct NativeCredentials;

#[cfg(target_os = "macos")]
impl CredentialStore for NativeCredentials {
    fn read(&self, account: &str) -> Result<Option<String>, StoreError> {
        match security_framework::passwords::get_generic_password(SERVICE, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| StoreError::new("corrupt", "Invalid credential encoding")),
            Err(e) if e.code() == -25300 => Ok(None),
            Err(e) => Err(StoreError::new(
                "locked",
                format!("Keychain read failed: {e}"),
            )),
        }
    }
    fn write(&self, account: &str, value: &str) -> Result<(), StoreError> {
        // Never delete first. A failed update must preserve the previous key.
        security_framework::passwords::set_generic_password(SERVICE, account, value.as_bytes())
            .map_err(|e| StoreError::new("write_failed", format!("Keychain write failed: {e}")))
    }
    fn delete(&self, account: &str) -> Result<(), StoreError> {
        match security_framework::passwords::delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(StoreError::new(
                "cleanup_failed",
                format!("Keychain cleanup failed: {e}"),
            )),
        }
    }
}

#[cfg(target_os = "windows")]
impl CredentialStore for NativeCredentials {
    fn read(&self, account: &str) -> Result<Option<String>, StoreError> {
        let entry = keyring::Entry::new(SERVICE, account)
            .map_err(|e| StoreError::new("locked", format!("Credential store: {e}")))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(StoreError::new(
                "locked",
                format!("Credential read failed: {e}"),
            )),
        }
    }
    fn write(&self, account: &str, value: &str) -> Result<(), StoreError> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|entry| entry.set_password(value))
            .map_err(|e| StoreError::new("write_failed", format!("Credential write failed: {e}")))
    }
    fn delete(&self, account: &str) -> Result<(), StoreError> {
        match keyring::Entry::new(SERVICE, account).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StoreError::new(
                "cleanup_failed",
                format!("Credential cleanup failed: {e}"),
            )),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl CredentialStore for NativeCredentials {
    fn read(&self, _: &str) -> Result<Option<String>, StoreError> {
        Err(StoreError::new(
            "locked",
            "No system credential store on this platform",
        ))
    }
    fn write(&self, _: &str, _: &str) -> Result<(), StoreError> {
        self.read("").map(|_| ())
    }
    fn delete(&self, _: &str) -> Result<(), StoreError> {
        self.read("").map(|_| ())
    }
}
