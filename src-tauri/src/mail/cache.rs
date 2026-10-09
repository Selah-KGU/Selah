//! The encrypted token owns this opaque cache namespace. Every authorization
//! gets a new identity; refresh and restart retain it. Legacy unowned rows are
//! deliberately not adopted. A disconnected client has no readable namespace.
use std::sync::RwLock;
static OWNER: RwLock<Option<String>> = RwLock::new(None);
pub(crate) fn owner() -> Option<String> {
    OWNER.read().unwrap_or_else(|e| e.into_inner()).clone()
}
pub(super) fn set_owner(owner: Option<String>) {
    *OWNER.write().unwrap_or_else(|e| e.into_inner()) = owner;
}
pub(crate) fn scoped_key(owner: Option<&str>, key: &str) -> String {
    use sha2::{Digest, Sha256};
    let owner = owner
        .map(|id| format!("{:x}", Sha256::digest(id.as_bytes())))
        .unwrap_or_else(|| "disconnected".into());
    format!("mail-connection/{owner}/{key}")
}
pub(crate) fn is_mail_key(key: &str) -> bool {
    key.starts_with("mail_")
        || key == "seen_notifs_mail"
        || key == "seen_notifs_init_mail"
        || key == "read_notifs_mail"
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connections_and_legacy_rows_have_disjoint_keys() {
        let a = scoped_key(Some("connection-a"), "mail_inbox");
        assert_eq!(a, scoped_key(Some("connection-a"), "mail_inbox"));
        assert_ne!(a, scoped_key(Some("connection-b"), "mail_inbox"));
        assert_ne!(a, scoped_key(None, "mail_inbox"));
        assert_ne!(a, "mail_inbox");
    }
}
