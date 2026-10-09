//! Account-bound database handles. A request never changes its database when
//! login changes while it is awaiting the network or a worker queue.
use super::*;
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::{Arc, MutexGuard},
};
use tauri::{
    ipc::{CommandArg, CommandItem, InvokeError},
    Manager, Runtime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountContext {
    pub generation: u64,
    pub username: Option<String>,
}

tokio::task_local! { static CURRENT_ACCOUNT: AccountContext; }
pub(crate) fn task_account() -> Option<AccountContext> {
    CURRENT_ACCOUNT
        .try_with(Clone::clone)
        .ok()
        .or_else(crate::agent_turn_scope::account_context)
}
pub(crate) fn capture_account() -> AccountContext {
    task_account().unwrap_or_else(|| crate::session_coordinator::SESSIONS.account_context())
}
pub(crate) async fn account_work<T>(
    context: AccountContext,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    if context != crate::session_coordinator::SESSIONS.account_context() {
        return Err(crate::session_coordinator::CANCELLED.into());
    }
    let result = CURRENT_ACCOUNT.scope(context.clone(), work).await;
    if context != crate::session_coordinator::SESSIONS.account_context() {
        return Err(crate::session_coordinator::CANCELLED.into());
    }
    result
}

struct Pool {
    directory: PathBuf,
    connections: HashMap<String, Connection>,
}
#[derive(Clone)]
pub(super) struct AccountConnection {
    pool: Arc<Mutex<Pool>>,
    bound: Option<AccountContext>,
    mail_owner: Option<Option<String>>,
    context: Option<Arc<dyn Fn() -> AccountContext + Send + Sync>>,
}
pub(super) struct ConnectionGuard<'a> {
    pool: MutexGuard<'a, Pool>,
    key: String,
}
impl Deref for ConnectionGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.pool.connections[&self.key]
    }
}
impl DerefMut for ConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.pool.connections.get_mut(&self.key).unwrap()
    }
}
impl AccountConnection {
    pub fn local(directory: PathBuf) -> Result<Self, String> {
        let connection = Database::open_connection(&directory)?;
        Ok(Self {
            pool: Arc::new(Mutex::new(Pool {
                directory,
                connections: HashMap::from([("local".into(), connection)]),
            })),
            bound: None,
            mail_owner: None,
            context: None,
        })
    }
    pub fn accounts(directory: PathBuf) -> Result<Self, String> {
        Self::with_context(
            directory,
            Arc::new(|| crate::session_coordinator::SESSIONS.account_context()),
        )
    }
    fn with_context(
        directory: PathBuf,
        context: Arc<dyn Fn() -> AccountContext + Send + Sync>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(directory.join("accounts")).map_err(|e| e.to_string())?;
        Ok(Self {
            pool: Arc::new(Mutex::new(Pool {
                directory,
                connections: HashMap::new(),
            })),
            bound: None,
            mail_owner: None,
            context: Some(context),
        })
    }
    #[cfg(test)]
    pub fn try_lock(&self) -> Result<(), String> {
        self.pool.try_lock().map(|_| ()).map_err(|e| e.to_string())
    }
    pub fn lock(&self) -> Result<ConnectionGuard<'_>, String> {
        let mut pool = self.pool.lock().map_err(|e| e.to_string())?;
        let key = if let Some(current) = &self.context {
            let current = current();
            let context = self.bound.as_ref().unwrap_or(&current);
            if context != &current {
                return Err(crate::session_coordinator::CANCELLED.into());
            }
            let username = context
                .username
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    "大学アカウントを確認できません。大学にログインしてください。".to_string()
                })?;
            account_key(username)
        } else {
            "local".into()
        };
        if !pool.connections.contains_key(&key) {
            let connection =
                Database::open_connection(&pool.directory.join("accounts").join(&key))?;
            pool.connections.insert(key.clone(), connection);
        }
        Ok(ConnectionGuard { pool, key })
    }
}
fn account_key(username: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(username.as_bytes()))
}

#[derive(Clone)]
pub(crate) struct AccountDb(Database);
impl Database {
    pub(super) fn cache_storage_key(&self, key: &str) -> String {
        if crate::mail::cache::is_mail_key(key)
            && (self.conn.context.is_some() || self.conn.mail_owner.is_some())
        {
            let owner = self
                .conn
                .mail_owner
                .clone()
                .unwrap_or_else(crate::mail::cache::owner);
            crate::mail::cache::scoped_key(owner.as_deref(), key)
        } else {
            key.to_owned()
        }
    }
    pub(crate) fn ensure_current_mail(&self) -> Result<(), String> {
        self.ensure_current_account()?;
        if let Some(owner) = &self.conn.mail_owner {
            if owner.is_none() || owner != &crate::mail::cache::owner() {
                return Err("Mail connection changed".into());
            }
        }
        Ok(())
    }
    pub(crate) fn ensure_mail_connection(&self, connection: &str) -> Result<(), String> {
        self.ensure_current_account()?;
        if let Some(owner) = &self.conn.mail_owner {
            if owner.as_deref() != Some(connection) {
                return Err("Mail connection changed; retry with its own cache".into());
            }
        }
        Ok(())
    }
    pub(crate) fn ensure_current_account(&self) -> Result<(), String> {
        if let (Some(bound), Some(current)) = (&self.conn.bound, &self.conn.context) {
            if bound != &current() {
                return Err(crate::session_coordinator::CANCELLED.into());
            }
        }
        Ok(())
    }
    pub(crate) fn scope_for(&self, context: AccountContext) -> AccountDb {
        let mut db = self.clone();
        if db.conn.bound.is_none() {
            db.conn.bound = Some(context);
        }
        if db.conn.mail_owner.is_none() && db.conn.context.is_some() {
            db.conn.mail_owner = Some(crate::mail::cache::owner());
        }
        AccountDb(db)
    }
    pub(crate) fn scope(&self) -> AccountDb {
        let mut db = self.clone();
        if db.conn.bound.is_none() {
            if let Some(context) = &db.conn.context {
                db.conn.bound = Some(task_account().unwrap_or_else(|| context()));
            }
        }
        if db.conn.mail_owner.is_none() && db.conn.context.is_some() {
            db.conn.mail_owner = Some(crate::mail::cache::owner());
        }
        AccountDb(db)
    }
}
impl AccountDb {
    pub fn inner(&self) -> &Database {
        &self.0
    }
}
impl Deref for AccountDb {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.0
    }
}
impl<'de, R: Runtime> CommandArg<'de, R> for AccountDb {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        Ok(command
            .message
            .webview_ref()
            .app_handle()
            .state::<Database>()
            .scope())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mail_cache_namespaces_cover_batches_timestamps_and_late_writes() {
        let dir = std::env::temp_dir().join(format!("selah-mail-cache-{}", uuid::Uuid::new_v4()));
        let db = Database::open(&dir).unwrap();
        db.save_data_cache("mail_inbox", "legacy").unwrap();
        let mut a = db.clone();
        a.conn.mail_owner = Some(Some("mail-a".into()));
        let mut b = db.clone();
        b.conn.mail_owner = Some(Some("mail-b".into()));
        assert!(a.cache_payload("mail_inbox").is_none());
        a.save_data_cache("mail_inbox", "alice").unwrap();
        b.save_data_cache("mail_inbox", "bob").unwrap();
        a.save_data_cache("mail_inbox", "late alice").unwrap();
        assert_eq!(b.cache_payload("mail_inbox").as_deref(), Some("bob"));
        assert_eq!(
            b.get_data_cache_many(&["mail_inbox".into()]).unwrap()["mail_inbox"].0,
            "bob"
        );
        let rows = b
            .get_data_cache_deltas(&[("mail_inbox".into(), a.cache_revision("mail_inbox"))])
            .unwrap();
        assert_eq!(rows[0].json.as_deref(), Some("bob"));
        assert_eq!(rows[0].key, "mail_inbox");
        assert_eq!(
            b.cache_timestamps(&["mail_inbox".into()], false)
                .unwrap()
                .rows[0]
                .updated_at,
            b.cache_updated_at("mail_inbox")
        );
        b.conn.mail_owner = Some(None);
        assert!(b.cache_payload("mail_inbox").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn capturing_an_account_does_not_wait_for_sqlite_io() {
        let directory =
            std::env::temp_dir().join(format!("selah-admission-{}", uuid::Uuid::new_v4()));
        let db = Database {
            conn: AccountConnection::with_context(
                directory.clone(),
                Arc::new(|| AccountContext {
                    generation: 1,
                    username: Some("alice".into()),
                }),
            )
            .unwrap(),
        };
        let guard = db.conn.lock().unwrap();
        let other = db.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _scope = other.scope();
            sent.send(()).unwrap();
        });
        let responsive = received.recv_timeout(std::time::Duration::from_secs(2));
        drop(guard);
        worker.join().unwrap();
        responsive.expect("IPC account capture waited for SQLite");
        drop(db);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn accounts_isolate_cache_and_tables_and_reject_retired_handles() {
        let directory =
            std::env::temp_dir().join(format!("selah-accounts-{}", uuid::Uuid::new_v4()));
        let current = Arc::new(Mutex::new(AccountContext {
            generation: 1,
            username: Some("alice".into()),
        }));
        let owner = current.clone();
        let db = Database {
            conn: AccountConnection::with_context(
                directory.clone(),
                Arc::new(move || owner.lock().unwrap().clone()),
            )
            .unwrap(),
        };
        let alice = db.scope();
        alice.save_data_cache("kwic_home", "alice").unwrap();
        alice.conn.lock().unwrap().execute("INSERT INTO luna_courses (luna_id,name,day,period) VALUES ('one','Alice course',1,1)", []).unwrap();
        *current.lock().unwrap() = AccountContext {
            generation: 2,
            username: Some("bob".into()),
        };
        let bob = db.scope();
        assert!(bob.get_data_cache("kwic_home").unwrap().is_none());
        assert_eq!(
            bob.conn
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM luna_courses", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(alice.save_data_cache("kwic_home", "late response").is_err());
        assert!(alice.get_data_cache("kwic_home").is_err());
        bob.save_data_cache("kwic_home", "bob").unwrap();
        *current.lock().unwrap() = AccountContext {
            generation: 3,
            username: Some("alice".into()),
        };
        assert!(alice
            .save_data_cache("kwic_home", "stale same-account request")
            .is_err());
        assert_eq!(
            db.scope().cache_payload("kwic_home").as_deref(),
            Some("alice")
        );
        *current.lock().unwrap() = AccountContext {
            generation: 4,
            username: None,
        };
        assert!(db.scope().get_data_cache("kwic_home").is_err());
        drop((alice, bob, db));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn unowned_legacy_data_is_preserved_but_never_assigned_to_a_login() {
        let directory = std::env::temp_dir().join(format!("selah-legacy-{}", uuid::Uuid::new_v4()));
        let legacy = Database::open(&directory).unwrap();
        legacy.save_data_cache("kwic_home", "unowned").unwrap();
        let db = Database {
            conn: AccountConnection::with_context(
                directory.clone(),
                Arc::new(|| AccountContext {
                    generation: 1,
                    username: Some("new-account".into()),
                }),
            )
            .unwrap(),
        };
        assert!(db.scope().get_data_cache("kwic_home").unwrap().is_none());
        assert_eq!(
            legacy.cache_payload("kwic_home").as_deref(),
            Some("unowned")
        );
        drop((db, legacy));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
